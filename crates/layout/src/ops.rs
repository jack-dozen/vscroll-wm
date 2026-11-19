//! Operations that modify the strip: add/remove, snap, focus, move, split lanes.

use std::collections::HashMap;

use crate::flow::Flow;
use crate::types::*;

impl Strip {
    pub fn new(viewport: Rect) -> Self {
        Self {
            items: Vec::new(),
            focused: None,
            scroll_y: 0,
            viewport,
            min_heights: HashMap::new(),
            min_widths: HashMap::new(),
            gap: 0,
            cue: None,
        }
    }

    /// Set the gap. Spacing between windows and from the screen edge is the same.
    pub fn set_gap(&mut self, gap: i32) {
        self.gap = gap.max(0);
        self.ensure_visible();
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = viewport;
        self.ensure_visible();
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

    /// Move the lane divider in the focused window's block: `delta_px` > 0 moves right,
    /// < 0 moves left. Width belongs to the lane, so all windows in that lane move with it.
    /// Returns false if nothing moved (the window is full-width or already at the limit).
    pub fn adjust_split(&mut self, delta_px: i32) -> bool {
        let Some(i) = self.focused.and_then(|f| self.index_of(f)) else {
            return false;
        };
        if !matches!(self.items[i].span, Span::Left | Span::Right) {
            return false;
        }
        let (lo, hi) = self.run_bounds(i);
        let (left, right) = self.lanes_in(lo, hi);
        let cw = self.content().w;
        let stored = self.run_split(&left, &right);
        // Start from the ACTUAL width (including the app's constraints), not the stored value,
        // so repeated presses at the limit do not accumulate.
        let current = self.left_width(cw, stored, &left, &right);
        let target = (current + delta_px).clamp(cw * 15 / 100, cw * 85 / 100);
        let new_split = (target as i64 * 1000 / cw.max(1) as i64) as i32;
        if new_split == stored {
            return false;
        }
        // Remeasure the narrowed lane: its previous limit may no longer apply.
        let shrinking = if delta_px > 0 { &right } else { &left };
        for &k in shrinking {
            self.min_widths.remove(&self.items[k].id);
        }
        for k in lo..hi {
            if self.items[k].span != Span::Full {
                self.items[k].split = new_split;
            }
        }
        true
    }

    /// Reset the divider in the focused window's block to 50/50.
    pub fn reset_split(&mut self) -> bool {
        let Some(i) = self.focused.and_then(|f| self.index_of(f)) else {
            return false;
        };
        if !matches!(self.items[i].span, Span::Left | Span::Right) {
            return false;
        }
        let (lo, hi) = self.run_bounds(i);
        let (left, right) = self.lanes_in(lo, hi);
        let mut changed = self.run_split(&left, &right) != 500;
        for &k in left.iter().chain(right.iter()) {
            changed |= self.min_widths.remove(&self.items[k].id).is_some();
        }
        for k in lo..hi {
            if self.items[k].span != Span::Full {
                self.items[k].split = 500;
            }
        }
        changed
    }

    /// `step` mode: change the focused window's height by `delta_px` (< 0 shorter,
    /// > 0 taller), analogous to `adjust_split`. Start from the ACTUAL height
    /// (including app constraints) so repeated presses at the limit do not accumulate.
    /// Clamp to 15%-100% of the content area. If the result is within half a step of
    /// 1/3, 1/2, or full height, snap to that preset (magnet), except for the preset
    /// currently in use. Returns false if nothing changed.
    pub fn adjust_height(&mut self, delta_px: i32) -> bool {
        let Some(id) = self.focused else {
            return false;
        };
        let Some(i) = self.index_of(id) else {
            return false;
        };
        let ch = self.content().h;
        let Some(current) = self.flow().iter().find(|f| f.id == id).map(|f| f.h) else {
            return false;
        };
        let target = (current + delta_px).clamp(ch * 15 / 100, ch);
        if target == current {
            return false; // Already at the upper / lower limit.
        }
        let old = self.items[i].height;
        let nearest = [Height::Third, Height::Half, Height::Full]
            .into_iter()
            .filter(|p| *p != old)
            .min_by_key(|p| (p.pixels(ch) - target).abs())
            .filter(|p| (p.pixels(ch) - target).abs() <= delta_px.abs() / 2);
        let new = nearest
            .unwrap_or_else(|| Height::Custom((target as i64 * 1000 / ch.max(1) as i64) as i32));
        if new == old {
            return false;
        }
        // The user changed the height -> remeasure this window's minimum from scratch.
        self.min_heights.remove(&id);
        self.items[i].height = new;
        self.ensure_visible();
        true
    }

    /// New window: full width and height, directly after the focused window.
    pub fn add_window(&mut self, id: WindowId) {
        if self.index_of(id).is_some() {
            return;
        }
        // Below the ENTIRE block containing the focused window (not directly after it),
        // so windows in the other lane are not pushed below the new window.
        let at = self
            .focused
            .and_then(|f| self.index_of(f))
            .map_or(self.items.len(), |f| {
                if self.items[f].span == Span::Full {
                    f + 1
                } else {
                    (f + 1..self.items.len())
                        .find(|&k| self.items[k].span == Span::Full)
                        .unwrap_or(self.items.len())
                }
            });
        self.items.insert(
            at,
            Item {
                id,
                span: Span::Full,
                height: Height::Full,
                restore: Span::Full,
                split: 500,
            },
        );
        self.focused = Some(id);
        self.ensure_visible();
    }

    /// Insertion point for a new window in the opposite lane from item `i` (half-width),
    /// aligned with it: (insertion index, width). The nth item in each lane is aligned;
    /// if the opposite lane is shorter, insert at its bottom.
    fn beside_slot(&self, i: usize) -> (usize, Span) {
        let t = self.items[i];
        let (lo, hi) = self.run_bounds(i);
        let (left, right) = self.lanes_in(lo, hi);
        let (mine, other) = if t.span == Span::Left {
            (&left, &right)
        } else {
            (&right, &left)
        };
        let rank = mine.iter().position(|&k| k == i).unwrap_or(0);
        let index = other.get(rank).copied().unwrap_or(hi);
        let span = if t.span == Span::Left {
            Span::Right
        } else {
            Span::Left
        };
        (index, span)
    }

    /// Insert new window `id` at `at` (dragged from another workspace in the overview):
    ///  - `Beside(x)`: in the opposite lane from `x`, aligned with it, with the same
    ///    height and split ratio as `x`; only valid for half-width `x`
    ///  - `FullBefore(x)` / `FullAfter(x)`: full-width at a block boundary of `x` (never
    ///    splitting a block), creating a new block separator
    ///  - `Split(x, first)`: split `x` in two. Full-width -> side by side: `x` on one side,
    ///    the new window on the other, same height. Half-width -> stacked in the same lane:
    ///    `x` and the new window each get half of `x`'s height (Full -> 1/2, 1/2 -> 1/4, ...).
    ///    `first` = the new window goes on the left / top.
    /// Returns false if `id` already exists, `x` is unknown / hidden, or `Beside` is used
    /// with a full-width window. The new window becomes focused.
    pub fn insert_window(&mut self, id: WindowId, at: InsertPosition) -> bool {
        if self.index_of(id).is_some() {
            return false;
        }
        let target = match at {
            InsertPosition::Beside(t)
            | InsertPosition::FullBefore(t)
            | InsertPosition::FullAfter(t)
            | InsertPosition::Split(t, _) => t,
        };
        let Some(i) = self.index_of(target) else {
            return false;
        };
        let t = self.items[i];
        if t.span == Span::Hidden {
            return false;
        }
        let full = |id| Item {
            id,
            span: Span::Full,
            height: Height::Full,
            restore: Span::Full,
            split: 500,
        };
        let (index, item) = match at {
            InsertPosition::Beside(_) => {
                if t.span == Span::Full {
                    return false;
                }
                let (index, span) = self.beside_slot(i);
                (
                    index,
                    Item {
                        id,
                        span,
                        height: t.height,
                        restore: span,
                        split: t.split,
                    },
                )
            }
            InsertPosition::Split(_, first) => match t.span {
                Span::Full => {
                    // Split the width: the target goes on one side, the new window on the other.
                    let (mine, theirs) = if first {
                        (Span::Right, Span::Left)
                    } else {
                        (Span::Left, Span::Right)
                    };
                    self.items[i].span = mine;
                    self.items[i].restore = mine;
                    let (index, _) = self.beside_slot(i);
                    (
                        index,
                        Item {
                            id,
                            span: theirs,
                            height: t.height,
                            restore: theirs,
                            split: t.split,
                        },
                    )
                }
                _ => {
                    // Split the height, stacking them in the same lane.
                    let h = t.height.halved();
                    self.items[i].height = h;
                    (
                        if first { i } else { i + 1 },
                        Item {
                            id,
                            span: t.span,
                            height: h,
                            restore: t.span,
                            split: t.split,
                        },
                    )
                }
            },
            InsertPosition::FullBefore(_) => {
                let index = if t.span == Span::Full {
                    i
                } else {
                    self.run_bounds(i).0
                };
                (index, full(id))
            }
            InsertPosition::FullAfter(_) => {
                let index = if t.span == Span::Full {
                    i + 1
                } else {
                    self.run_bounds(i).1
                };
                (index, full(id))
            }
        };
        self.items.insert(index, item);
        self.focused = Some(id);
        self.ensure_visible();
        true
    }

    /// Move existing window `id` to `at` in the same strip.
    /// If the insertion is invalid, restore the original arrangement.
    pub fn reinsert_window(&mut self, id: WindowId, at: InsertPosition) -> bool {
        let Some(idx) = self.index_of(id) else {
            return false;
        };
        let item = self.items.remove(idx);
        if self.insert_window(id, at) {
            true
        } else {
            self.items.insert(idx, item);
            false
        }
    }

    pub fn remove_window(&mut self, id: WindowId) {
        let Some(idx) = self.index_of(id) else { return };
        self.items.remove(idx);
        self.min_heights.remove(&id);
        self.min_widths.remove(&id);
        if self.focused == Some(id) {
            self.focused = self.pick_focus(idx);
        }
        self.ensure_visible();
    }

    /// Nearest visible window to position `idx` (search forward first, then backward).
    fn pick_focus(&self, idx: usize) -> Option<WindowId> {
        (idx..self.items.len())
            .find(|&k| self.items[k].span != Span::Hidden)
            .or_else(|| {
                (0..idx.min(self.items.len()))
                    .rev()
                    .find(|&k| self.items[k].span != Span::Hidden)
            })
            .map(|k| self.items[k].id)
    }

    /// Hide / show a window without losing its slot. Used when a window is minimized
    /// or reappears. True if anything changed.
    pub fn set_hidden(&mut self, id: WindowId, hide: bool) -> bool {
        let Some(i) = self.index_of(id) else {
            return false;
        };
        if hide {
            if self.items[i].span == Span::Hidden {
                return false;
            }
            self.items[i].restore = self.items[i].span;
            self.items[i].span = Span::Hidden;
            if self.focused == Some(id) {
                self.focused = self.pick_focus(i);
            }
        } else {
            if self.items[i].span != Span::Hidden {
                return false;
            }
            self.items[i].span = self.items[i].restore;
            if self.focused.is_none() {
                self.focused = Some(id);
            }
        }
        self.ensure_visible();
        true
    }

    pub fn set_focus(&mut self, id: WindowId) {
        if self
            .index_of(id)
            .map_or(false, |i| self.items[i].span != Span::Hidden)
        {
            self.focused = Some(id);
            self.ensure_visible();
        }
    }

    /// Snap the focused window.
    /// Up: make it shorter; Down: make it taller (per window).
    /// Left/Right: move it to that lane (see `snap_side`).
    pub fn snap(&mut self, dir: Dir) {
        let Some(id) = self.focused else { return };
        let Some(i) = self.index_of(id) else { return };
        match dir {
            // The user changed the height -> remeasure this window's minimum from scratch.
            Dir::Up => {
                self.min_heights.remove(&id);
                self.items[i].height = self.items[i].height.shrink();
            }
            Dir::Down => {
                self.min_heights.remove(&id);
                self.items[i].height = self.items[i].height.grow();
            }
            Dir::Left => self.snap_side(i, Span::Left),
            Dir::Right => self.snap_side(i, Span::Right),
        }
        self.ensure_visible();
    }

    /// Make window `i` full-width. Windows beside it (those starting above its
    /// bottom edge) stay above; this window moves below them.
    fn make_full(&mut self, i: usize) {
        let flow = self.flow();
        let w_bottom = flow[i].bottom();
        let mut w = self.items[i];
        w.span = Span::Full;
        let mut before = Vec::new();
        let mut after = Vec::new();
        for (j, it) in self.items.iter().enumerate() {
            if j == i {
                continue;
            }
            if flow[j].top < w_bottom {
                before.push(*it);
            } else {
                after.push(*it);
            }
        }
        before.push(w);
        before.extend(after);
        self.items = before;
    }

    /// Apply Left/Right to window `i`.
    ///
    ///  - already on that side         -> no change
    ///  - in the opposite lane         -> first make it full-width (height stays the same);
    ///                                    press again to move it to that side
    ///  - full-width                   -> enter the lane, following the rules for the
    ///    nearest occupant (X):
    ///      * target lane ends above the block bottom (there is empty space below it)
    ///                                 -> FILL GAP: insert at the bottom of the lane,
    ///                                    keeping its height; other windows do not move
    ///                                    (`fill_lane_gap`)
    ///      * opposite side of X is empty -> SWAP: move X to the other side; take X's position
    ///      * opposite side of X contains a window no taller than X
    ///        (the quarter row is full) -> insert BELOW X
    ///      * X is taller than the window and there is no empty space
    ///                                 -> insert ABOVE X; X moves down
    ///      * no X                        -> simply move to the other lane
    fn snap_side(&mut self, i: usize, target: Span) {
        let current = self.items[i].span;
        if current == target {
            return;
        }
        let opposite = target.opposite();
        if current == opposite {
            self.make_full(i);
            return;
        }

        let flow = self.flow();
        let run_start = self.items[..i]
            .iter()
            .rposition(|it| it.span == Span::Full)
            .map_or(0, |p| p + 1);
        let run_end = self.items[i + 1..]
            .iter()
            .position(|it| it.span == Span::Full)
            .map_or(self.items.len(), |p| i + 1 + p);

        // Entering a block with an existing split: use that block's ratio.
        if let Some(r) = (run_start..run_end)
            .filter(|&k| k != i && matches!(self.items[k].span, Span::Left | Span::Right))
            .map(|k| self.items[k].split)
            .next()
        {
            self.items[i].split = r;
        }

        let x = (run_start..i)
            .rev()
            .find(|&j| self.items[j].span == target)
            .or_else(|| (i + 1..run_end).find(|&j| self.items[j].span == target));

        let Some(j) = x else {
            self.items[i].span = target;
            return;
        };

        // The target lane is shorter than its block: fill the empty space at its bottom
        // without moving existing windows.
        if self.fill_lane_gap(i, j, target) {
            return;
        }

        let (tj, bj) = (flow[j].top, flow[j].bottom());
        let beside: Vec<Flow> = flow
            .iter()
            .enumerate()
            .filter(|(k, f)| {
                *k != i && *k != j && f.span == opposite && f.top < bj && f.bottom() > tj
            })
            .map(|(_, f)| *f)
            .collect();

        let above = if beside.is_empty() {
            self.items[j].span = opposite; // swap
            true
        } else {
            // Quarter row is full -> below X; if the other is taller -> above X.
            !beside.iter().all(|f| f.top >= tj && f.bottom() <= bj)
        };

        self.items[i].span = target;
        let w = self.items.remove(i);
        let jj = if j > i { j - 1 } else { j };
        self.items.insert(if above { jj } else { jj + 1 }, w);
    }

    /// Used by `snap_side` and `join`: window `i` enters the `target` lane in the block
    /// containing `anchor`. If that lane ends above the block bottom (leaving empty space
    /// below), insert the window at the bottom of the lane, keeping its own height (it may
    /// be taller than the available space; adjust it with `Win+Up/Down`). Other windows
    /// remain unchanged.
    ///
    /// Position is calculated from `flow()` without this window (including gaps, minimum
    /// heights, and hidden windows). Returns false (leaving the strip unchanged) if there
    /// is no empty space below that lane.
    fn fill_lane_gap(&mut self, i: usize, anchor: usize, target: Span) -> bool {
        let me = self.items.remove(i);
        let anchor_after = if anchor > i { anchor - 1 } else { anchor };
        let (lo, hi) = self.run_bounds(anchor_after);
        let flow = self.flow();
        let in_lane = |k: &usize| self.items[*k].span == target;
        let in_block = |k: &usize| matches!(self.items[*k].span, Span::Left | Span::Right);
        let lane_end = (lo..hi).filter(in_lane).last();
        let lane_bottom = (lo..hi).filter(in_lane).map(|k| flow[k].bottom()).max();
        let block_bottom = (lo..hi).filter(in_block).map(|k| flow[k].bottom()).max();
        let (Some(end), Some(lane_bottom), Some(block_bottom)) =
            (lane_end, lane_bottom, block_bottom)
        else {
            self.items.insert(i, me);
            return false;
        };
        if block_bottom <= lane_bottom {
            self.items.insert(i, me);
            return false;
        }
        let mut w = me;
        w.span = target;
        w.restore = target;
        w.split = self.items[anchor_after].split;
        self.items.insert(end + 1, w);
        true
    }

    /// Minimum slot height (including the gap) ever measured for window `id`.
    fn min_slot_height(&self, id: WindowId) -> i32 {
        self.min_heights
            .get(&id)
            .map_or(0, |m| m + 2 * (self.gap / 2))
    }

    /// Join the focused window to its neighbor in direction `dir` (see `join_into`).
    pub fn join(&mut self, dir: Dir) -> bool {
        let Some(f) = self.focused else { return false };
        let Some(t) = self.neighbor_of(f, dir) else {
            return false;
        };
        self.join_into(f, t)
    }

    /// Join window `src` to window `target` (keyboard version of `Split` drag-and-drop
    /// in the overview; `target` does not need to be adjacent to `src`):
    ///  - full-width `target` -> split the width, side by side in reading order
    ///    (`src` above `target` -> `src` on the left; below -> `src` on the right)
    ///  - half-width `target` -> if its lane has empty space below, fill that space
    ///    without shrinking `target`; otherwise split `target`'s height in two and stack
    ///    them in the same lane (`src` above -> place it above `target`)
    ///  - reject if `src == target`, they are in the same lane and block, either is hidden,
    ///    the height is already at its minimum (~1/4), or the minimum height will not fit.
    pub fn join_into(&mut self, src: WindowId, target: WindowId) -> bool {
        if src == target {
            return false;
        }
        let (Some(fi), Some(ti)) = (self.index_of(src), self.index_of(target)) else {
            return false;
        };
        let (fs, ts) = (self.items[fi].span, self.items[ti].span);
        if fs == Span::Hidden || ts == Span::Hidden {
            return false;
        }
        let ch = self.content().h;

        let first = match ts {
            Span::Full => {
                let target_px = self.items[ti].height.pixels(ch);
                if self.min_slot_height(src) > target_px {
                    return false;
                }
                let flow = self.flow();
                flow[fi].top < flow[ti].top // Reading order: src above -> left, below -> right.
            }
            Span::Left | Span::Right => {
                if ts == fs && self.run_bounds(fi) == self.run_bounds(ti) {
                    return false; // Already in the same lane and block.
                }
                if self.fill_lane_gap(fi, ti, ts) {
                    self.focused = Some(src);
                    self.ensure_visible();
                    return true;
                }
                let can_halve = match self.items[ti].height {
                    Height::Full | Height::Half | Height::Third => true,
                    Height::Custom(p) => p / 2 >= 150,
                };
                if !can_halve {
                    return false;
                }
                let slot_px = self.items[ti].height.halved().pixels(ch);
                if self.min_slot_height(src) > slot_px || self.min_slot_height(target) > slot_px {
                    return false;
                }
                let flow = self.flow();
                flow[fi].top <= flow[ti].top
            }
            Span::Hidden => return false,
        };
        self.reinsert_window(src, InsertPosition::Split(target, first))
    }

    /// Join a window from another workspace (`item`, not yet in this strip) to `target`.
    /// `from_left`: it came from the workspace on the left (`true` -> to the left of a
    /// full-width target). Returns false (leaving the strip unchanged) if the target is
    /// invalid or the minimum size will not fit.
    pub(crate) fn join_external_into(
        &mut self,
        item: Item,
        min_h: Option<i32>,
        min_w: Option<i32>,
        target: WindowId,
        from_left: bool,
    ) -> bool {
        if self.index_of(item.id).is_some() {
            return false;
        }
        let Some(ti) = self.index_of(target) else {
            return false;
        };
        let ts = self.items[ti].span;
        if ts == Span::Hidden {
            return false;
        }
        let ch = self.content().h;
        let src_min_slot = min_h.map_or(0, |m| m + 2 * (self.gap / 2));

        let ok = match ts {
            Span::Full => {
                if src_min_slot > self.items[ti].height.pixels(ch) {
                    return false;
                }
                self.insert_window(item.id, InsertPosition::Split(target, from_left))
            }
            Span::Left | Span::Right => {
                // Check for empty space below the target lane.
                let (lo, hi) = self.run_bounds(ti);
                let flow = self.flow();
                let in_lane = |k: &usize| self.items[*k].span == ts;
                let in_block = |k: &usize| matches!(self.items[*k].span, Span::Left | Span::Right);
                let lane_end = (lo..hi).filter(in_lane).last();
                let lane_bottom = (lo..hi).filter(in_lane).map(|k| flow[k].bottom()).max();
                let block_bottom = (lo..hi).filter(in_block).map(|k| flow[k].bottom()).max();
                if let (Some(end), Some(lb), Some(bb)) = (lane_end, lane_bottom, block_bottom) {
                    if bb > lb {
                        let mut w = item;
                        w.span = ts;
                        w.restore = ts;
                        w.split = self.items[ti].split;
                        self.items.insert(end + 1, w);
                        self.focused = Some(item.id);
                        true
                    } else {
                        self.split_external_into_lane(item.id, ti, target, src_min_slot, ch)
                    }
                } else {
                    self.split_external_into_lane(item.id, ti, target, src_min_slot, ch)
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

    fn split_external_into_lane(
        &mut self,
        src: WindowId,
        ti: usize,
        target: WindowId,
        src_min_slot: i32,
        ch: i32,
    ) -> bool {
        let can_halve = match self.items[ti].height {
            Height::Full | Height::Half | Height::Third => true,
            Height::Custom(p) => p / 2 >= 150,
        };
        if !can_halve {
            return false;
        }
        let slot_px = self.items[ti].height.halved().pixels(ch);
        if src_min_slot > slot_px || self.min_slot_height(target) > slot_px {
            return false;
        }
        self.insert_window(src, InsertPosition::Split(target, false))
    }

    /// Make the focused window full (width + height), below any windows beside it,
    /// and reset its lane ratio to 1/2.
    pub fn maximize(&mut self) {
        let Some(id) = self.focused else { return };
        let Some(i) = self.index_of(id) else { return };
        self.make_full(i);
        if let Some(k) = self.index_of(id) {
            self.items[k].height = Height::Full;
            // Restore the initial state: a previously adjusted lane ratio will not carry
            // over when this window re-enters a lane (Win+Left/Right becomes 1/2). If it
            // enters a block that already has a divider, `snap_side` still uses that ratio.
            self.items[k].split = 500;
        }
        self.ensure_visible();
    }

    /// Move focus based on the actual screen position.
    pub fn focus(&mut self, dir: Dir) {
        if let Some(n) = self.neighbor(dir) {
            self.focused = Some(n);
            self.ensure_visible();
        }
    }

    /// Swap the POSITION of the focused window with its neighbor in direction `dir`;
    /// each window keeps its own size (width / height).
    ///  - same lane, or either is full-width -> swap order
    ///  - different lanes                    -> swap sides
    pub fn move_window(&mut self, dir: Dir) {
        let Some(id) = self.focused else { return };
        let Some(n) = self.neighbor(dir) else { return };
        let (Some(a), Some(b)) = (self.index_of(id), self.index_of(n)) else {
            return;
        };
        let (sa, sb) = (self.items[a].span, self.items[b].span);
        if sa != Span::Full && sb != Span::Full && sa != sb {
            self.items[a].span = sb;
            self.items[b].span = sa;
        } else {
            self.items.swap(a, b);
        }
        // The swapped full-width window remains visible in the same place (the viewport
        // follows focus): notify the app so it can indicate the movement.
        self.cue = (sa == Span::Full).then_some((id, dir));
        self.ensure_visible();
    }

    /// Swap the positions of two specific windows (mouse drag). The rules match
    /// `move_window`: different lanes -> swap sides, otherwise swap order; each window
    /// keeps its size. Focus does not change. Returns false if either is unknown / hidden.
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId) -> bool {
        if a == b {
            return false;
        }
        let (Some(i), Some(j)) = (self.index_of(a), self.index_of(b)) else {
            return false;
        };
        let (sa, sb) = (self.items[i].span, self.items[j].span);
        if sa == Span::Hidden || sb == Span::Hidden {
            return false;
        }
        if sa != Span::Full && sb != Span::Full && sa != sb {
            self.items[i].span = sb;
            self.items[j].span = sa;
        } else {
            self.items.swap(i, j);
        }
        self.ensure_visible();
        true
    }

    /// Take (and clear) the latest swap cue: the full-width window just moved and its
    /// direction. Returns None if the last swap is already visible on screen (in a lane)
    /// or if there is no cue.
    pub fn take_move_cue(&mut self) -> Option<(WindowId, Dir)> {
        self.cue.take()
    }
}
