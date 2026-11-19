//! Snap layouts: drag a window to the top edge of the screen to show the preset menu;
//! release over a zone to place the window there. Other zones in the same preset show
//! a list of open windows (Snap Assist); click a window to place it in a zone.
//!
//! Flow:
//!  1. EVENT_SYSTEM_MOVESIZESTART -> `snap_drag_start`: start tracking the cursor
//!     with a timer (`tick_snap_drag`). Edge drags (resize) are detected by the
//!     size change and ignored.
//!  2. Cursor enters the trigger band at the top edge -> show the menu (snap_menu.rs);
//!     highlight the zone under the cursor. Hide the menu again when the cursor moves away.
//!  3. EVENT_SYSTEM_MOVESIZEEND -> `snap_drag_end`: if the menu is visible and the
//!     cursor is over a zone, apply the preset (`Workspaces::apply_snap`); otherwise
//!     process the drag as usual (swap positions / move divider, mouse.rs).
//!  4. Empty zones become slots in the strip. If `snap_assist` is enabled, show a
//!     window list (snap_slots.rs) above each slot. Selecting a window calls
//!     `snap_assist_pick`. The session ends when all slots are filled, or is canceled
//!     (Esc, close button, outside click, keyboard shortcut, overview, new drag);
//!     remaining slots are removed so those zones stay empty.

use std::sync::atomic::Ordering;

use layout::{Rect, WindowId};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use super::App;
use crate::consts::{EVENT_DEBOUNCE_MS, SNAP_TICK_MS};
use crate::hook::SNAP_OPEN;
use crate::winutil::{bounds, ext_bounds, title, wid, work_area, Bounds};
use crate::{config, rules, safety, snap_menu, snap_slots, tray};

/// Distance (pixels) from the menu panel still considered "inside" so it does not
/// disappear when the cursor strays slightly outside.
const MENU_MARGIN: i32 = 24;
/// Minimum movement (pixels) before a drag is considered to move a window, and
/// maximum size difference before it is treated as an edge drag (resize).
const MOVE_TOL: i32 = 6;

/// Window drag currently tracked for the snap layouts menu.
pub(crate) struct SnapDrag {
    id: WindowId,
    hwnd: HWND,
    /// Frame bounds when the drag started.
    start: Bounds,
    /// Menu layout, if currently visible.
    menu: Option<snap_menu::MenuLayout>,
    /// Zone (preset, zone) under the cursor.
    hover: Option<(usize, usize)>,
    /// Esc pressed during the drag: drag was canceled; do not apply the preset.
    cancelled: bool,
}

/// Window-list session for empty zones.
pub(crate) struct SnapAssist {
    /// Windows already occupying zones in this preset (for focus tracking).
    members: Vec<WindowId>,
    /// Unfilled slots.
    slots: Vec<WindowId>,
}

impl App {
    /// A window drag started.
    pub(crate) unsafe fn snap_drag_start(&mut self, hwnd: HWND) {
        if !self.cfg.snap_layouts_enabled
            || self.cfg.snap_layouts.is_empty()
            || self.overview.is_some()
            || tray::PAUSED.load(Ordering::Relaxed)
        {
            return;
        }
        let id = wid(hwnd);
        if self.ws.workspace_of(id) != Some(self.ws.active())
            || self.hidden.contains(&id)
            || self.parked.contains(&id)
        {
            return;
        }
        // A new drag cancels the previous session; do not call apply(): the
        // dragged window must not be pulled back to its original position.
        self.snap_reset(false);
        self.snap_drag = Some(SnapDrag {
            id,
            hwnd,
            start: bounds(ext_bounds(hwnd)),
            menu: None,
            hover: None,
            cancelled: false,
        });
        if self.snap_timer == 0 {
            self.snap_timer = SetTimer(None, 0, SNAP_TICK_MS, None);
        }
    }

    /// Stop tracking the drag: hide the menu and stop the timer.
    unsafe fn snap_stop_drag(&mut self) {
        self.snap_drag = None;
        snap_menu::hide();
        if self.snap_timer != 0 {
            let _ = KillTimer(None, self.snap_timer);
            self.snap_timer = 0;
        }
    }

    /// Stop the drag and window-list session (pause, config reload, new drag).
    pub(crate) unsafe fn snap_reset(&mut self, apply: bool) {
        self.snap_stop_drag();
        if self.snap_assist.is_some() {
            self.snap_assist_end(apply);
        }
    }

