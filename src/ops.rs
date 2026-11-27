//! Operations that modify the Space-based strip: add/remove, 2D snap, strict merge,
//! collision shift/evict, maximize (explode), focus, move, join, split lanes.

use std::collections::HashMap;

use crate::types::*;

impl Strip {
    pub fn new(viewport: Rect) -> Self {
        Self {
            spaces: Vec::new(),
            focused: None,
            scroll_y: 0,
            viewport,
            min_heights: HashMap::new(),
            min_widths: HashMap::new(),
            gap: 0,
            cue: None,
        }
    }

    /// Set the gap. Spacing between windows and from each Space edge is the same.
    pub fn set_gap(&mut self, gap: i32) {
        self.gap = gap.max(0);
        self.ensure_visible();
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }

    pub fn spaces(&self) -> &[Space] {
        &self.spaces
    }

    /// Flat list of all items across all Spaces (convenience helper).
    pub fn items(&self) -> Vec<Item> {
        self.spaces
            .iter()
            .flat_map(|sp| sp.items.iter().copied())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.spaces.iter().all(|sp| sp.items.is_empty())
    }

    pub fn item(&self, id: WindowId) -> Option<Item> {
        let (si, ii) = self.find_item(id)?;
        Some(self.spaces[si].items[ii])
    }

    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = viewport;
        self.ensure_visible();
    }

    pub(crate) fn cleanup_empty_spaces(&mut self) {
        self.spaces.retain(|sp| !sp.items.is_empty());
    }

    /// Sync each item's `split` mirror field with its owning Space's `split`.
    pub(crate) fn sync_space_split(space: &mut Space) {
        let split = space.split.clamp(150, 850);
        space.split = split;
        for it in &mut space.items {
            it.split = split;
        }
    }

    /// Record the window's minimum height. True if it increases (the layout changes).
    pub fn set_min_height(&mut self, id: WindowId, h: i32) -> bool {
        let h = h.min((self.content().h - 2 * (self.gap / 2)).max(0));
        let cur = self.min_heights.get(&id).copied().unwrap_or(0);
        if h > cur {
            self.min_heights.insert(id, h);
            self.ensure_visible();
            true
        } else {
            false
        }
    }

    /// Record the window's minimum width. True if it increases (the layout changes).
    pub fn set_min_width(&mut self, id: WindowId, w: i32) -> bool {
        let w = w.min((self.content().w - 2 * (self.gap / 2)).max(0));
        let cur = self.min_widths.get(&id).copied().unwrap_or(0);
        if w > cur {
            self.min_widths.insert(id, w);
            true
        } else {
            false
        }
    }

    /// Move the lane divider in the focused window's Space: `delta_px` > 0 moves right,
    /// < 0 moves left. Width belongs to the Space.
    pub fn adjust_split(&mut self, delta_px: i32) -> bool {
        let Some((si, ii)) = self.focused.and_then(|f| self.find_item(f)) else {
            return false;
        };
        if !matches!(self.spaces[si].items[ii].span, Span::Left | Span::Right) {
            return false;
        }
        let cw = self.content().w;
        let stored = self.spaces[si].split;
        let current = self.space_left_width(&self.spaces[si], cw);
        let target = (current + delta_px).clamp(cw * 15 / 100, cw * 85 / 100);
        let new_split = (target as i64 * 1000 / cw.max(1) as i64) as i32;
        if new_split == stored {
            return false;
        }
        let shrinking_span = if delta_px > 0 { Span::Right } else { Span::Left };
        let shrinking_ids: Vec<WindowId> = self.spaces[si]
            .items
            .iter()
            .filter(|it| it.span == shrinking_span)
            .map(|it| it.id)
            .collect();
        for id in shrinking_ids {
            self.min_widths.remove(&id);
        }
        self.spaces[si].split = new_split;
        Self::sync_space_split(&mut self.spaces[si]);
        true
    }

    /// Reset the divider in the focused window's Space to 50/50.
    pub fn reset_split(&mut self) -> bool {
        let Some((si, ii)) = self.focused.and_then(|f| self.find_item(f)) else {
            return false;
        };
        if !matches!(self.spaces[si].items[ii].span, Span::Left | Span::Right) {
            return false;
        }
        let mut changed = self.spaces[si].split != 500;
        let ids: Vec<WindowId> = self.spaces[si]
            .items
            .iter()
            .filter(|it| matches!(it.span, Span::Left | Span::Right))
            .map(|it| it.id)
            .collect();
        for id in ids {
            changed |= self.min_widths.remove(&id).is_some();
        }
        self.spaces[si].split = 500;
        Self::sync_space_split(&mut self.spaces[si]);
        changed
    }

    /// Incrementally adjust height inside the Space bounds (kept for compatibility).
    pub fn adjust_height(&mut self, delta_px: i32) -> bool {
        let Some(id) = self.focused else {
            return false;
        };
        let Some((si, ii)) = self.find_item(id) else {
            return false;
        };
        let ch = self.content().h;
        let Some(current) = self.flow().iter().find(|f| f.id == id).map(|f| f.h) else {
            return false;
        };
        let it = self.spaces[si].items[ii];
        let max_h_px = ch * (1000 - it.top) / 1000;
        let target = (current + delta_px).clamp(ch * 15 / 100, max_h_px.max(1));
        if target == current {
            return false;
        }
        let new_permil = ((target as i64 * 1000 / ch.max(1) as i64) as i32).clamp(150, 1000 - it.top);
        let new_h = Height::from_permil(new_permil);
        if new_h == it.height {
            return false;
        }
        if self.spaces[si].has_overlap(it.span, it.top, new_h, Some(id)) {
            return false;
        }
        self.min_heights.remove(&id);
        self.spaces[si].items[ii].height = new_h;
        self.ensure_visible();
        true
    }

    /// Resize window `id` vertically by dragging its top (`top_edge = true`) or bottom
    /// (`top_edge = false`) edge by `delta_px` (> 0 moves downward, < 0 moves upward).
    /// If another window touches that shared horizontal boundary inside the same Space,
    /// both sides move together like a vertical divider; if the adjacent area is empty,
    /// the window resizes directly into/out of the empty space.
    pub fn adjust_vertical_edge(&mut self, id: WindowId, top_edge: bool, delta_px: i32) -> bool {
        if delta_px == 0 {
            return false;
        }
        let Some((si, ii)) = self.find_item(id) else {
            return false;
        };
        let it = self.spaces[si].items[ii];
        if it.span == Span::Hidden {
            return false;
        }
        let ch = self.content().h.max(1);
        let delta_p = (delta_px as i64 * 1000 / ch as i64) as i32;
        if delta_p == 0 {
            return false;
        }
        let (y0, y1) = it.v_range();
        let (hx0, hx1) = it.span.h_range();
        let horiz_overlap = |other: &Item| {
            if other.span == Span::Hidden {
                return false;
            }
            let (ox0, ox1) = other.span.h_range();
            hx0 < ox1 && ox0 < hx1
        };

        if top_edge {
            let new_y0 = (y0 + delta_p).clamp(0, y1 - 150);
            if new_y0 == y0 {
                return false;
            }
            // Find any windows whose bottom edge touches `y0` and horizontally overlap `it`.
            let touching: Vec<usize> = self.spaces[si]
                .items
                .iter()
                .enumerate()
                .filter(|&(k, o)| {
                    k != ii && horiz_overlap(o) && (o.v_range().1 - y0).abs() <= 3
                })
                .map(|(k, _)| k)
                .collect();

            let mut candidate = self.spaces[si].clone();
            candidate.items[ii].top = new_y0;
            candidate.items[ii].height = Height::from_permil(y1 - new_y0);

            if !touching.is_empty() {
                for &k in &touching {
                    let (oy0, _) = candidate.items[k].v_range();
                    let new_oh = new_y0 - oy0;
                    if new_oh < 150 {
                        return false;
                    }
                    candidate.items[k].height = Height::from_permil(new_oh);
                }
            }
            // Ensure no overlap with any window in the Space.
            for (a_i, a) in candidate.items.iter().enumerate() {
                for b in &candidate.items[a_i + 1..] {
                    if a.overlaps_rect(b.span, b.top, b.height) {
                        return false;
                    }
                }
            }
            self.min_heights.remove(&id);
            for &k in &touching {
                self.min_heights.remove(&candidate.items[k].id);
            }
            self.spaces[si] = candidate;
            self.ensure_visible();
            true
        } else {
            let new_y1 = (y1 + delta_p).clamp(y0 + 150, 1000);
            if new_y1 == y1 {
                return false;
            }
            // Find any windows whose top edge touches `y1` and horizontally overlap `it`.
            let touching: Vec<usize> = self.spaces[si]
                .items
                .iter()
                .enumerate()
                .filter(|&(k, o)| {
                    k != ii && horiz_overlap(o) && (o.v_range().0 - y1).abs() <= 3
                })
                .map(|(k, _)| k)
                .collect();

            let mut candidate = self.spaces[si].clone();
            candidate.items[ii].height = Height::from_permil(new_y1 - y0);

            if !touching.is_empty() {
                for &k in &touching {
                    let (_, oy1) = candidate.items[k].v_range();
                    let new_oh = oy1 - new_y1;
                    if new_oh < 150 {
                        return false;
                    }
                    candidate.items[k].top = new_y1;
                    candidate.items[k].height = Height::from_permil(new_oh);
                }
            }
            for (a_i, a) in candidate.items.iter().enumerate() {
                for b in &candidate.items[a_i + 1..] {
                    if a.overlaps_rect(b.span, b.top, b.height) {
                        return false;
                    }
                }
            }
            self.min_heights.remove(&id);
            for &k in &touching {
                self.min_heights.remove(&candidate.items[k].id);
            }
            self.spaces[si] = candidate;
            self.ensure_visible();
            true
        }
    }

    /// New window: creates a new Full Space directly below the focused window's Space.
    pub fn add_window(&mut self, id: WindowId) {
        if self.find_item(id).is_some() {
            return;
        }
        let at = self
            .focused
            .and_then(|f| self.find_item(f))
            .map_or(self.spaces.len(), |(si, _)| si + 1);
        self.spaces.insert(at, Space::new_full(id));
        self.focused = Some(id);
        self.ensure_visible();
    }

    /// New window with optional initial `span` and `height` (used by window rules).
    pub fn add_window_with(&mut self, id: WindowId, span: Option<Span>, height: Option<Height>) {
        if self.find_item(id).is_some() {
            return;
        }
        self.add_window(id);
        if let Some((si, ii)) = self.find_item(id) {
            if let Some(h) = height {
                self.spaces[si].items[ii].height = h;
            }
            if let Some(sp) = span {
                match sp {
                    Span::Left => self.snap(Dir::Left),
                    Span::Right => self.snap(Dir::Right),
                    _ => {}
                }
            }
            self.ensure_visible();
        }
    }

    /// Insert new window `id` at `at` (dragged from overview or pick mode).
    pub fn insert_window(&mut self, id: WindowId, at: InsertPosition) -> bool {
        if self.find_item(id).is_some() {
            return false;
        }
        let target = match at {
            InsertPosition::Beside(t)
            | InsertPosition::FullBefore(t)
            | InsertPosition::FullAfter(t)
            | InsertPosition::Split(t, _) => t,
        };
        let Some((si, ii)) = self.find_item(target) else {
            return false;
        };
        let t = self.spaces[si].items[ii];
        if t.span == Span::Hidden {
            return false;
        }

        match at {
            InsertPosition::FullBefore(_) => {
                self.spaces.insert(si, Space::new_full(id));
            }
            InsertPosition::FullAfter(_) => {
                self.spaces.insert(si + 1, Space::new_full(id));
            }
            InsertPosition::Beside(_) => {
                if t.span == Span::Full {
                    return false;
                }
                let opp = t.span.opposite();
                if self.spaces[si].has_overlap(opp, t.top, t.height, None) {
                    return false;
                }
                let split = self.spaces[si].split;
                self.spaces[si].items.push(Item {
                    id,
                    span: opp,
                    top: t.top,
                    height: t.height,
                    restore: opp,
                    split,
                });
            }
            InsertPosition::Split(_, first) => match t.span {
                Span::Full => {
                    let (mine, theirs) = if first {
                        (Span::Right, Span::Left)
                    } else {
                        (Span::Left, Span::Right)
                    };
                    let split = self.spaces[si].split;
                    self.spaces[si].items[ii].span = mine;
                    self.spaces[si].items[ii].restore = mine;
                    self.spaces[si].items.push(Item {
                        id,
                        span: theirs,
                        top: t.top,
                        height: t.height,
                        restore: theirs,
                        split,
                    });
                }
                Span::Left | Span::Right => {
                    let (vy0, vy1) = t.v_range();
                    let span_h = vy1 - vy0;
                    if span_h / 2 < 150 {
                        return false;
                    }
                    let h1 = span_h / 2;
                    let h2 = span_h - h1;
                    let split = self.spaces[si].split;
                    let (t_top, t_h, n_top, n_h) = if first {
                        (vy0 + h1, h2, vy0, h1)
                    } else {
                        (vy0, h1, vy0 + h1, h2)
                    };
                    self.spaces[si].items[ii].top = t_top;
                    self.spaces[si].items[ii].height = Height::from_permil(t_h);
                    self.spaces[si].items.push(Item {
                        id,
                        span: t.span,
                        top: n_top,
                        height: Height::from_permil(n_h),
                        restore: t.span,
                        split,
                    });
                }
                Span::Hidden => return false,
            },
        }
        self.focused = Some(id);
        self.ensure_visible();
        true
    }

    /// Move existing window `id` to `at` in the same strip.
    /// If the insertion is invalid, restore the original arrangement.
    pub fn reinsert_window(&mut self, id: WindowId, at: InsertPosition) -> bool {
        let Some((si, ii)) = self.find_item(id) else {
            return false;
        };
        let saved_spaces = self.spaces.clone();
        let saved_focused = self.focused;
        self.spaces[si].items.remove(ii);
        self.cleanup_empty_spaces();
        if self.insert_window(id, at) {
            true
        } else {
            self.spaces = saved_spaces;
            self.focused = saved_focused;
            false
        }
    }

    pub fn remove_window(&mut self, id: WindowId) {
        let Some((si, ii)) = self.find_item(id) else {
            return;
        };
        self.spaces[si].items.remove(ii);
        self.min_heights.remove(&id);
        self.min_widths.remove(&id);
        let space_empty = self.spaces[si].items.is_empty();
        if space_empty {
            self.spaces.remove(si);
        }
        if self.focused == Some(id) {
            self.focused = self.pick_focus(si);
        }
        self.ensure_visible();
    }

    /// Nearest visible window to Space `si` (same space first, then below, then above).
    fn pick_focus(&self, si: usize) -> Option<WindowId> {
        for k in si..self.spaces.len() {
            if let Some(it) = self.spaces[k].items.iter().find(|it| it.span != Span::Hidden) {
                return Some(it.id);
            }
        }
        for k in (0..si.min(self.spaces.len())).rev() {
            if let Some(it) = self.spaces[k].items.iter().find(|it| it.span != Span::Hidden) {
                return Some(it.id);
            }
        }
        None
    }

    /// Hide / show a window without losing its slot. Used when a window is minimized
    /// or reappears. True if anything changed.
    pub fn set_hidden(&mut self, id: WindowId, hide: bool) -> bool {
        let Some((si, ii)) = self.find_item(id) else {
            return false;
        };
        if hide {
            if self.spaces[si].items[ii].span == Span::Hidden {
                return false;
            }
            self.spaces[si].items[ii].restore = self.spaces[si].items[ii].span;
            self.spaces[si].items[ii].span = Span::Hidden;
            if self.focused == Some(id) {
                self.focused = self.pick_focus(si);
            }
        } else {
            if self.spaces[si].items[ii].span != Span::Hidden {
                return false;
            }
            let restore = self.spaces[si].items[ii].restore;
            self.spaces[si].items[ii].span = if restore == Span::Hidden {
                Span::Full
            } else {
                restore
            };
            if self.focused.is_none() {
                self.focused = Some(id);
            }
        }
        self.ensure_visible();
        true
    }

    pub fn set_focus(&mut self, id: WindowId) {
        if let Some((si, ii)) = self.find_item(id) {
            if self.spaces[si].items[ii].span != Span::Hidden {
                self.focused = Some(id);
                self.ensure_visible();
            }
        }
    }

    /// Compute the next 2D snap target `(span, top, height)` for `dir`.
    /// Horizontal: `Left <-> Full <-> Right`
    /// Vertical:   `Top  <-> Full <-> Bottom`
    fn next_snap_rect(it: &Item, dir: Dir) -> Option<(Span, i32, Height)> {
        if it.span == Span::Hidden {
            return None;
        }
        let (vy0, vy1) = it.v_range();
        let is_full_v = vy0 == 0 && vy1 >= 1000;
        let is_top_v = vy0 == 0 && vy1 < 1000;
        let is_bottom_v = vy0 > 0;

        let target = match dir {
            Dir::Left => match it.span {
                Span::Right => (Span::Full, it.top, it.height),
                Span::Full => (Span::Left, it.top, it.height),
                Span::Left | Span::Hidden => return None,
            },
            Dir::Right => match it.span {
                Span::Left => (Span::Full, it.top, it.height),
                Span::Full => (Span::Right, it.top, it.height),
                Span::Right | Span::Hidden => return None,
            },
            Dir::Up => {
                if is_bottom_v {
                    (it.span, 0, Height::Full)
                } else if is_full_v {
                    (it.span, 0, Height::Half)
                } else {
                    return None;
                }
            }
            Dir::Down => {
                if is_top_v {
                    (it.span, 0, Height::Full)
                } else if is_full_v {
                    (it.span, 500, Height::Half)
                } else {
                    return None;
                }
            }
        };
        Some(target)
    }

    /// 2D Snap (`Win+Arrow`):
    /// - Horizontal: `Left <-> Full <-> Right`
    /// - Vertical:   `Top  <-> Full <-> Bottom`
    /// - Strict merge: if solo in Space `k > 0` and target fits in empty slot of Space `k - 1`,
    ///   merge into Space `k - 1` and remove empty Space `k`.
    /// - Collision inside Space:
    ///   - If target becomes full-space `(Full, 0, Full)`, evict all other windows together
    ///     into one new Space at `k + 1`.
    ///   - Otherwise, try shifting each colliding window to the opposite side; if occupied,
    ///     evict colliding windows together into one new Space at `k + 1`.
    pub fn snap(&mut self, dir: Dir) {
        let Some(id) = self.focused else { return };
        let Some((si, ii)) = self.find_item(id) else {
            return;
        };
        let cur = self.spaces[si].items[ii];
        let Some((new_span, new_top, new_height)) = Self::next_snap_rect(&cur, dir) else {
            return;
        };

        if matches!(dir, Dir::Up | Dir::Down) {
            self.min_heights.remove(&id);
        } else {
            self.min_widths.remove(&id);
        }

        let target_is_full_space =
            new_span == Span::Full && new_top == 0 && new_height.permil() >= 998;

        // 1. Solo window in Space `si`: check strict merge into Space `si - 1`.
        if self.spaces[si].visible_count() == 1 && !target_is_full_space && si > 0 {
            let prev_has_visible = self.spaces[si - 1].visible_count() > 0;
            if prev_has_visible
                && !self.spaces[si - 1].has_overlap(new_span, new_top, new_height, None)
            {
                let mut moved = self.spaces[si].items.remove(ii);
                moved.span = new_span;
                moved.restore = new_span;
                moved.top = new_top;
                moved.height = new_height;
                moved.split = self.spaces[si - 1].split;
                self.spaces[si - 1].items.push(moved);
                self.cleanup_empty_spaces();
                self.ensure_visible();
                return;
            }
        }

        // 2. Apply new geometry in place.
        self.spaces[si].items[ii].span = new_span;
        self.spaces[si].items[ii].restore = new_span;
        self.spaces[si].items[ii].top = new_top;
        self.spaces[si].items[ii].height = new_height;

        // 3. Handle collisions in Space `si`.
        if target_is_full_space {
            // Evict all other items in this Space together into a new Space below.
            let old_split = self.spaces[si].split;
            let mut evicted = Vec::new();
            let mut kept = Vec::new();
            for it in self.spaces[si].items.drain(..) {
                if it.id == id {
                    kept.push(it);
                } else {
                    evicted.push(it);
                }
            }
            self.spaces[si].items = kept;
            self.spaces[si].split = 500;
            Self::sync_space_split(&mut self.spaces[si]);
            if !evicted.is_empty() {
                let mut new_sp = Space {
                    items: evicted,
                    split: old_split,
                };
                Self::sync_space_split(&mut new_sp);
                self.spaces.insert(si + 1, new_sp);
            }
            self.ensure_visible();
            return;
        }

        // Non-full expansion: shift colliding items to opposite side if free, else evict together.
        let colliding_ids: Vec<WindowId> = self.spaces[si]
            .items
            .iter()
            .filter(|it| it.id != id && it.overlaps_rect(new_span, new_top, new_height))
            .map(|it| it.id)
            .collect();

        let mut did_evict = false;
        if !colliding_ids.is_empty() {
            let mut to_evict: Vec<WindowId> = Vec::new();
            for cid in colliding_ids {
                let Some(ci) = self.spaces[si].items.iter().position(|it| it.id == cid) else {
                    continue;
                };
                let c_item = self.spaces[si].items[ci];
                // Compute opposite candidate: flip Left<->Right if half-width, else flip Top<->Bottom.
                let candidate = match c_item.span {
                    Span::Left | Span::Right => {
                        Some((c_item.span.opposite(), c_item.top, c_item.height))
                    }
                    Span::Full => {
                        let (cy0, cy1) = c_item.v_range();
                        if cy0 == 0 && cy1 < 1000 {
                            Some((Span::Full, 1000 - c_item.height.permil(), c_item.height))
                        } else if cy0 > 0 {
                            Some((Span::Full, 0, c_item.height))
                        } else {
                            None
                        }
                    }
                    Span::Hidden => None,
                };
                if let Some((cs, ct, ch)) = candidate {
                    if !self.spaces[si].has_overlap(cs, ct, ch, Some(cid)) {
                        self.spaces[si].items[ci].span = cs;
                        self.spaces[si].items[ci].restore = cs;
                        self.spaces[si].items[ci].top = ct;
                        self.spaces[si].items[ci].height = ch;
                        continue;
                    }
                }
                to_evict.push(cid);
            }

            if !to_evict.is_empty() {
                let old_split = self.spaces[si].split;
                let mut evicted = Vec::new();
                self.spaces[si].items.retain(|it| {
                    if to_evict.contains(&it.id) {
                        evicted.push(*it);
                        false
                    } else {
                        true
                    }
                });
                if !evicted.is_empty() {
                    let mut new_sp = Space {
                        items: evicted,
                        split: old_split,
                    };
                    Self::sync_space_split(&mut new_sp);
                    // Evict upward when pushing from below (`Win+Up`, or horizontal expand
                    // from the bottom half); otherwise evict downward.
                    let evict_above = match dir {
                        Dir::Up => true,
                        Dir::Down => false,
                        Dir::Left | Dir::Right => new_top > 0,
                    };
                    if evict_above {
                        self.spaces.insert(si, new_sp);
                    } else {
                        self.spaces.insert(si + 1, new_sp);
                    }
                    did_evict = true;
                }
            }
        }

        // 4. Pull-back strict merge: if we freed space in Space `si` (no eviction) and all
        // visible windows in an adjacent Space (`si + 1` below or `si - 1` above) now fit
        // cleanly into Space `si`, merge them back.
        if !did_evict {
            let can_pull_from = |spaces: &[Space], dst: usize, src: usize| -> bool {
                spaces[src].visible_count() > 0
                    && spaces[src]
                        .items
                        .iter()
                        .filter(|it| it.span != Span::Hidden)
                        .all(|it| !spaces[dst].has_overlap(it.span, it.top, it.height, None))
            };
            if si + 1 < self.spaces.len() && can_pull_from(&self.spaces, si, si + 1) {
                let next_sp = self.spaces.remove(si + 1);
                self.spaces[si].items.extend(next_sp.items);
                Self::sync_space_split(&mut self.spaces[si]);
            } else if si > 0 && can_pull_from(&self.spaces, si, si - 1) {
                let prev_sp = self.spaces.remove(si - 1);
                self.spaces[si - 1].items.extend(prev_sp.items);
                Self::sync_space_split(&mut self.spaces[si - 1]);
            }
        }

        self.cleanup_empty_spaces();
        self.ensure_visible();
    }

    /// Minimum slot height (including the gap) ever measured for window `id`.
    fn min_slot_height(&self, id: WindowId) -> i32 {
        self.min_heights
            .get(&id)
            .map_or(0, |m| m + 2 * (self.gap / 2))
    }

    /// Join the focused window to its neighbor in direction `dir`.
    pub fn join(&mut self, dir: Dir) -> bool {
        let Some(f) = self.focused else { return false };
        let Some(t) = self.neighbor_of(f, dir) else {
            return false;
        };
        self.join_into(f, t)
    }

    /// Join window `src` to `target` (same workspace).
    pub fn join_into(&mut self, src: WindowId, target: WindowId) -> bool {
        if src == target {
            return false;
        }
        let (Some((fsi, fii)), Some((tsi, tii))) = (self.find_item(src), self.find_item(target))
        else {
            return false;
        };
        let f_item = self.spaces[fsi].items[fii];
        let t_item = self.spaces[tsi].items[tii];
        if f_item.span == Span::Hidden || t_item.span == Span::Hidden {
            return false;
        }
        let ch = self.content().h;
        let flow = self.flow();
        let f_top = flow.iter().find(|f| f.id == src).map_or(0, |f| f.top);
        let t_top = flow.iter().find(|f| f.id == target).map_or(0, |f| f.top);

        match t_item.span {
            Span::Full => {
                let target_px = t_item.height.pixels(ch);
                if self.min_slot_height(src) > target_px {
                    return false;
                }
                let first = f_top < t_top;
                self.reinsert_window(src, InsertPosition::Split(target, first))
            }
            Span::Left | Span::Right => {
                if fsi == tsi && f_item.span == t_item.span {
                    return false;
                }
                // If there is empty space below target in the same lane of target's Space, fill it.
                let (_, ty1) = t_item.v_range();
                if ty1 < 1000 {
                    let rem = 1000 - ty1;
                    if rem >= 150
                        && !self.spaces[tsi].has_overlap(
                            t_item.span,
                            ty1,
                            Height::from_permil(rem),
                            Some(src),
                        )
                    {
                        let slot_px = Height::from_permil(rem).pixels(ch);
                        if self.min_slot_height(src) <= slot_px {
                            let mut moved = self.spaces[fsi].items.remove(fii);
                            let dest_si = if fsi < tsi && self.spaces[fsi].items.is_empty() {
                                tsi - 1
                            } else {
                                tsi
                            };
                            self.cleanup_empty_spaces();
                            moved.span = t_item.span;
                            moved.restore = t_item.span;
                            moved.top = ty1;
                            moved.height = Height::from_permil(rem);
                            moved.split = self.spaces[dest_si].split;
                            self.spaces[dest_si].items.push(moved);
                            self.focused = Some(src);
                            self.ensure_visible();
                            return true;
                        }
                    }
                }
                let can_halve = match t_item.height {
                    Height::Full | Height::Half | Height::Third => true,
                    Height::Custom(p) => p / 2 >= 150,
                };
                if !can_halve {
                    return false;
                }
                let slot_px = t_item.height.halved().pixels(ch);
                if self.min_slot_height(src) > slot_px || self.min_slot_height(target) > slot_px {
                    return false;
                }
                let first = f_top <= t_top;
                self.reinsert_window(src, InsertPosition::Split(target, first))
            }
            Span::Hidden => false,
        }
    }

    /// Join a window from another workspace into `target`'s Space.
    pub(crate) fn join_external_into(
        &mut self,
        item: Item,
        min_h: Option<i32>,
        min_w: Option<i32>,
        target: WindowId,
        from_left: bool,
    ) -> bool {
        if self.find_item(item.id).is_some() {
            return false;
        }
        let Some((tsi, tii)) = self.find_item(target) else {
            return false;
        };
        let t_item = self.spaces[tsi].items[tii];
        if t_item.span == Span::Hidden {
            return false;
        }
        let ch = self.content().h;
        let src_min_slot = min_h.map_or(0, |m| m + 2 * (self.gap / 2));

        let ok = match t_item.span {
            Span::Full => {
                if src_min_slot > t_item.height.pixels(ch) {
                    return false;
                }
                self.insert_window(item.id, InsertPosition::Split(target, from_left))
            }
            Span::Left | Span::Right => {
                let (_, ty1) = t_item.v_range();
                let rem = 1000 - ty1;
                if rem >= 150
                    && !self.spaces[tsi].has_overlap(t_item.span, ty1, Height::from_permil(rem), None)
                    && src_min_slot <= Height::from_permil(rem).pixels(ch)
                {
                    let split = self.spaces[tsi].split;
                    self.spaces[tsi].items.push(Item {
                        id: item.id,
                        span: t_item.span,
                        top: ty1,
                        height: Height::from_permil(rem),
                        restore: t_item.span,
                        split,
                    });
                    self.focused = Some(item.id);
                    true
                } else {
                    let can_halve = match t_item.height {
                        Height::Full | Height::Half | Height::Third => true,
                        Height::Custom(p) => p / 2 >= 150,
                    };
                    if !can_halve {
                        return false;
                    }
                    let slot_px = t_item.height.halved().pixels(ch);
                    if src_min_slot > slot_px || self.min_slot_height(target) > slot_px {
                        return false;
                    }
                    self.insert_window(item.id, InsertPosition::Split(target, false))
                }
            }
            Span::Hidden => false,
        };
        if ok {
            if let Some(h) = min_h {
                self.min_heights.insert(item.id, h);
            }
            if let Some(w) = min_w {
                self.min_widths.insert(item.id, w);
            }
            self.ensure_visible();
        }
        ok
    }

    /// Maximize / Explode Space (`Win+Enter`):
    /// Every window in the focused Space is separated into its own Full Space
    /// (focused window keeps focus, split reset to 500).
    pub fn maximize(&mut self) {
        let Some(id) = self.focused else { return };
        let Some((si, _)) = self.find_item(id) else {
            return;
        };
        let old = self.spaces.remove(si);
        let mut new_spaces = Vec::with_capacity(old.items.len());
        for it in old.items {
            if it.span == Span::Hidden {
                let mut h = Item::full(it.id);
                h.span = Span::Hidden;
                h.restore = Span::Full;
                new_spaces.push(Space {
                    items: vec![h],
                    split: 500,
                });
            } else {
                new_spaces.push(Space::new_full(it.id));
            }
        }
        for (offset, sp) in new_spaces.into_iter().enumerate() {
            self.spaces.insert(si + offset, sp);
        }
        self.ensure_visible();
    }

    /// Move focus based on actual screen position (automatically scrolls across Spaces).
    pub fn focus(&mut self, dir: Dir) {
        if let Some(n) = self.neighbor(dir) {
            self.focused = Some(n);
            self.ensure_visible();
        }
    }

    /// Try an authority-based side/band flip inside Space `si` when the focused window `ia`
    /// is strictly larger than its neighbor `ib`:
    /// - Horizontal (`Left`/`Right`): `ia` moves to the opposite lane keeping its size, and
    ///   all overlapping windows in that lane flip together to `ia`'s old lane.
    /// - Vertical (`Up`/`Down`): `ia` moves to the opposite vertical half keeping its size,
    ///   and all overlapping windows in that half shift together into `ia`'s old band.
    fn try_authority_flip(&mut self, si: usize, ia: usize, ib: usize, dir: Dir) -> bool {
        let item_a = self.spaces[si].items[ia];
        let item_b = self.spaces[si].items[ib];
        if item_a.area() <= item_b.area() {
            return false;
        }

        match dir {
            Dir::Left | Dir::Right => {
                let target_span = match (dir, item_a.span) {
                    (Dir::Left, Span::Right) => Span::Left,
                    (Dir::Right, Span::Left) => Span::Right,
                    _ => return false,
                };
                let old_span = item_a.span;
                let (ay0, ay1) = item_a.v_range();
                let mut group = Vec::new();
                for (k, it) in self.spaces[si].items.iter().enumerate() {
                    if k == ia || it.span == Span::Hidden {
                        continue;
                    }
                    if it.span == target_span {
                        let (cy0, cy1) = it.v_range();
                        if cy0 + 2 < ay1 && ay0 + 2 < cy1 {
                            group.push(k);
                        }
                    }
                }
                if group.is_empty() {
                    return false;
                }
                // Verify no overlap with any untouched window.
                let mut candidate = self.spaces[si].clone();
                candidate.items[ia].span = target_span;
                candidate.items[ia].restore = target_span;
                for &k in &group {
                    candidate.items[k].span = old_span;
                    candidate.items[k].restore = old_span;
                }
                for (i, a) in candidate.items.iter().enumerate() {
                    for b in &candidate.items[i + 1..] {
                        if a.overlaps_rect(b.span, b.top, b.height) {
                            return false;
                        }
                    }
                }
                candidate.split = (1000 - candidate.split).clamp(150, 850);
                Self::sync_space_split(&mut candidate);
                self.spaces[si] = candidate;
                true
            }
            Dir::Up | Dir::Down => {
                let (ay0, ay1) = item_a.v_range();
                let ah = ay1 - ay0;
                let (new_a_top, shift_other) = match dir {
                    Dir::Down if ay0 == 0 && ay1 < 1000 => (1000 - ah, -(1000 - ah)),
                    Dir::Up if ay0 > 0 && ay1 >= 1000 => (0, ah),
                    _ => return false,
                };
                let mut group = Vec::new();
                for (k, it) in self.spaces[si].items.iter().enumerate() {
                    if k == ia || it.span == Span::Hidden {
                        continue;
                    }
                    if it.overlaps_rect(item_a.span, new_a_top, item_a.height) {
                        group.push(k);
                    }
                }
                if group.is_empty() {
                    return false;
                }
                let mut candidate = self.spaces[si].clone();
                candidate.items[ia].top = new_a_top;
                for &k in &group {
                    let nt = candidate.items[k].top + shift_other;
                    if nt < 0 || nt + candidate.items[k].height.permil() > 1002 {
                        return false;
                    }
                    candidate.items[k].top = nt;
                }
                for (i, a) in candidate.items.iter().enumerate() {
                    for b in &candidate.items[i + 1..] {
                        if a.overlaps_rect(b.span, b.top, b.height) {
                            return false;
                        }
                    }
                }
                self.spaces[si] = candidate;
                true
            }
        }
    }

    /// Swap the POSITION of the focused window with its neighbor in direction `dir`.
    /// Within the same Space, a strictly larger window has higher authority and flips
    /// the opposing group as a whole while preserving sizes; a smaller or equal window
    /// swaps into the neighbor's slot.
    pub fn move_window(&mut self, dir: Dir) {
        let Some(id) = self.focused else { return };
        let Some(n) = self.neighbor(dir) else { return };
        let (Some((sa, ia)), Some((sb, ib))) = (self.find_item(id), self.find_item(n)) else {
            return;
        };
        if sa == sb && self.try_authority_flip(sa, ia, ib, dir) {
            self.ensure_visible();
            return;
        }
        let solo_full_a = self.spaces[sa].visible_count() == 1
            && self.spaces[sa].items[ia].is_full_space();
        let solo_full_b = self.spaces[sb].visible_count() == 1
            && self.spaces[sb].items[ib].is_full_space();

        if sa != sb && solo_full_a && solo_full_b {
            self.spaces.swap(sa, sb);
            self.cue = Some((id, dir));
        } else {
            // Swap the two windows into each other's exact slots (always overlap-free).
            let id_a = self.spaces[sa].items[ia].id;
            let id_b = self.spaces[sb].items[ib].id;
            self.spaces[sa].items[ia].id = id_b;
            self.spaces[sb].items[ib].id = id_a;
            if sa != sb && solo_full_a {
                self.cue = Some((id, dir));
            }
        }
        self.ensure_visible();
    }

    /// Swap the positions of two specific windows (mouse drag). Focus does not change.
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId) -> bool {
        if a == b {
            return false;
        }
        let (Some((sa, ia)), Some((sb, ib))) = (self.find_item(a), self.find_item(b)) else {
            return false;
        };
        if self.spaces[sa].items[ia].span == Span::Hidden
            || self.spaces[sb].items[ib].span == Span::Hidden
        {
            return false;
        }
        let solo_full_a = self.spaces[sa].visible_count() == 1
            && self.spaces[sa].items[ia].is_full_space();
        let solo_full_b = self.spaces[sb].visible_count() == 1
            && self.spaces[sb].items[ib].is_full_space();
        if sa != sb && solo_full_a && solo_full_b {
            self.spaces.swap(sa, sb);
        } else {
            self.spaces[sa].items[ia].id = b;
            self.spaces[sb].items[ib].id = a;
        }
        self.ensure_visible();
        true
    }

    /// Take (and clear) the latest swap cue for a full-space window.
    pub fn take_move_cue(&mut self) -> Option<(WindowId, Dir)> {
        self.cue.take()
    }
}
