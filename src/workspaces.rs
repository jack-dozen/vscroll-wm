//! Multiple strips (workspaces), like in GNOME. Pure logic, without Win32.
//!
//! Each workspace is a `Strip` containing vertical `Space` pages.
//! Rule: there is always exactly one empty workspace at the far right.

use crate::snap::SnapPreset;
use crate::types::*;

/// Saved arrangement of one Space inside a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSpace {
    pub split: i32,
    pub items: Vec<Item>,
}

/// Saved arrangement of one workspace: ordered Spaces and the focused window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedStrip {
    pub spaces: Vec<SavedSpace>,
    pub focused: Option<WindowId>,
}

/// Saved arrangements for all workspaces and the active workspace number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedLayout {
    pub active: usize,
    pub strips: Vec<SavedStrip>,
}

pub struct Workspaces {
    strips: Vec<Strip>,
    active: usize,
    viewport: Rect,
    gap: i32,
}

impl Workspaces {
    pub fn new(viewport: Rect) -> Self {
        Self {
            strips: vec![Strip::new(viewport)],
            active: 0,
            viewport,
            gap: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.strips.len()
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn cur(&self) -> &Strip {
        &self.strips[self.active]
    }

    /// For operations that do not change workspace membership (snap, focus, move, ...).
    pub fn cur_mut(&mut self) -> &mut Strip {
        &mut self.strips[self.active]
    }

    pub fn strip(&self, n: usize) -> Option<&Strip> {
        self.strips.get(n)
    }

    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = viewport;
        for s in &mut self.strips {
            s.set_viewport(viewport);
        }
    }

    pub fn set_gap(&mut self, gap: i32) {
        self.gap = gap.max(0);
        for s in &mut self.strips {
            s.set_gap(gap);
        }
    }

    /// Remove empty workspaces other than the active one, then ensure the last one is empty.
    fn normalize(&mut self) {
        let old = std::mem::take(&mut self.strips);
        let mut new_active = 0;
        for (k, s) in old.into_iter().enumerate() {
            if k == self.active {
                new_active = self.strips.len();
                self.strips.push(s);
            } else if !s.is_empty() {
                self.strips.push(s);
            }
        }
        self.active = new_active;
        if self.strips.last().map_or(true, |s| !s.is_empty()) {
            let mut s = Strip::new(self.viewport);
            s.set_gap(self.gap);
            self.strips.push(s);
        }
    }

    /// Copy of the current arrangement, for saving to a file.
    pub fn snapshot(&self) -> SavedLayout {
        SavedLayout {
            active: self.active,
            strips: self
                .strips
                .iter()
                .map(|s| SavedStrip {
                    spaces: s
                        .spaces()
                        .iter()
                        .filter_map(|sp| {
                            let items: Vec<Item> = sp
                                .items
                                .iter()
                                .copied()
                                .filter(|it| !it.id.is_slot())
                                .collect();
                            (!items.is_empty()).then_some(SavedSpace {
                                split: sp.split,
                                items,
                            })
                        })
                        .collect(),
                    focused: s.focused().filter(|f| !f.is_slot()),
                })
                .collect(),
        }
    }

    /// Restore the saved arrangement for windows that still exist (`live`).
    pub fn restore(&mut self, saved: &SavedLayout, live: impl Fn(WindowId) -> bool) {
        if self.strips.iter().any(|s| !s.is_empty()) {
            return;
        }
        let mut strips: Vec<Strip> = Vec::new();
        let mut active = 0;
        let mut seen: Vec<WindowId> = Vec::new();
        for (k, ss) in saved.strips.iter().enumerate() {
            let mut s = Strip::new(self.viewport);
            s.set_gap(self.gap);
            for saved_sp in &ss.spaces {
                let mut sp = Space {
                    items: Vec::new(),
                    split: saved_sp.split.clamp(150, 850),
                };
                for it in &saved_sp.items {
                    if seen.contains(&it.id) || !live(it.id) {
                        continue;
                    }
                    seen.push(it.id);
                    let mut item = *it;
                    item.split = sp.split;
                    item.top = item.top.clamp(0, 850);
                    if item.span == Span::Hidden {
                        item.span = if item.restore == Span::Hidden {
                            Span::Full
                        } else {
                            item.restore
                        };
                    }
                    sp.items.push(item);
                }
                if !sp.items.is_empty() {
                    s.spaces.push(sp);
                }
            }
            if s.is_empty() {
                continue;
            }
            s.focused = ss
                .focused
                .filter(|f| s.find_item(*f).is_some())
                .or_else(|| s.spaces.first().and_then(|sp| sp.items.first()).map(|it| it.id));
            s.ensure_visible();
            if k == saved.active {
                active = strips.len();
            }
            strips.push(s);
        }
        if strips.is_empty() {
            return;
        }
        self.strips = strips;
        self.active = active;
        self.normalize();
    }

    /// Workspace number containing `id`.
    pub fn workspace_of(&self, id: WindowId) -> Option<usize> {
        self.strips.iter().position(|s| s.find_item(id).is_some())
    }

    /// All windows (including hidden ones) in workspace `n`.
    pub fn window_ids(&self, n: usize) -> Vec<WindowId> {
        self.strips.get(n).map_or_else(Vec::new, |s| {
            s.spaces()
                .iter()
                .flat_map(|sp| sp.items.iter().map(|it| it.id))
                .collect()
        })
    }

    /// Add a new window to the active workspace (as a new Full Space).
    pub fn add_window(&mut self, id: WindowId) {
        if self.workspace_of(id).is_some() {
            return;
        }
        self.cur_mut().add_window(id);
        self.normalize();
    }

    /// Add a new window with optional workspace (0-indexed, 0..=8), span, and height.
    pub fn add_window_with(
        &mut self,
        id: WindowId,
        workspace: Option<usize>,
        span: Option<Span>,
        height: Option<Height>,
    ) {
        if self.workspace_of(id).is_some() {
            return;
        }
        let target = match workspace {
            Some(n) => {
                let n = n.min(8);
                while self.strips.len() <= n {
                    let mut s = Strip::new(self.viewport);
                    s.set_gap(self.gap);
                    self.strips.push(s);
                }
                n
            }
            None => self.active,
        };
        self.strips[target].add_window_with(id, span, height);
        self.normalize();
    }

    /// Remove a window from any workspace.
    pub fn remove_window(&mut self, id: WindowId) {
        if let Some(n) = self.workspace_of(id) {
            self.strips[n].remove_window(id);
            self.normalize();
        }
    }

    /// Minimized / cloaked: keep the slot in its owning workspace.
    pub fn set_hidden(&mut self, id: WindowId, hide: bool) -> bool {
        match self.workspace_of(id) {
            Some(n) => self.strips[n].set_hidden(id, hide),
            None => false,
        }
    }

    /// Switch to workspace `n`.
    pub fn switch_to(&mut self, n: usize) -> bool {
        if n == self.active || n >= self.strips.len() {
            return false;
        }
        self.active = n;
        self.normalize();
        true
    }

    /// Previous workspace (`delta` < 0) / next workspace (> 0). Does not wrap.
    pub fn switch_rel(&mut self, delta: i32) -> bool {
        let n = self.active as i64 + delta as i64;
        usize::try_from(n).map_or(false, |n| self.switch_to(n))
    }

    /// Send the focused window to workspace `n` (as a new Full Space).
    pub fn move_focused_to(&mut self, n: usize) -> Option<WindowId> {
        let a = self.active;
        if n == a || n >= self.strips.len() {
            return None;
        }
        let id = self.strips[a].focused()?;
        let min_h = self.strips[a].min_heights.get(&id).copied();
        let min_w = self.strips[a].min_widths.get(&id).copied();
        self.strips[a].remove_window(id);
        let dst = &mut self.strips[n];
        dst.add_window(id);
        if let Some(h) = min_h {
            dst.min_heights.insert(id, h);
        }
        if let Some(w) = min_w {
            dst.min_widths.insert(id, w);
        }
        dst.ensure_visible();
        self.normalize();
        Some(id)
    }

    /// Move window `id` to workspace `n` without changing the active workspace.
    pub fn move_window_to(&mut self, id: WindowId, n: usize) -> bool {
        self.move_window_with(id, n, None)
    }

    /// Like `move_window_to`, but insert the window at `at` in the destination workspace.
    pub fn move_window_at(&mut self, id: WindowId, n: usize, at: InsertPosition) -> bool {
        self.move_window_with(id, n, Some(at))
    }

    fn move_window_with(&mut self, id: WindowId, n: usize, at: Option<InsertPosition>) -> bool {
        let Some(from) = self.workspace_of(id) else {
            return false;
        };
        if from == n || n >= self.strips.len() {
            return false;
        }
        let min_h = self.strips[from].min_heights.get(&id).copied();
        let min_w = self.strips[from].min_widths.get(&id).copied();
        self.strips[from].remove_window(id);
        let dst = &mut self.strips[n];
        if !at.is_some_and(|p| dst.insert_window(id, p)) {
            dst.add_window(id);
        }
        if let Some(h) = min_h {
            dst.min_heights.insert(id, h);
        }
        if let Some(w) = min_w {
            dst.min_widths.insert(id, w);
        }
        dst.ensure_visible();
        self.normalize();
        true
    }

    /// Move a window to `at` in its own workspace.
    pub fn reposition_window(&mut self, id: WindowId, at: InsertPosition) -> bool {
        let Some(n) = self.workspace_of(id) else {
            return false;
        };
        let ok = self.strips[n].reinsert_window(id, at);
        if ok {
            self.normalize();
        }
        ok
    }

    /// Swap the positions of two windows in the same workspace.
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId) -> bool {
        match (self.workspace_of(a), self.workspace_of(b)) {
            (Some(x), Some(y)) if x == y => self.strips[x].swap_windows(a, b),
            _ => false,
        }
    }

