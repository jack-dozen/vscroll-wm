//! Handle hotkeys, Windows events, and the debounce timer.

use std::sync::atomic::Ordering;
use std::time::Instant;

use windows::Win32::Foundation::HWND;

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use super::workspace::{slide_dir, Slide};
use super::{App, JoinAim};
use crate::consts::{
    DIRS, EVENT_DEBOUNCE_MS, HOTKEY_CHEATSHEET, HOTKEY_CLOSE, HOTKEY_FLOAT, HOTKEY_INSPECT,
    HOTKEY_JOIN_DOWN, HOTKEY_JOIN_LEFT, HOTKEY_MAXIMIZE, HOTKEY_MINIMIZE, HOTKEY_OVERVIEW,
    HOTKEY_PICK, HOTKEY_SPLIT_LEFT, HOTKEY_SPLIT_RESET, HOTKEY_SPLIT_RIGHT, HOTKEY_STICKY,
    HOTKEY_WS_GO, HOTKEY_WS_NEXT, HOTKEY_WS_NOOP, HOTKEY_WS_PREV, HOTKEY_WS_SEND,
    HOTKEY_WS_SEND_NEXT, HOTKEY_WS_SEND_PREV,
};
use crate::winutil::{manageable, wid, work_area};
use crate::{cheatsheet, hook, tray};
use crate::config::HeightMode;
use layout::{Dir, Rect, WindowId};

impl App {
    /// Schedule sync + apply in `ms` milliseconds (coalesce if already scheduled).
    pub(crate) unsafe fn schedule_sync(&mut self, ms: u32) {
        if self.sync_timer == 0 {
            self.sync_timer = SetTimer(None, 0, ms, None);
        }
    }

    pub(crate) unsafe fn stop_sync_timer(&mut self) {
        if self.sync_timer != 0 {
            let _ = KillTimer(None, self.sync_timer);
            self.sync_timer = 0;
        }
    }

    /// Windows events: only those involving windows we manage (or new
    /// candidates) trigger rearrangement.
    pub(crate) unsafe fn on_event(&mut self, event: u32, hwnd: HWND) {
        let known = self.map.contains_key(&wid(hwnd));
        let relevant = match event {
            EVENT_SYSTEM_FOREGROUND => true,
            EVENT_SYSTEM_MOVESIZEEND
            | EVENT_SYSTEM_MINIMIZESTART
            | EVENT_OBJECT_DESTROY
            | EVENT_OBJECT_HIDE
            | EVENT_OBJECT_CLOAKED => known,
            EVENT_SYSTEM_MINIMIZEEND | EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => {
                known || manageable(hwnd)
            }
            _ => false,
        };
        if event == EVENT_SYSTEM_MOVESIZESTART && known {
            // Drag started: track the cursor for the snap layouts menu (snap.rs).
            self.snap_drag_start(hwnd);
        }
        if event == EVENT_SYSTEM_MOVESIZEEND && known {
            // Window drag / edge resize ended: apply a snap layout preset if released
            // over a menu zone (snap.rs); otherwise swap positions or move the divider (mouse.rs).
            if !self.snap_drag_end(hwnd) {
                self.on_drag_end(hwnd);
            }
        }
        if relevant {
            self.schedule_sync(EVENT_DEBOUNCE_MS);
        }
    }