    unsafe fn snap_show_menu(&mut self, work: Rect) {
        let scale = GetDpiForSystem() as f32 / 96.0;
        let menu = snap_menu::layout_menu(work, &self.cfg.snap_layouts, scale);
        let names: Vec<String> = self
            .cfg
            .snap_layouts
            .iter()
            .map(|p| p.name.clone())
            .collect();
        snap_menu::show(menu.clone(), names, self.cfg.overview_colors);
        if let Some(d) = self.snap_drag.as_mut() {
            d.menu = Some(menu);
            d.hover = None;
        }
    }

    unsafe fn snap_hide_menu(&mut self) {
        snap_menu::hide();
        if let Some(d) = self.snap_drag.as_mut() {
            d.menu = None;
            d.hover = None;
        }
    }

    /// Called by the timer during a drag: show / hide the menu and highlight the zone.
    pub(crate) unsafe fn tick_snap_drag(&mut self) {
        let Some(d) = self.snap_drag.as_ref() else {
            self.snap_stop_drag();
            return;
        };
        let (hwnd, start, cancelled) = (d.hwnd, d.start, d.cancelled);
        if tray::PAUSED.load(Ordering::Relaxed)
            || self.overview.is_some()
            || !safety::alive(hwnd)
        {
            self.snap_stop_drag();
            return;
        }
        if cancelled {
            return;
        }
        if GetAsyncKeyState(VK_ESCAPE.0 as i32) < 0 {
            // Esc cancels the drag in Windows: do not apply a preset on release.
            self.snap_hide_menu();
            if let Some(d) = self.snap_drag.as_mut() {
                d.cancelled = true;
            }
            return;
        }
        let now = bounds(ext_bounds(hwnd));
        let size_changed = ((now.2 - now.0) - (start.2 - start.0)).abs() > MOVE_TOL
            || ((now.3 - now.1) - (start.3 - start.1)).abs() > MOVE_TOL;
        if size_changed {
            // A window edge was dragged (resize), not moved: this menu does not handle it.
            self.snap_stop_drag();
            return;
        }
        if GetAsyncKeyState(VK_LBUTTON.0 as i32) >= 0 {
            return; // Left mouse button is not pressed (e.g. keyboard navigation).
        }
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return;
        }
        let work = work_area();
        let in_band = pt.y < work.y + self.cfg.snap_trigger
            && pt.x >= work.x
            && pt.x < work.x + work.w;
        let moved = (now.0 - start.0).abs() > MOVE_TOL || (now.1 - start.1).abs() > MOVE_TOL;

