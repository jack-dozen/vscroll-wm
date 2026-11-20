//! Workspaces: switch, send windows, and park windows in other workspaces.
//!
//! Inactive-workspace windows are parked at x = PARK_X (size is preserved, so
//! reopening them only changes their position) and their taskbar buttons are hidden
//! (see taskbar.rs). Windows remain "visible" to EnumWindows, so sync() does not
//! treat them as minimized.
//!
//! Workspace switches / sends use a horizontal slide animation (`Slide`): outgoing
//! windows slide off-screen and are parked only when the animation finishes
//! (`leaving`, see tick_anim); incoming windows start off-screen on the opposite
//! side and slide into place. When animations are disabled, they move immediately.

use std::sync::atomic::Ordering;
use std::time::Instant;

use layout::{Rect, WindowId};
use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows::Win32::UI::WindowsAndMessaging::{
    GetShellWindow, SetForegroundWindow, SetWindowPos, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER,
};
use windows::core::HRESULT;

use super::App;
use crate::consts::PARK_X;
use crate::winutil::{bounds, ext_bounds, insets, is_hung, wid, work_area};
use crate::{osd, safety, taskbar, tray};

/// Workspace-switch slide direction: `enter_from` = starting side for incoming
/// windows (+1 right, -1 left), `exit_to` = destination side for outgoing windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slide {
    pub(crate) enter_from: i32,
    pub(crate) exit_to: i32,
}

impl Slide {
    /// Switch workspace in direction `dir`: screen contents shift oppositely (right = contents move left).
    pub(crate) fn switching(dir: i32) -> Self {
        Slide {
            enter_from: dir,
            exit_to: -dir,
        }
    }

    /// Send a window to the workspace in direction `dir`: the window slides toward that side.
    pub(crate) fn sending(dir: i32) -> Self {
        Slide {
            enter_from: -dir,
            exit_to: dir,
        }
    }
}

/// Direction from workspace `old` to `new` (after renumbering): -1 left, +1 right.
/// The same number means the old workspace was empty and removed, so we always move right.
pub(crate) fn slide_dir(old: usize, new: usize) -> i32 {
    if new < old {
        -1
    } else {
        1
    }
}

impl App {
    /// Switch to workspace number `n` (0 = first).
    pub(crate) unsafe fn switch_workspace(&mut self, n: usize) {
        let old = self.ws.active();
        if self.ws.switch_to(n) {
            self.slide = Some(Slide::switching(slide_dir(old, self.ws.active())));
            self.after_workspace_change();
        }
    }

    /// Previous / next workspace (does not wrap).
    pub(crate) unsafe fn switch_workspace_rel(&mut self, delta: i32) {
        let old = self.ws.active();
        if self.ws.switch_rel(delta) {
            self.slide = Some(Slide::switching(slide_dir(old, self.ws.active())));
            self.after_workspace_change();
        }
    }

    /// Send the focused window to workspace `n`; stay in this workspace.
    pub(crate) unsafe fn send_to_workspace(&mut self, n: usize) {
        let dir = slide_dir(self.ws.active(), n);
        if self.ws.move_focused_to(n).is_some() {
            self.slide = Some(Slide::sending(dir));
            self.after_workspace_change();
        }
    }

    pub(crate) unsafe fn send_to_workspace_rel(&mut self, delta: i32) {
        if self.ws.move_focused_rel(delta).is_some() {
            self.slide = Some(Slide::sending(delta.signum()));
            self.after_workspace_change();
        }
    }

    unsafe fn after_workspace_change(&mut self) {
        self.apply(); // Unpark the active workspace and park the others.
        self.focus_current();
        self.update_workspace_tip();
    }

    /// Focus the focused window in the active workspace; if the workspace is empty,
    /// focus the desktop so the foreground is not left on a newly parked window.
    unsafe fn focus_current(&mut self) {
        let target = self
            .ws
            .cur()
            .focused()
            .and_then(|f| self.map.get(&f).copied());
        // GUESS: SetForegroundWindow(GetShellWindow()) means "no focus".
        let h = target.unwrap_or_else(|| GetShellWindow());
        let _ = SetForegroundWindow(h);
        self.last_fg = Some(wid(h));
    }