    pub(crate) unsafe fn on_hotkey(&mut self, id: i32) {
        if tray::PAUSED.load(Ordering::Relaxed) {
            return;
        }
        if self.snap_assist.is_some() {
            self.snap_assist_end(true); // Any shortcut closes the snap layouts window list.
        }
        if id == HOTKEY_PICK {
            self.toggle_pick();
            return;
        }
        if self.pick.is_some() {
            self.pick_cancel();
        }
        if (HOTKEY_JOIN_LEFT..=HOTKEY_JOIN_DOWN).contains(&id) {
            // Quick join goes straight to the neighbor; switch to `join_step` (and enable
            // JOIN_ACTIVE in hook.rs) to return to modifier-hold mode.
            self.join_quick(DIRS[(id - HOTKEY_JOIN_LEFT) as usize].1);
            return;
        }
        if let Some(aim) = self.join_aim.take() {
            hook::JOIN_ACTIVE.store(false, Ordering::Relaxed);
            if let Some(src_ws) = self.ws.workspace_of(aim.src) {
                if self.ws.active() != src_ws {
                    self.ws.switch_to(src_ws);
                }
            }
            self.ws.cur_mut().set_focus(aim.src);
        }
        self.sync();
        match id {
            // Step mode: Win+↑/↓ adjusts height incrementally instead of jumping between presets.
            3 | 4 if self.cfg.height_mode == HeightMode::Step => {
                let step = self.cfg.height_step;
                self.ws.cur_mut().adjust_height(if id == 3 { -step } else { step });
            }
            1..=4 => self.ws.cur_mut().snap(DIRS[(id - 1) as usize].1),
            5..=8 => self.ws.cur_mut().focus(DIRS[(id - 5) as usize].1),
            HOTKEY_MAXIMIZE => self.ws.cur_mut().maximize(),
            HOTKEY_INSPECT => {
                self.inspect();
                return;
            }
            HOTKEY_OVERVIEW => {
                self.toggle_overview();
                return;
            }
            // The column divider moved; apply() below rearranges the windows.
            HOTKEY_SPLIT_RIGHT => {
                self.ws.cur_mut().adjust_split(self.cfg.resize_step);
            }
            HOTKEY_SPLIT_LEFT => {
                self.ws.cur_mut().adjust_split(-self.cfg.resize_step);
            }
            HOTKEY_SPLIT_RESET => {
                self.ws.cur_mut().reset_split();
            }
            HOTKEY_CHEATSHEET => {
                cheatsheet::toggle(work_area(), &self.cfg);
                return;
            }
            HOTKEY_CLOSE => {
                self.close_active();
                return;
            }
            HOTKEY_MINIMIZE => {
                self.minimize_active();
                return;
            }
            HOTKEY_FLOAT => {
                self.toggle_float();
                self.sync(); // Add the newly tiled window to the strip.
            }
            HOTKEY_STICKY => {
                self.toggle_sticky();
                self.sync();
            }
            10..=13 => self.ws.cur_mut().move_window(DIRS[(id - 10) as usize].1),
            HOTKEY_WS_PREV => {
                self.switch_workspace_rel(-1);
                return;
            }
            HOTKEY_WS_NEXT => {
                self.switch_workspace_rel(1);
                return;
            }
            HOTKEY_WS_SEND_PREV => {
                self.send_to_workspace_rel(-1);
                return;
            }
            HOTKEY_WS_SEND_NEXT => {
                self.send_to_workspace_rel(1);
                return;
            }
            n if (HOTKEY_WS_GO..HOTKEY_WS_GO + 9).contains(&n) => {
                self.switch_workspace((n - HOTKEY_WS_GO) as usize);
                return;
            }
            n if (HOTKEY_WS_SEND..HOTKEY_WS_SEND + 9).contains(&n) => {
                self.send_to_workspace((n - HOTKEY_WS_SEND) as usize);
                return;
            }
            HOTKEY_WS_NOOP => return,
            _ => return,
        }
        self.apply();
        if (5..=8).contains(&id) {
            if let Some(h) = self.ws.cur().focused().and_then(|f| self.map.get(&f).copied()) {
                let prev = self.last_fg;
                let _ = SetForegroundWindow(h);
                let next = wid(h);
                self.last_fg = Some(next);
                if let Some(p) = prev {
                    self.auto_minimize_float(p, next);
                }
            }
        }
    }

    /// Quick join (`Win+Alt+Shift+Arrow`): immediately join the focused window
    /// with its neighbor in direction `dir`, using a slide animation.
    unsafe fn join_quick(&mut self, dir: Dir) {
        self.sync();
        let Some(src) = self.ws.cur().focused() else {
            return;
        };
        let old_rect = self.actual.get(&src).copied();
        if !self.ws.cur_mut().join(dir) {
            return;
        }
        self.apply();
        self.animate_join_slide(src, false, false, old_rect);
    }

    /// Slide animation for window `src` after it has just been joined / dropped.
    pub(super) unsafe fn animate_join_slide(
        &mut self,
        src: WindowId,
        cross_ws: bool,
        from_left: bool,
        old_rect: Option<(i32, i32, i32, i32)>,
    ) {
        if self.anim_ms <= 0.0 {
            return;
        }
        let (Some(&hwnd), Some(&(l, t, r, b))) = (self.map.get(&src), self.actual.get(&src)) else {
            return;
        };
        let work = work_area();
        let new_rect = Rect {
            x: l,
            y: t,
            w: r - l,
            h: b - t,
        };
        let offset = if cross_ws {
            (if from_left { -work.w / 3 } else { work.w / 3 }, 0)
        } else if let Some((ol, ot, or, ob)) = old_rect {
            let dx = ((ol + or) / 2 - (l + r) / 2).clamp(-work.w / 3, work.w / 3);
            let dy = ((ot + ob) / 2 - (t + b) / 2).clamp(-work.h / 3, work.h / 3);
            (dx, dy)
        } else {
            (0, 0)
        };
        if offset != (0, 0) {
            let now = Instant::now();
            self.slide_from(src, hwnd, new_rect, offset, work, now);
            if !self.anim.is_empty() && self.anim_timer == 0 {
                self.anim_timer = SetTimer(None, 0, 10, None);
            }
        }
    }

