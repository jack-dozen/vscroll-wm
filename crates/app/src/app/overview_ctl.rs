//! Pengendali overview (sisi aplikasi; gambarnya ada di overview.rs).

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use layout::WindowId;

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{App, OverviewSession};
use crate::consts::{DIRS, EVENT_DEBOUNCE_MS};
use crate::hook::{OV_MOVED, OV_OPEN};
use crate::winutil::{title, wid, work_area};
use crate::{config, overview, rules, safety};

impl App {
    /// Win+Alt+Space: buka / tutup overview.
    pub(crate) unsafe fn toggle_overview(&mut self) {
        if self.overview.is_some() {
            self.close_overview(false);
        } else {
            self.open_overview();
        }
    }

    pub(crate) unsafe fn open_overview(&mut self) {
        if !self.try_open_overview() {
            // gagal membuka: batalkan status "terbuka" yang mungkin sudah diset hook
            OV_OPEN.store(false, Ordering::Relaxed);
            OV_MOVED.store(false, Ordering::Relaxed);
        }
    }

    pub(crate) unsafe fn try_open_overview(&mut self) -> bool {
        if !overview::available() {
            return false;
        }
        let Some(focused) = self.strip.focused() else {
            return false;
        };
        let data = self.strip.overview();
        if data.items.is_empty() {
            return false;
        }
        let work = work_area();
        let mode = self.cfg.overview_label;
        let titles: HashMap<u64, String> = data
            .items
            .iter()
            .filter_map(|it| {
                self.map.get(&it.id).map(|&h| {
                    let t = title(h);
                    let exe = if mode == config::LabelMode::App {
                        rules::exe_name(h)
                    } else {
                        None
                    };
                    (
                        it.id.0,
                        config::label(mode, &t, exe.as_deref(), &self.cfg.aliases),
                    )
                })
            })
            .collect();
        let sources: HashMap<u64, isize> = data
            .items
            .iter()
            .filter_map(|it| self.map.get(&it.id).map(|&h| (it.id.0, h.0 as isize)))
            .collect();
        let opts = overview::Options {
            hint: self.cfg.overview_hint,
            size_pct: self.cfg.overview_size,
            colors: self.cfg.overview_colors,
        };
        let draw = overview::build(&data, &titles, &sources, focused.0, focused.0, work, &opts);
        self.overview = Some(OverviewSession {
            selected: focused,
            prev_fg: GetForegroundWindow(),
        });
        overview::show(draw, self.cfg.overview_thumbnails);
        OV_MOVED.store(false, Ordering::Relaxed);
        OV_OPEN.store(true, Ordering::Relaxed);
        true
    }

    /// Tab di overview: kursor ke window berikutnya / sebelumnya (urutan baca).
    pub(crate) unsafe fn overview_cycle(&mut self, forward: bool) {
        if let Some(s) = self.overview.as_mut() {
            if let Some(n) = self.strip.cycle_from(s.selected, forward) {
                s.selected = n;
                overview::set_selected(n.0);
            }
        }
    }

    /// Panah di overview: geser kursor ke tetangga (window belum berpindah fokus).
    pub(crate) unsafe fn overview_move(&mut self, dir_index: usize) {
        let Some(dir) = DIRS.get(dir_index).map(|d| d.1) else {
            return;
        };
        if let Some(s) = self.overview.as_mut() {
            if let Some(n) = self.strip.neighbor_of(s.selected, dir) {
                s.selected = n;
                overview::set_selected(n.0);
            }
        }
    }

    pub(crate) unsafe fn overview_pick(&mut self, id: u64) {
        if let Some(s) = self.overview.as_mut() {
            s.selected = WindowId(id);
        }
        self.close_overview(true);
    }

    /// Tutup overview. `accept` = loncat ke window yang dipilih; kalau tidak,
    /// fokus dikembalikan ke window sebelumnya.
    pub(crate) unsafe fn close_overview(&mut self, accept: bool) {
        OV_OPEN.store(false, Ordering::Relaxed);
        OV_MOVED.store(false, Ordering::Relaxed);
        let Some(s) = self.overview.take() else {
            return;
        };
        overview::hide();
        if accept {
            self.strip.set_focus(s.selected);
            self.apply();
            if let Some(&h) = self.map.get(&s.selected) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(wid(h));
            }
            self.schedule_sync(EVENT_DEBOUNCE_MS); // border dan penataan setelah aktivasi
        } else if safety::alive(s.prev_fg) {
            let _ = SetForegroundWindow(s.prev_fg);
        }
    }
}