    /// Join window `src` to `target`, in the same or another workspace.
    pub fn join_into(&mut self, src: WindowId, target: WindowId) -> bool {
        let (Some(from), Some(to)) = (self.workspace_of(src), self.workspace_of(target)) else {
            return false;
        };
        if from == to {
            return self.strips[from].join_into(src, target);
        }
        let Some(item) = self.strips[from].item(src) else {
            return false;
        };
        let min_h = self.strips[from].min_heights.get(&src).copied();
        let min_w = self.strips[from].min_widths.get(&src).copied();
        if !self.strips[to].join_external_into(item, min_h, min_w, target, from < to) {
            return false;
        }
        self.strips[from].remove_window(src);
        self.active = to;
        self.normalize();
        true
    }

    pub fn move_focused_rel(&mut self, delta: i32) -> Option<WindowId> {
        let n = self.active as i64 + delta as i64;
        usize::try_from(n).ok().and_then(|n| self.move_focused_to(n))
    }

    pub fn overviews(&self) -> Vec<(usize, Overview)> {
        self.strips
            .iter()
            .enumerate()
            .map(|(n, s)| (n, s.overview()))
            .filter(|(_, o)| !o.items.is_empty())
            .collect()
    }

    pub fn adjacent_workspace(&self, from: usize, forward: bool) -> Option<usize> {
        let has = |n: usize| {
            self.strips
                .get(n)
                .is_some_and(|s| !s.overview().items.is_empty())
        };
        if forward {
            (from + 1..self.strips.len()).find(|&n| has(n))
        } else {
            (0..from.min(self.strips.len())).rev().find(|&n| has(n))
        }
    }