        let shown = self
            .snap_drag
            .as_ref()
            .map_or(false, |d| d.menu.is_some());
        if !shown {
            if moved && in_band {
                self.snap_show_menu(work);
            }
            return;
        }
        // Menu is visible: keep it open while the cursor is over the menu or trigger band.
        let (hover, keep) = {
            let Some(m) = self.snap_drag.as_ref().and_then(|d| d.menu.as_ref()) else {
                return;
            };
            let hover = snap_menu::hit(m, pt.x, pt.y);
            let keep = hover.is_some()
                || snap_menu::inside(m, pt.x, pt.y, MENU_MARGIN)
                || pt.y < work.y + self.cfg.snap_trigger;
            (hover, keep)
        };
        if !keep {
            self.snap_hide_menu();
            return;
        }
        if let Some(d) = self.snap_drag.as_mut() {
            if d.hover != hover {
                d.hover = hover;
                snap_menu::set_hover(hover);
            }
        }
    }

    /// Drag ended. True if it was used to apply a preset (caller need not process it
    /// as a position swap / divider movement).
    pub(crate) unsafe fn snap_drag_end(&mut self, hwnd: HWND) -> bool {
        let Some(d) = self.snap_drag.take() else {
            return false;
        };
        self.snap_stop_drag(); // Hide the menu and stop the timer.
        if d.id != wid(hwnd) || d.cancelled {
            return false;
        }
        let Some(menu) = d.menu else {
            return false;
        };
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return false;
        }
        let Some((pi, zi)) = snap_menu::hit(&menu, pt.x, pt.y) else {
            return false;
        };
        self.snap_apply(d.id, pi, zi);
        true
    }

    /// Apply preset `pi` with `id` to zone `zi`.
    unsafe fn snap_apply(&mut self, id: WindowId, pi: usize, zi: usize) {
        let Some(preset) = self.cfg.snap_layouts.get(pi).cloned() else {
            return;
        };
        let slots = self.ws.apply_snap(id, &preset, zi);
        if !self.cfg.snap_assist {
            self.ws.clear_slots(); // Leave the other zones empty.
        }
        self.apply();
        if let Some(&h) = self.map.get(&id) {
            let _ = SetForegroundWindow(h);
            self.last_fg = Some(id);
        }
        if self.cfg.snap_assist && !slots.is_empty() {
            self.snap_assist = Some(SnapAssist {
                members: vec![id],
                slots,
            });
            if !self.snap_assist_show() {
                self.snap_assist_end(true); // No other windows to select.
            }
        }
        self.schedule_sync(EVENT_DEBOUNCE_MS);
    }

    /// Windows that can fill a slot: only open windows tiled in the active workspace
    /// (in strip order), excluding `members` and currently hidden windows.
    unsafe fn snap_candidates(&self, members: &[WindowId]) -> Vec<snap_slots::Candidate> {
        let mode = self.cfg.overview_label;
        let mut out = Vec::new();
        for id in self.ws.window_ids(self.ws.active()) {
            if id.is_slot() || members.contains(&id) || self.hidden.contains(&id) {
                continue;
            }
            let Some(&h) = self.map.get(&id) else {
                continue;
            };
            let t = title(h);
            let exe = if mode == config::LabelMode::App {
                rules::exe_name(h)
            } else {
                None
            };
            out.push(snap_slots::Candidate {
                id: id.0,
                label: config::label(mode, &t, exe.as_deref(), &self.cfg.aliases),
                source: h.0 as isize,
            });
        }
        out
    }

    /// Show / refresh window lists above empty slots. False if nothing can be shown
    /// (no visible slots or no windows).
    unsafe fn snap_assist_show(&mut self) -> bool {
        let Some(a) = self.snap_assist.as_ref() else {
            return false;
        };
        let places = self.ws.cur().layout();
        let views: Vec<snap_slots::SlotView> = a
            .slots
            .iter()
            .filter_map(|s| {
                places
                    .iter()
                    .find(|p| p.id == *s && p.visible) // Off-screen zones cannot be clicked.
                    .map(|p| snap_slots::SlotView {
                        slot: s.0,
                        rect: p.rect,
                    })
            })
            .collect();
        if views.is_empty() {
            return false;
        }
        let cands = self.snap_candidates(&a.members);
        if cands.is_empty() {
            return false;
        }
        snap_slots::show(
            &views,
            &cands,
            self.cfg.overview_colors,
            self.cfg.overview_thumbnails,
        );
        SNAP_OPEN.store(true, Ordering::Relaxed);
        true
    }

    /// A window was selected to fill a slot (WM_SNAP_PICK message).
    pub(crate) unsafe fn snap_assist_pick(&mut self, win: u64, slot: u64) {
        if self.snap_assist.is_none() {
            return;
        }
        let (id, slot_id) = (WindowId(win), WindowId(slot));
        if self.ws.fill_slot(slot_id, id) {
            if let Some(a) = self.snap_assist.as_mut() {
                a.slots.retain(|s| *s != slot_id);
                a.members.push(id);
            }
            self.apply();
            if let Some(&h) = self.map.get(&id) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(id);
            }
            self.schedule_sync(EVENT_DEBOUNCE_MS);
        }
        // Failed (window already closed): just refresh the list.
        let more = self
            .snap_assist
            .as_ref()
            .map_or(false, |a| !a.slots.is_empty());
        if !more || !self.snap_assist_show() {
            self.snap_assist_end(true);
        }
    }

    /// End the session: hide the window list and remove remaining slots (those zones
    /// stay empty). `apply` = rearrange windows after removing the slots.
    pub(crate) unsafe fn snap_assist_end(&mut self, apply: bool) {
        self.snap_assist = None;
        snap_slots::hide();
        SNAP_OPEN.store(false, Ordering::Relaxed);
        if self.ws.clear_slots() && apply {
            self.apply();
        }
    }

    /// Called by sync(): end the session if focus moves to a window outside this
    /// preset (click another window, Alt+Tab).
    pub(crate) unsafe fn snap_assist_follow_focus(&mut self, fg: HWND) {
        let Some(a) = self.snap_assist.as_ref() else {
            return;
        };
        if fg.0.is_null() || snap_slots::is_ours(fg) || a.members.contains(&wid(fg)) {
            return; // Foreground is transitioning / owned by us / part of the preset.
        }
        self.snap_assist_end(true);
    }
}
