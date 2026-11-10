//! Workspace: pindah, kirim window, dan parkir window workspace lain.
//!
//! Window workspace yang tidak aktif diparkir di x = PARK_X (ukuran tetap, jadi saat
//! dibuka lagi hanya posisinya yang berubah) dan tombol taskbar-nya disembunyikan
//! (lihat taskbar.rs). Window tetap "visible" bagi EnumWindows, jadi sync() tidak
//! menganggapnya minimize.

use std::sync::atomic::Ordering;

use layout::{Rect, WindowId};
use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, SetForegroundWindow};

use super::App;
use crate::consts::PARK_X;
use crate::winutil::{bounds, ext_bounds, place, wid};
use crate::{taskbar, tray};

impl App {
    /// Pindah ke workspace nomor `n` (0 = pertama).
    pub(crate) unsafe fn switch_workspace(&mut self, n: usize) {
        if self.ws.switch_to(n) {
            self.after_workspace_change();
        }
    }

    /// Workspace sebelumnya / berikutnya (tidak berputar).
    pub(crate) unsafe fn switch_workspace_rel(&mut self, delta: i32) {
        if self.ws.switch_rel(delta) {
            self.after_workspace_change();
        }
    }

    /// Kirim window yang fokus ke workspace `n`; kita tetap di workspace ini.
    pub(crate) unsafe fn send_to_workspace(&mut self, n: usize) {
        if self.ws.move_focused_to(n).is_some() {
            self.after_workspace_change();
        }
    }

    pub(crate) unsafe fn send_to_workspace_rel(&mut self, delta: i32) {
        if self.ws.move_focused_rel(delta).is_some() {
            self.after_workspace_change();
        }
    }

    unsafe fn after_workspace_change(&mut self) {
        self.apply(); // buka parkir workspace aktif, parkir yang lain
        self.focus_current();
        self.update_workspace_tip();
    }

    /// Fokus ke window yang fokus di workspace aktif; kalau workspace kosong, ke desktop
    /// supaya foreground tidak tertinggal di window yang baru diparkir.
    unsafe fn focus_current(&mut self) {
        let target = self
            .ws
            .cur()
            .focused()
            .and_then(|f| self.map.get(&f).copied());
        // TEBAKAN: SetForegroundWindow(GetShellWindow()) sebagai "tidak ada fokus".
        let h = target.unwrap_or_else(|| GetShellWindow());
        let _ = SetForegroundWindow(h);
        self.last_fg = Some(wid(h));
    }

    /// Tooltip tray menunjukkan workspace aktif.
    pub(crate) unsafe fn update_workspace_tip(&self) {
        if tray::PAUSED.load(Ordering::Relaxed) {
            return;
        }
        tray::set_tip(&format!("vscroll-wm - workspace {}", self.ws.active() + 1));
    }

    /// Saat start: kembalikan tombol taskbar yang mungkin tertinggal terhapus oleh
    /// proses sebelumnya yang mati paksa.
    pub(crate) unsafe fn restore_all_tabs(&self) {
        for &h in self.map.values() {
            taskbar::restore(h);
        }
    }

    /// Parkir semua window di workspace selain yang aktif. Idempoten.
    pub(crate) unsafe fn park_inactive(&mut self) {
        let active = self.ws.active();
        let ids: Vec<WindowId> = (0..self.ws.count())
            .filter(|&n| n != active)
            .flat_map(|n| self.ws.window_ids(n))
            .collect();
        for id in ids {
            self.park(id);
        }
    }

    unsafe fn park(&mut self, id: WindowId) {
        let Some(&h) = self.map.get(&id) else {
            return;
        };
        if !self.parked.insert(id) {
            return;
        }
        self.anim.remove(&id);
        taskbar::hide(h);
        if self.hidden.contains(&id) {
            return; // minimize / cloaked: cukup tombolnya yang disembunyikan
        }
        let cur = ext_bounds(h);
        let target = Rect {
            x: PARK_X,
            y: cur.top,
            w: cur.right - cur.left,
            h: cur.bottom - cur.top,
        };
        let (actual, denied) = place(h, target);
        if denied {
            self.mark_blocked(id, h); // hak akses lebih tinggi: tidak bisa diparkir
            return;
        }
        // yang dicatat posisi sebenarnya (USER32 bisa menjepit), supaya bukan "drift"
        self.actual.insert(id, bounds(actual));
    }
}