    pub fn wrapped_workspace(&self, from: usize, forward: bool) -> Option<usize> {
        let has = |n: usize| {
            self.strips
                .get(n)
                .is_some_and(|s| !s.overview().items.is_empty())
        };
        let target = if forward {
            (0..self.strips.len()).find(|&n| has(n))
        } else {
            (0..self.strips.len()).rev().find(|&n| has(n))
        };
        target.filter(|&n| n != from)
    }

    pub fn representative(&self, n: usize) -> Option<WindowId> {
        let s = self.strips.get(n)?;
        let ov = s.overview();
        s.focused()
            .filter(|f| ov.items.iter().any(|it| it.id == *f))
            .or_else(|| ov.items.first().map(|it| it.id))
    }

    pub fn entry_window(&self, n: usize, from_left: bool, y: i32) -> Option<WindowId> {
        const SLACK: i32 = 2;
        let ov = self.strips.get(n)?.overview();
        let edge = if from_left {
            ov.items.iter().map(|i| i.rect.x).min()?
        } else {
            ov.items.iter().map(|i| i.rect.x + i.rect.w).max()?
        };
        ov.items
            .iter()
            .filter(|i| {
                if from_left {
                    i.rect.x - edge <= SLACK
                } else {
                    edge - (i.rect.x + i.rect.w) <= SLACK
                }
            })
            .min_by_key(|i| ((i.rect.y + i.rect.h / 2 - y).abs(), i.rect.y))
            .map(|i| i.id)
    }

    pub fn apply_snap(&mut self, id: WindowId, preset: &SnapPreset, zone: usize) -> Vec<WindowId> {
        if self.workspace_of(id) != Some(self.active) {
            return Vec::new();
        }
        self.cur_mut().apply_snap(id, preset, zone)
    }