    /// Tray tooltip shows the active workspace; if `workspace_osd` is enabled, a small
    /// panel appears briefly on each workspace change (osd.rs ignores repeated calls).
    pub(crate) unsafe fn update_workspace_tip(&self) {
        if tray::PAUSED.load(Ordering::Relaxed) {
            return;
        }
        let active = self.ws.active();
        tray::set_tip(&format!("vscroll-wm - workspace {}", active + 1));
        if self.cfg.workspace_osd {
            let total = self
                .ws
                .overviews()
                .iter()
                .map(|(n, _)| n + 1)
                .max()
                .unwrap_or(0)
                .max(active + 1);
            osd::notify(
                active,
                total,
                self.cfg.overview_colors,
                work_area(),
                self.cfg.workspace_osd_dots,
            );
        }
    }

    /// At startup: restore taskbar buttons that may have been left removed by a
    /// previous process that was force-killed.
    pub(crate) unsafe fn restore_all_tabs(&self) {
        for &h in self.map.values() {
            taskbar::restore(h);
        }
    }

    /// Park all windows in workspaces other than the active one. Idempotent. With
    /// `slide`, currently visible windows slide out first.
    pub(crate) unsafe fn park_inactive(&mut self, slide: Option<Slide>, work: Rect, now: Instant) {
        let active = self.ws.active();
        let ids: Vec<WindowId> = (0..self.ws.count())
            .filter(|&n| n != active)
            .flat_map(|n| self.ws.window_ids(n))
            .collect();
        for id in ids {
            self.park(id, slide, work, now);
        }
    }

    unsafe fn park(&mut self, id: WindowId, slide: Option<Slide>, work: Rect, now: Instant) {
        let Some(&h) = self.map.get(&id) else {
            return;
        };
        if !self.parked.insert(id) {
            return; // Already parked, or still sliding out (leaving).
        }
        taskbar::hide(h);
        if self.hidden.contains(&id) {
            self.anim.remove(&id);
            return; // Minimized / cloaked: hide only the taskbar button.
        }
        if let Some(s) = slide {
            // Slide out first; actually park it when the animation finishes.
            if self.slide_out(id, h, s.exit_to, work, now) {
                self.leaving.insert(id);
                return;
            }
        }
        self.anim.remove(&id);
        self.finish_park(id);
    }

    /// Move a window to its parked position (off-screen, size unchanged).
    pub(crate) unsafe fn finish_park(&mut self, id: WindowId) {
        let Some(&h) = self.map.get(&id) else {
            return;
        };
        if !safety::alive(h) || is_hung(h) {
            return;
        }
        let cur = ext_bounds(h);
        let inset = insets(h);
        let res = SetWindowPos(
            h,
            None,
            PARK_X - inset.0,
            cur.top - inset.1,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
        if res.is_err_and(|e| e.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)) {
            self.mark_blocked(id, h); // Higher privileges: cannot park it.
            return;
        }
        let actual = ext_bounds(h);
        // Record the actual position (USER32 may clamp it), to avoid "drift".
        self.actual.insert(id, bounds(actual));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_direction_follows_workspace_order() {
        assert_eq!(slide_dir(0, 1), 1);
        assert_eq!(slide_dir(3, 1), -1);
        // Old workspace (empty, in the middle) removed: its new number may be the same -> move right.
        assert_eq!(slide_dir(1, 1), 1);
    }

    #[test]
    fn switching_moves_content_against_travel_and_sending_goes_toward_the_target() {
        assert_eq!(
            Slide::switching(1),
            Slide {
                enter_from: 1,
                exit_to: -1
            }
        );
        assert_eq!(
            Slide::switching(-1),
            Slide {
                enter_from: -1,
                exit_to: 1
            }
        );
        assert_eq!(Slide::sending(1).exit_to, 1);
        assert_eq!(Slide::sending(-1).exit_to, -1);
    }
}