    /// Find the next target from `from` in direction `dir`. At the left/right
    /// edge of the strip, cross to the neighboring workspace with visible windows.
    /// Returns `(target, new_workspace_if_switched)`.
    #[allow(dead_code)]
    fn join_next_target(&self, from: WindowId, dir: Dir) -> Option<(WindowId, Option<usize>)> {
        if let Some(next) = self.ws.cur().neighbor_of(from, dir) {
            return Some((next, None));
        }
        if !matches!(dir, Dir::Left | Dir::Right) {
            return None;
        }
        let forward = dir == Dir::Right;
        let next_ws = self.ws.adjacent_workspace(self.ws.active(), forward)?;
        let y_mid = self
            .ws
            .cur()
            .overview()
            .items
            .iter()
            .find(|i| i.id == from)
            .map(|i| i.rect.y + i.rect.h / 2)
            .unwrap_or(0);
        let next = self
            .ws
            .entry_window(next_ws, forward, y_mid)
            .or_else(|| self.ws.representative(next_ws))?;
        Some((next, Some(next_ws)))
    }

    /// One `join` targeting step: start at the focused window (first time) or move
    /// the target to a neighbor in direction `dir` (Left/Right can cross to the
    /// neighboring workspace). Scroll the viewport to the target and highlight its
    /// border until the modifier is released (`join_commit`) or canceled (`join_cancel`).
    #[allow(dead_code)]
    unsafe fn join_step(&mut self, dir: Dir) {
        let (src, from) = match self.join_aim {
            Some(aim) => (aim.src, aim.target),
            None => {
                self.sync();
                let Some(src) = self.ws.cur().focused() else {
                    return;
                };
                (src, src)
            }
        };
        let Some((target, cross_ws)) = self.join_next_target(from, dir) else {
            return;
        };
        if let Some(next_ws) = cross_ws {
            let old = self.ws.active();
            if self.ws.switch_to(next_ws) {
                self.slide = Some(Slide::switching(slide_dir(old, self.ws.active())));
            }
        }
        self.join_aim = Some(JoinAim { src, target });
        self.ws.cur_mut().set_focus(target);
        self.apply();
        if cross_ws.is_some() {
            self.update_workspace_tip();
        }
    }

    /// Restore the viewport and workspace to source window `src` (on cancel / failure).
    pub(super) unsafe fn join_restore_src(&mut self, src: WindowId) {
        let mut switched = false;
        if let Some(src_ws) = self.ws.workspace_of(src) {
            if self.ws.active() != src_ws {
                let old = self.ws.active();
                if self.ws.switch_to(src_ws) {
                    self.slide = Some(Slide::switching(slide_dir(old, self.ws.active())));
                    switched = true;
                }
            }
        }
        self.ws.cur_mut().set_focus(src);
        self.apply();
        if switched {
            self.update_workspace_tip();
        }
    }

    /// Modifier released: join the source window to the last target (including across
    /// workspaces), sliding it to its new position.
    pub(crate) unsafe fn join_commit(&mut self) {
        let Some(aim) = self.join_aim.take() else {
            return;
        };
        let src_ws = self.ws.workspace_of(aim.src);
        let target_ws = self.ws.workspace_of(aim.target);
        let cross_ws = src_ws.is_some() && target_ws.is_some() && src_ws != target_ws;
        let old_rect = if !cross_ws {
            self.actual.get(&aim.src).copied()
        } else {
            None
        };
        if !self.ws.join_into(aim.src, aim.target) {
            self.join_restore_src(aim.src);
            return;
        }
        self.apply();
        self.animate_join_slide(aim.src, cross_ws, src_ws < target_ws, old_rect);
        if cross_ws {
            if let Some(&h) = self.map.get(&aim.src) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(aim.src);
            }
            self.update_workspace_tip();
        }
    }

    /// Esc pressed while targeting: cancel without changing the layout.
    pub(crate) unsafe fn join_cancel(&mut self) {
        let Some(aim) = self.join_aim.take() else {
            return;
        };
        self.join_restore_src(aim.src);
    }
}