    pub fn fill_slot(&mut self, slot: WindowId, id: WindowId) -> bool {
        let a = self.active;
        if id.is_slot() || self.strips[a].find_item(slot).is_none() {
            return false;
        }
        let Some(from) = self.workspace_of(id) else {
            return false;
        };
        if self.strips[from]
            .item(id)
            .is_some_and(|it| it.span == Span::Hidden)
        {
            return false;
        }
        self.strips[from].remove_window(id);
        let ok = self.strips[a].fill_slot(slot, id);
        if !ok {
            self.strips[a].add_window(id);
        }
        self.normalize();
        ok
    }

    pub fn clear_slots(&mut self) -> bool {
        let mut any = false;
        for s in &mut self.strips {
            any |= s.clear_slots();
        }
        if any {
            self.normalize();
        }
        any
    }

    pub fn has_slots(&self) -> bool {
        self.strips.iter().any(|s| !s.slots().is_empty())
    }

    pub fn focus_window(&mut self, id: WindowId) -> bool {
        let Some(n) = self.workspace_of(id) else {
            return false;
        };
        let switched = self.switch_to(n);
        self.cur_mut().set_focus(id);
        switched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WindowId = WindowId(1);
    const B: WindowId = WindowId(2);
    const C: WindowId = WindowId(3);
    const X: WindowId = WindowId(4);

    fn rect(w: i32, h: i32) -> Rect {
        Rect { x: 0, y: 0, w, h }
    }

    fn ws() -> Workspaces {
        Workspaces::new(rect(1920, 1080))
    }

    #[test]
    fn starts_with_a_single_empty_workspace() {
        let w = ws();
        assert_eq!((w.count(), w.active()), (1, 0));
        assert!(w.cur().is_empty());
    }

    #[test]
    fn first_window_opens_an_empty_workspace_next_to_it() {
        let mut w = ws();
        w.add_window(A);
        assert_eq!((w.count(), w.active()), (2, 0));
        assert_eq!(w.workspace_of(A), Some(0));
        assert!(w.strip(1).unwrap().is_empty());
    }

    #[test]
    fn workspaces_keep_growing_to_the_right() {
        let mut w = ws();
        w.add_window(A);
        assert!(w.switch_to(1));
        w.add_window(B);
        assert_eq!(w.count(), 3);
        assert!(w.switch_to(2));
        w.add_window(C);
        assert_eq!(w.count(), 4);
    }

    #[test]
    fn empty_workspace_in_the_middle_goes_away_when_left() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B);
        w.remove_window(B);
        assert_eq!((w.count(), w.active()), (2, 1));
        assert!(w.switch_to(0));
        assert_eq!((w.count(), w.active()), (2, 0));
    }

    #[test]
    fn hidden_windows_still_count_as_content() {
        let mut w = ws();
        w.add_window(A);
        assert!(w.set_hidden(A, true));
        w.switch_to(1);
        assert_eq!(w.workspace_of(A), Some(0));
        assert_eq!(w.count(), 2);
    }

    #[test]
    fn move_focused_sends_window_and_stays_put() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B);
        assert_eq!(w.move_focused_to(1), Some(B));
        assert_eq!(w.active(), 0);
        assert_eq!(w.cur().focused(), Some(A));
        assert_eq!((w.workspace_of(A), w.workspace_of(B)), (Some(0), Some(1)));
    }

    #[test]
    fn snapshot_and_restore_preserves_spaces_and_positions() {
        let mut w = ws();
        w.add_window(A);
        w.cur_mut().snap(Dir::Left);
        w.add_window(B);
        w.cur_mut().snap(Dir::Right); // merges into Space 0
        w.cur_mut().snap(Dir::Down); // B is now (Right, Bottom)
        w.add_window(C); // Space 1
        let snap = w.snapshot();

        let mut w2 = ws();
        w2.restore(&snap, |_| true);
        assert_eq!(w2.cur().spaces().len(), 2);
        assert_eq!(w2.cur().spaces()[0].items.len(), 2);
        assert_eq!(w2.cur().spaces()[0].items[1].top, 500);
        assert_eq!(w2.cur().spaces()[1].items[0].id, C);
        assert_eq!(w2.cur().focused(), Some(C));
    }

    #[test]
    fn move_window_to_and_at_work_across_workspaces() {
        let mut w = ws();
        w.add_window(A);
        w.cur_mut().snap(Dir::Left);
        w.switch_to(1);
        w.add_window(X);
        assert!(w.move_window_at(X, 0, InsertPosition::Beside(A)));
        assert_eq!(w.strip(0).unwrap().spaces().len(), 1);
        assert_eq!(w.strip(0).unwrap().spaces()[0].items.len(), 2);
    }
}
