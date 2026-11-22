//! Actions for the active window: close, minimize, float, pause, and inspect.

use std::sync::atomic::Ordering;

use layout::WindowId;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use super::App;
use crate::winutil::{ext_bounds, place, unmanaged_reason, wid, work_area};
use crate::{bindings, cheatsheet, osd, rules, safety, taskbar, tray};

/// Show config issues in a message box (release builds have no console).
/// Run on a separate thread so the message loop is not blocked. Does nothing
/// when there are no issues.
pub(crate) fn show_config_problems(report: &crate::config::LoadReport) {
    if report.problems.is_empty() {
        return;
    }
    let file = report
        .path
        .as_ref()
        .map_or_else(|| "configuration".to_string(), |p| p.display().to_string());
    let list: Vec<String> = report.problems.iter().map(|p| format!("  - {p}")).collect();
    let text = format!(
        "{} problem(s) in {file}:\n\n{}\n\nThe affected settings keep their defaults.\nFix the file, then use the tray menu: Reload configuration.",
        report.problems.len(),
        list.join("\n")
    );
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    std::thread::spawn(move || unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide.as_ptr()),
            w!("vscroll-wm: configuration problems"),
            MB_OK | MB_ICONWARNING,
        );
    });
}

impl App {
    /// Win+Q: close the active window (equivalent to Alt+F4). Only windows we
    /// recognize, so the desktop / taskbar do not receive a close command.
    pub(crate) unsafe fn close_active(&self) {
        let fg = GetForegroundWindow();
        let id = wid(fg);
        if self.map.contains_key(&id) || self.floating.contains_key(&id) {
            let _ = PostMessageW(
                Some(fg),
                WM_SYSCOMMAND,
                WPARAM(SC_CLOSE as usize),
                LPARAM(0),
            );
        }
    }

