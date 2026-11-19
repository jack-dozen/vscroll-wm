//! Multiple strips (workspaces), like in GNOME. Pure logic, without Win32.
//!
//! Rule: there is always exactly one empty workspace at the far right. An empty
//! workspace in the middle is removed as soon as it is left (the numbers to its right
//! shift). The active workspace may be empty while occupied. "Empty" means there are
//! no items in the strip; minimized windows still count as contents.
//!
//! Adding/removing windows and switching workspaces MUST go through the methods here
//! (not `cur_mut().add_window()`, etc.) so the rules above are always maintained.

use crate::snap::SnapPreset;
use crate::types::*;

/// Saved arrangement of one workspace: ordered items (width, height, split ratio) and
/// the focused window.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedStrip {
    pub items: Vec<Item>,
    pub focused: Option<WindowId>,
}

/// Saved arrangements for all workspaces and the active workspace number.
#[derive(Debug, Clone, PartialEq)]
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

    /// For operations that do not change contents (snap, focus, move, ...).
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
            } else if !s.items().is_empty() {
                self.strips.push(s);
            }
        }
        self.active = new_active;
        if self.strips.last().map_or(true, |s| !s.items().is_empty()) {
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
                    // Snap layout slots are temporary and are not saved.
                    items: s
                        .items()
                        .iter()
                        .copied()
                        .filter(|it| !it.id.is_slot())
                        .collect(),
                    focused: s.focused().filter(|f| !f.is_slot()),
                })
                .collect(),
        }
    }

    /// Restore the saved arrangement for windows that still exist (`live`). Only applies
    /// while all workspaces are empty (at startup). Items not in `live` are discarded
    /// (workspaces that become empty are also removed); previously hidden (minimized)
    /// windows regain their original width.
    pub fn restore(&mut self, saved: &SavedLayout, live: impl Fn(WindowId) -> bool) {
        if self.strips.iter().any(|s| !s.items().is_empty()) {
            return;
        }
        let mut strips: Vec<Strip> = Vec::new();
        let mut active = 0;
        let mut seen: Vec<WindowId> = Vec::new();
        for (k, ss) in saved.strips.iter().enumerate() {
            let mut s = Strip::new(self.viewport);
            s.set_gap(self.gap);
            for it in &ss.items {
                if seen.contains(&it.id) || !live(it.id) {
                    continue;
                }
                seen.push(it.id);
                let mut item = *it;
                if item.span == Span::Hidden {
                    item.span = if item.restore == Span::Hidden {
                        Span::Full
                    } else {
                        item.restore
                    };
                }
                s.items.push(item);
            }
            if s.items.is_empty() {
                continue;
            }
            s.focused = ss
                .focused
                .filter(|f| s.index_of(*f).is_some())
                .or_else(|| s.items.first().map(|it| it.id));
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
        self.strips
            .iter()
            .position(|s| s.items().iter().any(|it| it.id == id))
    }

    /// All windows (including hidden ones) in workspace `n`.
    pub fn window_ids(&self, n: usize) -> Vec<WindowId> {
        self.strips
            .get(n)
            .map_or_else(Vec::new, |s| s.items().iter().map(|it| it.id).collect())
    }

    /// Add a new window to the active workspace.
    pub fn add_window(&mut self, id: WindowId) {
        if self.workspace_of(id).is_some() {
            return;
        }
        self.cur_mut().add_window(id);
        self.normalize();
    }

    /// Tambah window baru dengan workspace (0-indexed, 0..=8), lebar, dan tinggi awal
    /// opsional (dipakai window rules).
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

    /// Switch to workspace `n`. Returns false if `n` is out of range or already active.
    /// Other workspace numbers may shift if the one left behind was empty.
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

    /// Send the focused window to workspace `n` (which may be the empty workspace at the end).
    /// The user stays in the active workspace; focus moves to that window's neighbor.
    /// The window enters the destination at full width and height; its measured minimum
    /// size is carried over.
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

    /// Move window `id` (not only the focused one) to workspace `n`; the active workspace
    /// does not change. Same rules as `move_focused_to`. Returns false if `id` is unknown,
    /// already in `n`, or `n` is out of range.
    pub fn move_window_to(&mut self, id: WindowId, n: usize) -> bool {
        self.move_window_with(id, n, None)
    }

    /// Like `move_window_to`, but insert the window at `at` in the destination workspace
    /// (see `Strip::insert_window`); if `at` is invalid, add it as usual.
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

    /// Move a window to `at` in its own workspace (drag into an empty space).
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

    /// Swap the positions of two windows in the same workspace (mouse drag).
    pub fn swap_windows(&mut self, a: WindowId, b: WindowId) -> bool {
        match (self.workspace_of(a), self.workspace_of(b)) {
            (Some(x), Some(y)) if x == y => self.strips[x].swap_windows(a, b),
            _ => false,
        }
    }

    /// Join window `src` to `target`, whether in the same or a different workspace.
    /// If they are in different workspaces, move `src` to `target`'s workspace, make
    /// that workspace active, and tidy up the old workspace if it becomes empty.
    /// Returns false (without changes) if the insertion is rejected.
    pub fn join_into(&mut self, src: WindowId, target: WindowId) -> bool {
        let (Some(from), Some(to)) = (self.workspace_of(src), self.workspace_of(target)) else {
            return false;
        };
        if from == to {
            return self.strips[from].join_into(src, target);
        }
        let Some(item) = self.strips[from]
            .items()
            .iter()
            .find(|it| it.id == src)
            .copied()
        else {
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

    /// Like `move_focused_to`, to the previous / next workspace (does not wrap).
    pub fn move_focused_rel(&mut self, delta: i32) -> Option<WindowId> {
        let n = self.active as i64 + delta as i64;
        usize::try_from(n).ok().and_then(|n| self.move_focused_to(n))
    }

    /// Overview maps for each workspace with visible windows, ordered from left to right,
    /// followed by their numbers (0 = first). Empty workspaces / those containing only
    /// hidden windows are skipped.
    pub fn overviews(&self) -> Vec<(usize, Overview)> {
        self.strips
            .iter()
            .enumerate()
            .map(|(n, s)| (n, s.overview()))
            .filter(|(_, o)| !o.items.is_empty())
            .collect()
    }

    /// Nearest workspace with visible windows from `from` in the `forward` direction.
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

    /// Like `adjacent_workspace`, but wraps: from the last end (`forward`), continue to
    /// the first workspace with visible windows, and vice versa. Returns None if no other
    /// workspace has visible windows.
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

    /// Representative window for workspace `n`: the focused one, or the first visible one.
    /// Returns None if the workspace does not exist / has no visible windows.
    pub fn representative(&self, n: usize) -> Option<WindowId> {
        let s = self.strips.get(n)?;
        let ov = s.overview();
        s.focused()
            .filter(|f| ov.items.iter().any(|it| it.id == *f))
            .or_else(|| ov.items.first().map(|it| it.id))
    }

    /// Window where the cursor enters when crossing into workspace `n` from the side.
    /// `from_left`: enter from the left edge (pressing Right). Choose the window at that
    /// edge that is vertically closest to `y` (strip coordinate).
    pub fn entry_window(&self, n: usize, from_left: bool, y: i32) -> Option<WindowId> {
        const SLACK: i32 = 2; // Rounded lane widths: allow a few pixels of tolerance.
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

    /// Apply a snap layout preset in the active workspace: `id` fills zone `zone`, and
    /// the other zones become empty slots (see `Strip::apply_snap`). Returns empty if
    /// `id` is not in the active workspace.
    pub fn apply_snap(&mut self, id: WindowId, preset: &SnapPreset, zone: usize) -> Vec<WindowId> {
        if self.workspace_of(id) != Some(self.active) {
            return Vec::new();
        }
        self.cur_mut().apply_snap(id, preset, zone)
    }

    /// Fill a slot in the active workspace with window `id` from any workspace (first
    /// remove it from its previous location and move it to the active workspace).
    /// Returns false without changes if the slot does not exist, `id` is unknown, or it is hidden.
    pub fn fill_slot(&mut self, slot: WindowId, id: WindowId) -> bool {
        let a = self.active;
        if id.is_slot() || self.strips[a].index_of(slot).is_none() {
            return false;
        }
        let Some(from) = self.workspace_of(id) else {
            return false;
        };
        if self.strips[from]
            .items()
            .iter()
            .any(|it| it.id == id && it.span == Span::Hidden)
        {
            return false;
        }
        self.strips[from].remove_window(id);
        let ok = self.strips[a].fill_slot(slot, id);
        if !ok {
            self.strips[a].add_window(id); // The window must not be lost from the strip.
        }
        self.normalize();
        ok
    }

    /// Remove all empty slots from all workspaces. True if any were removed.
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

    /// Are there any empty slots waiting to be filled?
    pub fn has_slots(&self) -> bool {
        self.strips
            .iter()
            .any(|s| s.items().iter().any(|it| it.id.is_slot()))
    }

    /// Focus `id` in its owning workspace; switch to it if it is not active.
    /// True if the active workspace changed.
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
        assert!(w.cur().items().is_empty());
    }

    #[test]
    fn first_window_opens_an_empty_workspace_next_to_it() {
        let mut w = ws();
        w.add_window(A);
        assert_eq!((w.count(), w.active()), (2, 0));
        assert_eq!(w.workspace_of(A), Some(0));
        assert!(w.strip(1).unwrap().items().is_empty());
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
        assert_eq!(
            (w.workspace_of(A), w.workspace_of(B), w.workspace_of(C)),
            (Some(0), Some(1), Some(2))
        );
    }

    #[test]
    fn empty_workspace_in_the_middle_goes_away_when_left() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B); // [A] [B] []
        w.remove_window(B);
        // The active workspace may be empty while occupied; it becomes the empty one at the end.
        assert_eq!((w.count(), w.active()), (2, 1));
        assert!(w.switch_to(0));
        assert_eq!((w.count(), w.active()), (2, 0)); // [A] []
    }

    #[test]
    fn removing_in_an_inactive_workspace_shifts_the_active_index() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B); // aktif = 1
        w.remove_window(A); // Workspace 0 is empty -> removed.
        assert_eq!((w.count(), w.active()), (2, 0));
        assert_eq!(w.workspace_of(B), Some(0));
    }

    #[test]
    fn moving_does_not_wrap_and_stops_at_the_ends() {
        let mut w = ws();
        w.add_window(A);
        assert!(!w.switch_rel(-1));
        assert!(w.switch_rel(1)); // Switch to the empty workspace at the end.
        assert_eq!((w.count(), w.active()), (2, 1));
        assert!(!w.switch_rel(1));
        assert!(w.switch_rel(-1));
        assert!(!w.switch_to(0)); // Already active.
        assert!(!w.switch_to(9));
    }

    #[test]
    fn leaving_the_trailing_empty_workspace_keeps_exactly_one_at_the_end() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        assert!(w.switch_to(0));
        assert_eq!(w.count(), 2); // [A] []
        assert!(w.strip(1).unwrap().items().is_empty());
    }

    #[test]
    fn hidden_windows_still_count_as_content() {
        let mut w = ws();
        w.add_window(A);
        assert!(w.set_hidden(A, true));
        w.switch_to(1);
        assert_eq!(w.workspace_of(A), Some(0)); // Workspace 0 is not removed.
        assert_eq!(w.count(), 2);
    }

    #[test]
    fn move_focused_sends_the_window_and_stays_put() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B); // B is focused.
        assert_eq!(w.move_focused_to(1), Some(B));
        assert_eq!(w.active(), 0);
        assert_eq!(w.cur().focused(), Some(A));
        assert_eq!((w.workspace_of(A), w.workspace_of(B)), (Some(0), Some(1)));
        assert_eq!(w.count(), 3); // [A] [B] []
        assert_eq!(w.move_focused_to(0), None); // Destination is the active workspace.
        assert_eq!(w.move_focused_to(7), None);
    }

    #[test]
    fn moving_the_last_window_leaves_an_empty_active_workspace_until_you_leave() {
        let mut w = ws();
        w.add_window(A);
        assert_eq!(w.move_focused_to(1), Some(A));
        assert_eq!(w.count(), 3); // [] [A] []
        assert!(w.cur().items().is_empty());
        assert!(w.switch_to(1));
        assert_eq!((w.count(), w.active()), (2, 0)); // [A] []
        assert_eq!(w.workspace_of(A), Some(0));
    }

    #[test]
    fn move_focused_rel_follows_neighbours_and_has_no_wrap() {
        let mut w = ws();
        w.add_window(A);
        assert_eq!(w.move_focused_rel(-1), None); // Already at the far left.
        w.switch_to(1);
        w.add_window(B); // [A] [B] []
        assert_eq!(w.move_focused_rel(-1), Some(B));
        assert_eq!(w.workspace_of(B), Some(0));
        assert_eq!(w.move_focused_rel(1), None); // The active workspace is now empty.
    }

    #[test]
    fn move_carries_measured_minimums() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B);
        w.cur_mut().set_min_height(B, 700);
        w.cur_mut().set_min_width(B, 500);
        assert_eq!(w.move_focused_to(1), Some(B));
        assert_eq!(w.strips[1].min_heights.get(&B), Some(&700));
        assert_eq!(w.strips[1].min_widths.get(&B), Some(&500));
        assert_eq!(w.strips[0].min_heights.get(&B), None);
    }

    #[test]
    fn focus_window_switches_to_the_owning_workspace() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B); // [A] [B] []
        assert!(w.focus_window(A));
        assert_eq!(w.active(), 0);
        assert_eq!(w.cur().focused(), Some(A));
        assert!(!w.focus_window(A)); // Already there.
        assert!(!w.focus_window(WindowId(99)));
    }

    #[test]
    fn viewport_and_gap_apply_to_every_strip_including_new_ones() {
        let mut w = ws();
        w.set_gap(20);
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B);
        let r = w.cur().layout()[0].rect;
        assert_eq!((r.x, r.w), (20, 1880));
        w.set_viewport(rect(1280, 720));
        w.switch_to(2); // Empty workspace at the end.
        w.add_window(C);
        assert_eq!(w.cur().layout()[0].rect.w, 1240);
        w.switch_to(1);
        assert_eq!(w.cur().layout()[0].rect.w, 1240);
    }

    #[test]
    fn overviews_list_only_workspaces_with_visible_windows() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B); // [A] [B] []
        let idx: Vec<usize> = w.overviews().iter().map(|(n, _)| *n).collect();
        assert_eq!(idx, vec![0, 1]);
        w.set_hidden(B, true); // Only hidden windows -> nothing can be selected.
        let idx: Vec<usize> = w.overviews().iter().map(|(n, _)| *n).collect();
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn adjacent_workspace_skips_empty_ones_and_stops_at_the_ends() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B);
        w.switch_to(2);
        w.add_window(C); // [A] [B] [C] []
        assert_eq!(w.adjacent_workspace(0, true), Some(1));
        assert_eq!(w.adjacent_workspace(2, true), None); // The empty workspace at the end is skipped.
        assert_eq!(w.adjacent_workspace(0, false), None);
        assert_eq!(w.adjacent_workspace(2, false), Some(1));
        // The active workspace is empty in the middle: it can still be crossed.
        w.switch_to(1);
        w.remove_window(B); // [A] [] [C] ...
        assert_eq!(w.adjacent_workspace(1, true), Some(2));
        assert_eq!(w.adjacent_workspace(1, false), Some(0));
    }

    #[test]
    fn representative_prefers_the_focused_window() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B); // B fokus
        assert_eq!(w.representative(0), Some(B));
        w.cur_mut().set_focus(A);
        assert_eq!(w.representative(0), Some(A));
        assert_eq!(w.representative(1), None); // Empty at the end.
        assert_eq!(w.representative(9), None);
    }

    #[test]
    fn entry_window_picks_the_edge_window_closest_in_height() {
        let mut w = ws();
        w.add_window(A);
        w.cur_mut().snap(Dir::Left);
        w.add_window(B);
        w.cur_mut().snap(Dir::Right); // [A left] [B right]
        assert_eq!(w.entry_window(0, true, 500), Some(A)); // Enter from the left.
        assert_eq!(w.entry_window(0, false, 500), Some(B)); // Enter from the right.
        assert_eq!(w.entry_window(5, true, 0), None); // Workspace does not exist.
        assert_eq!(w.entry_window(1, true, 0), None); // Empty at the end.

        // Two full-width windows are stacked: the one at the cursor's height wins.
        let mut w = ws();
        w.add_window(A);
        w.cur_mut().snap(Dir::Up); // Half-height: center y = 270.
        w.add_window(B); // Below it: center y = 1080.
        assert_eq!(w.entry_window(0, true, 100), Some(A));
        assert_eq!(w.entry_window(0, false, 1200), Some(B));
    }

    #[test]
    fn move_window_to_works_for_any_window_and_keeps_the_active_workspace() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B);
        w.switch_to(1);
        w.add_window(C); // [A B] [C] []
        assert!(w.move_window_to(A, 1)); // Not the focused window.
        assert_eq!(w.workspace_of(A), Some(1));
        assert_eq!(w.workspace_of(B), Some(0));
        assert_eq!(w.active(), 1);
        assert!(!w.move_window_to(A, 1)); // Already there.
        assert!(!w.move_window_to(A, 99)); // Out of range.
        assert!(!w.move_window_to(X, 0)); // Unknown.
        // Moving to the empty workspace at the end is allowed; remove the old one if it is empty.
        let last = w.count() - 1;
        assert!(w.move_window_to(B, last));
        assert!(w.workspace_of(B).is_some());
        assert_ne!(w.workspace_of(B), w.workspace_of(C)); // Separate workspaces.
    }

    #[test]
    fn move_window_at_places_the_window_in_the_target_block() {
        let mut w = ws();
        w.add_window(A);
        w.cur_mut().snap(Dir::Left);
        w.switch_to(1);
        w.add_window(X); // [A] [X] []
        assert!(w.move_window_at(X, 0, InsertPosition::Beside(A)));
        assert_eq!(w.workspace_of(X), Some(0));
        let items = w.strip(0).unwrap().items();
        assert_eq!((items[0].id, items[0].span), (A, Span::Left));
        assert_eq!((items[1].id, items[1].span), (X, Span::Right));

        // Invalid insertion point (target does not exist): still add it to that workspace.
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(X);
        assert!(w.move_window_at(X, 0, InsertPosition::Beside(C)));
        assert_eq!(w.workspace_of(X), Some(0));
    }

    #[test]
    fn swap_windows_only_within_one_workspace() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B);
        w.switch_to(1);
        w.add_window(C); // [A B] [C]
        assert!(w.swap_windows(A, B));
        assert_eq!(w.strip(0).unwrap().items()[0].id, B);
        assert!(!w.swap_windows(A, C)); // Different workspaces.
        assert!(!w.swap_windows(A, X)); // Unknown.
    }

    #[test]
    fn restore_brings_back_order_widths_focus_and_active_workspace() {
        let mut w = ws();
        w.add_window(A);
        w.cur_mut().snap(Dir::Left);
        w.add_window(B);
        w.cur_mut().snap(Dir::Up);
        w.switch_to(1);
        w.add_window(C); // [A B] [C] []
        let saved = w.snapshot();

        let mut fresh = ws();
        fresh.restore(&saved, |_| true);
        assert_eq!(fresh.snapshot(), saved);
        assert_eq!(fresh.active(), 1);

        // Windows that no longer exist are discarded; workspaces that become empty are also removed.
        let mut part = ws();
        part.restore(&saved, |id| id != C);
        assert_eq!(part.workspace_of(C), None);
        assert_eq!(part.workspace_of(A), Some(0));
        assert_eq!(part.active(), 0);

        // Does not overwrite an existing arrangement.
        let mut busy = ws();
        busy.add_window(X);
        busy.restore(&saved, |_| true);
        assert_eq!(busy.workspace_of(A), None);
        assert_eq!(busy.workspace_of(X), Some(0));

        // Nothing is still alive: no changes.
        let mut none = ws();
        none.restore(&saved, |_| false);
        assert!(none.overviews().is_empty());
    }

    #[test]
    fn wrapped_workspace_jumps_to_the_opposite_end() {
        let mut w = ws();
        w.add_window(A);
        w.switch_to(1);
        w.add_window(B);
        w.switch_to(2);
        w.add_window(C); // [A] [B] [C] []
        assert_eq!(w.wrapped_workspace(2, true), Some(0)); // After the last: first.
        assert_eq!(w.wrapped_workspace(0, false), Some(2)); // Before the first: last.
        assert_eq!(w.wrapped_workspace(1, true), Some(0)); // Always wrap to the opposite end.
        // Only one workspace has contents: there is no other destination.
        let mut w = ws();
        w.add_window(A);
        assert_eq!(w.wrapped_workspace(0, true), None);
        assert_eq!(w.wrapped_workspace(0, false), None);
    }

    #[test]
    fn join_into_across_workspaces_splits_and_cleans_empty_source() {
        let mut w = ws();
        w.add_window(A); // ws 0: [A]
        w.switch_to(1);
        w.add_window(B); // ws 1: [B]
        // Join A (from the left) to B (ws 1): A on the left, B on the right; clean up empty ws 0.
        assert!(w.join_into(A, B));
        assert_eq!(w.active(), 0);
        assert_eq!(w.workspace_of(A), Some(0));
        assert_eq!(w.workspace_of(B), Some(0));
        let ov = w.cur().overview();
        let rect_of = |id| ov.items.iter().find(|i| i.id == id).unwrap().rect;
        assert_eq!(rect_of(A), Rect { x: 0, y: 0, w: 960, h: 1080 });
        assert_eq!(rect_of(B), Rect { x: 960, y: 0, w: 960, h: 1080 });
        assert_eq!(w.cur().focused(), Some(A));

        // Join C from ws 1 (right) to B (right lane of ws 0) -> stack below B.
        w.switch_to(1);
        w.add_window(C);
        assert!(w.join_into(C, B));
        assert_eq!(w.workspace_of(C), Some(0));
        let ov = w.cur().overview();
        let rect_of = |id| ov.items.iter().find(|i| i.id == id).unwrap().rect;
        assert_eq!(rect_of(B).h, 540);
        assert_eq!(rect_of(C).h, 540);
        assert_eq!(rect_of(C).y, 540);
        assert_eq!(rect_of(C).x, rect_of(B).x);
    }

    #[test]
    fn join_into_across_workspaces_rejects_min_height_without_moving() {
        let mut w = ws();
        w.add_window(A);
        w.add_window(B);
        assert!(w.cur_mut().join(Dir::Up)); // A left, B right.
        w.cur_mut().set_min_height(B, 600);
        w.switch_to(1);
        w.add_window(C);
        assert!(!w.join_into(C, B));
        // C stays in ws 1; it does not move.
        assert_eq!(w.workspace_of(C), Some(1));
        assert_eq!(w.workspace_of(B), Some(0));
    }

    #[test]
    fn add_window_with_places_window_in_target_workspace_with_span_and_height() {
        let mut w = ws();
        w.add_window(A);
        // Buka B langsung di workspace 2 (indeks 1), lajur kanan, tinggi 1/2
        w.add_window_with(B, Some(1), Some(Span::Right), Some(Height::Half));
        assert_eq!(w.workspace_of(A), Some(0));
        assert_eq!(w.workspace_of(B), Some(1));
        assert_eq!(w.active(), 0); // workspace aktif tidak berubah sendiri
        let b = w.strip(1).unwrap().items()[0];
        assert_eq!((b.id, b.span, b.height), (B, Span::Right, Height::Half));
    }
}
