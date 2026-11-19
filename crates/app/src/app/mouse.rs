//! Mouse: drag a window to swap positions with another, or drag an edge adjacent
//! to the column divider to move it. Uses built-in Windows drag / resize (the window
//! follows the cursor normally); only the result is read here when the button is
//! released (EVENT_SYSTEM_MOVESIZEEND). Other movement is reverted by sync / apply
//! as before. Disabled via `mouse_drag = off`.

use std::sync::atomic::Ordering;

use layout::{Span, WindowId};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use super::App;
use crate::tray;
use crate::winutil::{bounds, ext_bounds, wid, Bounds};

/// Tolerance (pixels) for "this edge did not move".
const TOL: i32 = 6;

/// Was the window moved (same size, different position)?
pub(crate) fn was_moved(before: Bounds, after: Bounds) -> bool {
    let (w0, h0) = (before.2 - before.0, before.3 - before.1);
    let (w1, h1) = (after.2 - after.0, after.3 - after.1);
    (w1 - w0).abs() <= TOL
        && (h1 - h0).abs() <= TOL
        && ((after.0 - before.0).abs() > TOL || (after.1 - before.1).abs() > TOL)
}

/// If a window in column `span` is dragged at the edge adjacent to the divider
/// (right edge for the left column, left edge for the right column), while the other
/// edge and height remain unchanged: how many pixels did the divider move (> 0 right)?
/// Otherwise, None.
pub(crate) fn divider_shift(span: Span, before: Bounds, after: Bounds) -> Option<i32> {
    let (l0, t0, r0, b0) = before;
    let (l1, t1, r1, b1) = after;
    if (t1 - t0).abs() > TOL || (b1 - b0).abs() > TOL {
        return None;
    }
    match span {
        Span::Left if (l1 - l0).abs() <= TOL && (r1 - r0).abs() > TOL => Some(r1 - r0),
        Span::Right if (r1 - r0).abs() <= TOL && (l1 - l0).abs() > TOL => Some(l1 - l0),
        _ => None,
    }
}

/// Window (other than `skip`) containing point (x, y).
pub(crate) fn window_at(
    x: i32,
    y: i32,
    skip: WindowId,
    rects: &[(WindowId, Bounds)],
) -> Option<WindowId> {
    rects
        .iter()
        .find(|(id, (l, t, r, b))| *id != skip && x >= *l && x < *r && y >= *t && y < *b)
        .map(|(id, _)| *id)
}

impl App {
    /// A drag / resize of a managed window has ended (mouse button released).
    pub(crate) unsafe fn on_drag_end(&mut self, hwnd: HWND) {
        if !self.cfg.mouse_drag
            || self.overview.is_some()
            || tray::PAUSED.load(Ordering::Relaxed)
        {
            return;
        }
        let id = wid(hwnd);
        let active = self.ws.active();
        if self.ws.workspace_of(id) != Some(active)
            || self.hidden.contains(&id)
            || self.parked.contains(&id)
        {
            return;
        }
        let Some(&before) = self.actual.get(&id) else {
            return;
        };
        let after = bounds(ext_bounds(hwnd));
        if was_moved(before, after) {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_err() {
                return;
            }
            let rects: Vec<(WindowId, Bounds)> = self
                .ws
                .window_ids(active)
                .into_iter()
                .filter(|w| !self.hidden.contains(w))
                .filter_map(|w| self.actual.get(&w).map(|&b| (w, b)))
                .collect();
            if let Some(other) = window_at(pt.x, pt.y, id, &rects) {
                if self.ws.cur_mut().swap_windows(id, other) {
                    self.ws.cur_mut().set_focus(id);
                    self.apply(); // Arrange each in its new position.
                }
            }
            return; // Not dropped on another window: sync restores it to its original position.
        }
        let span = self
            .ws
            .cur()
            .items()
            .iter()
            .find(|it| it.id == id)
            .map(|it| it.span);
        if let Some(dx) = span.and_then(|s| divider_shift(s, before, after)) {
            self.ws.cur_mut().set_focus(id);
            self.ws.cur_mut().adjust_split(dx);
            self.apply(); // New divider; if rejected (limit), the window returns to its column.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B0: Bounds = (100, 50, 900, 650);

    #[test]
    fn moving_keeps_the_size() {
        assert!(was_moved(B0, (400, 200, 1200, 800)));
        assert!(!was_moved(B0, B0));
        assert!(!was_moved(B0, (103, 52, 903, 652))); // Small movement = click.
        assert!(!was_moved(B0, (100, 50, 1000, 650))); // Resized.
    }

    #[test]
    fn dragging_the_inner_edge_moves_the_divider() {
        // Left column: right edge 900 -> 1000 = divider moved 100 px right.
        assert_eq!(divider_shift(Span::Left, B0, (100, 50, 1000, 650)), Some(100));
        // Right column: left edge 100 -> 40 = divider moved 60 px left.
        assert_eq!(divider_shift(Span::Right, B0, (40, 50, 900, 650)), Some(-60));
        // Dragged outer edge: not the divider.
        assert_eq!(divider_shift(Span::Left, B0, (40, 50, 900, 650)), None);
        assert_eq!(divider_shift(Span::Right, B0, (100, 50, 1000, 650)), None);
        // Height also changed (e.g. Windows snap): not the divider.
        assert_eq!(divider_shift(Span::Left, B0, (100, 50, 1000, 900)), None);
        // Full-width window has no divider.
        assert_eq!(divider_shift(Span::Full, B0, (100, 50, 1000, 650)), None);
        // Ignore small jitter.
        assert_eq!(divider_shift(Span::Left, B0, (100, 50, 903, 650)), None);
    }

    #[test]
    fn finds_the_window_under_the_cursor_except_the_dragged_one() {
        let a = WindowId(1);
        let b = WindowId(2);
        let rects = [(a, (0, 0, 500, 500)), (b, (500, 0, 1000, 500))];
        assert_eq!(window_at(700, 100, a, &rects), Some(b));
        assert_eq!(window_at(100, 100, a, &rects), None); // Only the window itself.
        assert_eq!(window_at(100, 100, b, &rects), Some(a));
        assert_eq!(window_at(1200, 100, a, &rects), None);
        assert_eq!(window_at(500, 0, a, &rects), Some(b)); // Left edge is included.
    }
}