    /// Win+M: minimize only the active window. Focus moves to its neighbor in
    /// the strip, not to the window chosen by Windows.
    pub(crate) unsafe fn minimize_active(&mut self) {
        let fg = GetForegroundWindow();
        let id = wid(fg);
        let tiled = self.map.contains_key(&id);
        if !tiled && !self.floating.contains_key(&id) {
            return;
        }
        if tiled {
            self.ws.set_hidden(id, true); // Focus moves to the neighbor.
            self.hidden.insert(id);
            self.anim.remove(&id);
        }
        let _ = ShowWindow(fg, SW_MINIMIZE);
        if tiled {
            if let Some(h) = self.ws.cur().focused().and_then(|f| self.map.get(&f).copied()) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(wid(h));
            }
            self.apply();
        }
    }

    /// Pause / resume (tray menu). When paused, all windows are restored to
    /// their original positions and hotkeys are passed through to Windows.
    pub(crate) unsafe fn toggle_pause(&mut self) {
        let paused = !tray::PAUSED.load(Ordering::Relaxed);
        tray::PAUSED.store(paused, Ordering::Relaxed);
        tray::set_tip(if paused {
            "vscroll-wm (paused)"
        } else {
            "vscroll-wm"
        });
        if paused {
            self.snap_reset(false);
            self.close_overview(false);
            cheatsheet::hide();
            osd::hide();
            self.finish_all_anims();
            self.stop_sync_timer();
            self.unstick_all();
            safety::restore_all(); // Also restores border colors.
            taskbar::show_all();
            self.parked.clear();
            self.leaving.clear();
            self.border_applied = None;
        } else {
            self.sync();
            self.apply();
            self.update_workspace_tip();
        }
    }

    /// Win+Alt+I: show the active window's class, exe, and title, plus why it
    /// is or is not tiled. Useful for writing rules in vscroll-wm.ignore.
    pub(crate) unsafe fn inspect(&self) {
        let h = GetForegroundWindow();
        let id = wid(h);
        let class = rules::class_name(h);
        let title = rules::window_title(h);
        let exe = rules::exe_name(h).unwrap_or_else(|| "(unreadable)".into());
        let status = if self.floating.contains_key(&id) {
            format!("floating (toggle with {})", bindings::active_label("float"))
        } else if self.blocked.contains_key(&id) {
            "cannot be controlled: probably running as administrator \
             (run vscroll-wm as administrator too)"
                .to_string()
        } else if self.map.contains_key(&id) {
            "tiled".to_string()
        } else {
            match unmanaged_reason(h) {
                Some(why) => format!("not tiled: {why}"),
                None => "not tiled yet (waiting for sync)".to_string(),
            }
        };
        let text = format!(
            "title : {title}\nclass : {class}\nexe   : {exe}\nstatus: {status}\n\nLines for vscroll-wm.ignore:\n  class:{class}\n  exe:{exe}\n  title:{title}"
        );
        #[cfg(debug_assertions)]
        println!("---------- window aktif ----------\n{text}\n----------------------------------");
        if !cfg!(debug_assertions) {
            // Release builds have no console: show a message box on a separate
            // thread so the message loop is not blocked.
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            std::thread::spawn(move || {
                MessageBoxW(
                    None,
                    PCWSTR(wide.as_ptr()),
                    w!("vscroll-wm: window info"),
                    MB_OK | MB_ICONINFORMATION,
                );
            });
        }
    }

    /// The window refused to move because it has higher privileges than us
    /// (usually running as administrator; UIPI). Remove it from the strip so
    /// it does not occupy a slot that can never be filled.
    pub(crate) unsafe fn mark_blocked(&mut self, id: WindowId, h: HWND) {
        self.ws.remove_window(id);
        self.map.remove(&id);
        self.hidden.remove(&id);
        self.actual.remove(&id);
        self.anim.remove(&id);
        safety::forget(h);
        self.parked.remove(&id);
        self.leaving.remove(&id);
        taskbar::show(h);
        self.blocked.insert(id, h);
        crate::log_warn!(
            "[access denied] {} cannot be managed (likely running as administrator)",
            super::persist::identity(h)
        );
    }

    /// Win+Alt+F: remove the foreground window from the strip (float it, free
    /// to move / resize), or add it back.
    pub(crate) unsafe fn toggle_float(&mut self) {
        let fg = GetForegroundWindow();
        let id = wid(fg);
        if self.floating.remove(&id).is_some() {
            if let Some(h) = self.sticky.remove(&id) {
                let _ = SetWindowPos(
                    h,
                    Some(HWND_NOTOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            crate::log_info!("tile: {}", super::persist::identity(fg)); // It rejoins the strip on the next sync.
            return;
        }
        self.detach_to_float(id);
    }

    /// Lepas window `id` dari strip ke mode float (kalau sedang ditata), lalu perkecil
    /// sedikit dan tengahkan di layar supaya langsung terlihat bedanya dengan tile.
    unsafe fn detach_to_float(&mut self, id: WindowId) -> Option<HWND> {
        let h = self.map.remove(&id)?;
        self.ws.remove_window(id);
        self.hidden.remove(&id);
        self.actual.remove(&id);
        self.anim.remove(&id);
        let work = work_area();
        let cur = ext_bounds(h);
        let cw = (cur.right - cur.left).max(1);
        let ch = (cur.bottom - cur.top).max(1);
        let w = (cw * 80 / 100).min(work.w * 72 / 100).max(480.min(work.w));
        let hh = (ch * 80 / 100).min(work.h * 72 / 100).max(320.min(work.h));
        let _ = place(
            h,
            layout::Rect {
                x: work.x + (work.w - w) / 2,
                y: work.y + (work.h - hh) / 2,
                w,
                h: hh,
            },
        );
        safety::forget(h); // Do not move it when the app exits.
        self.floating.insert(id, h);
        crate::log_info!("float: {}", super::persist::identity(h));
        Some(h)
    }

    /// Win+Alt+S: toggle sticky overlay (float selalu di atas di semua workspace).
    pub(crate) unsafe fn toggle_sticky(&mut self) {
        let fg = GetForegroundWindow();
        let id = wid(fg);
        if let Some(h) = self.sticky.remove(&id) {
            let _ = SetWindowPos(
                h,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            self.floating.remove(&id); // kembali ke tile pada sync berikutnya
            crate::log_info!("unstick: {}", super::persist::identity(h));
            return;
        }
        let h = if let Some(&h) = self.floating.get(&id) {
            h
        } else if let Some(h) = self.detach_to_float(id) {
            h
        } else {
            return;
        };
        let _ = SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        self.sticky.insert(id, h);
        crate::log_info!("sticky: {}", super::persist::identity(h));
    }

    /// Lepas flag always-on-top dari semua window sticky (dipanggil saat pause / keluar).
    pub(crate) unsafe fn unstick_all(&mut self) {
        for (_, h) in self.sticky.drain() {
            if safety::alive(h) {
                let _ = SetWindowPos(
                    h,
                    Some(HWND_NOTOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
}
