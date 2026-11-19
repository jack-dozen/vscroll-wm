//! Lift-and-drop (`pick`) mode directly on the desktop with an overlay marker.

use std::sync::atomic::Ordering;

use layout::{Dir, Rect, WindowId};
use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

use super::workspace::{slide_dir, Slide};
use super::App;
use crate::consts::DIRS;
use crate::hook;
use crate::pick_overlay::{self, PickSlot, PickTarget};
use crate::winutil::work_area;

pub(crate) struct PickSession {
    pub(crate) src: WindowId,
    /// App icon for `src` (HICON as isize; 0 = none), captured once when picked up.
    pub(crate) icon: isize,
    pub(crate) slots: Vec<PickSlot>,
    pub(crate) idx: usize,
}

impl App {
    /// Enter / exit lift-and-drop mode for the focused window.
    pub(crate) unsafe fn toggle_pick(&mut self) {
        if self.pick.is_some() {
            self.pick_cancel();
            return;
        }
        self.sync();
        let Some(src) = self.ws.cur().focused() else {
            return;
        };
        let icon = self
            .map
            .get(&src)
            .copied()
            .map_or(0, |h| pick_overlay::window_icon(h));
        let ov = self.ws.cur().overview();
        let src_rect = ov.items.iter().find(|i| i.id == src).map(|i| i.rect);
        let slots = pick_overlay::build_slots(&ov, src);
        let idx = pick_overlay::initial_slot(&slots, src_rect);
        self.pick = Some(PickSession {
            src,
            icon,
            slots,
            idx,
        });
        hook::PICK_OPEN.store(true, Ordering::Relaxed);
        self.pick_show_current();
    }

    /// Scroll the viewport to the selected slot and show the overlay marker there.
    unsafe fn pick_show_current(&mut self) {
        let Some((slot, icon)) = self
            .pick
            .as_ref()
            .and_then(|s| s.slots.get(s.idx).copied().map(|sl| (sl, s.icon)))
        else {
            return;
        };
        if let Some(anchor) = slot.anchor {
            self.ws.cur_mut().set_focus(anchor);
            self.apply();
        }
        let work = work_area();
        let ov = self.ws.cur().overview();
        let g = self.cfg.gap / 2;
        let x0 = (work.x + g + slot.rect.x).clamp(work.x, work.x + work.w - 1);
        let y0 = (work.y + g + slot.rect.y - ov.view_y).clamp(work.y, work.y + work.h - 1);
        let x1 = (work.x + g + slot.rect.x + slot.rect.w).clamp(x0 + 1, work.x + work.w);
        let y1 = (work.y + g + slot.rect.y + slot.rect.h - ov.view_y)
            .clamp(y0 + 1, work.y + work.h);
        let screen = Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        };
        let color = self
            .cfg
            .join_border
            .or(self.focus_border)
            .unwrap_or(self.cfg.overview_colors.active);
        pick_overlay::show(screen, color, icon);
    }

    /// Move the overlay marker in the arrow direction (`dir_idx` = index in `DIRS`).
    /// At the strip's left / right edge, cross to the neighboring workspace (including an empty one).
    pub(crate) unsafe fn pick_move(&mut self, dir_idx: usize) {
        let Some(&(_, dir)) = DIRS.get(dir_idx) else {
            return;
        };
        let Some(sess) = self.pick.as_mut() else {
            return;
        };
        if let Some(next) = pick_overlay::step_slot(&sess.slots, sess.idx, dir) {
            sess.idx = next;
            self.pick_show_current();
            return;
        }
        if !matches!(dir, Dir::Left | Dir::Right) {
            return;
        }
        let forward = dir == Dir::Right;
        let cur_ws = self.ws.active();
        let next_ws = if forward {
            (cur_ws + 1 < self.ws.count()).then_some(cur_ws + 1)
        } else {
            cur_ws.checked_sub(1)
        };
        let Some(next_ws) = next_ws else { return };
        let cur_y = sess
            .slots
            .get(sess.idx)
            .map_or(0, |s| s.rect.y + s.rect.h / 2);
        let src = sess.src;
        let icon = sess.icon;
        if !self.ws.switch_to(next_ws) {
            return;
        }
        self.slide = Some(Slide::switching(slide_dir(cur_ws, self.ws.active())));
        let ov = self.ws.cur().overview();
        let slots = pick_overlay::build_slots(&ov, src);
        let idx = pick_overlay::entry_slot(&slots, forward, cur_y);
        self.pick = Some(PickSession {
            src,
            icon,
            slots,
            idx,
        });
        self.apply();
        self.update_workspace_tip();
        self.pick_show_current();
    }

    /// Drop the window at the selected marker (`Enter` / `Space`).
    pub(crate) unsafe fn pick_accept(&mut self) {
        let Some(sess) = self.pick.take() else {
            return;
        };
        hook::PICK_OPEN.store(false, Ordering::Relaxed);
        pick_overlay::hide();
        let Some(&slot) = sess.slots.get(sess.idx) else {
            return;
        };
        let src = sess.src;
        let src_ws = self.ws.workspace_of(src);
        let target_ws = self.ws.active();
        let cross_ws = src_ws.is_some_and(|w| w != target_ws);
        let old_rect = if !cross_ws {
            self.actual.get(&src).copied()
        } else {
            None
        };

        let changed = match slot.target {
            PickTarget::EmptyWorkspace => {
                if cross_ws {
                    self.ws.move_window_to(src, target_ws)
                } else {
                    false
                }
            }
            other => match other.to_insert_position() {
                Some(at) if cross_ws => self.ws.move_window_at(src, target_ws, at),
                Some(at) => self.ws.reposition_window(src, at),
                None => false,
            },
        };
        if !changed {
            self.join_restore_src(src);
            return;
        }
        if cross_ws {
            // Ensure the destination workspace remains active after normalize().
            let _ = self.ws.focus_window(src);
        }
        self.apply();
        self.animate_join_slide(src, cross_ws, src_ws < Some(target_ws), old_rect);
        if cross_ws {
            if let Some(&h) = self.map.get(&src) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(src);
            }
            self.update_workspace_tip();
        }
    }

    /// Cancel lift-and-drop mode (`Esc` or press the pick key again).
    pub(crate) unsafe fn pick_cancel(&mut self) {
        let Some(sess) = self.pick.take() else {
            return;
        };
        hook::PICK_OPEN.store(false, Ordering::Relaxed);
        pick_overlay::hide();
        self.join_restore_src(sess.src);
    }
}
