//! Status aplikasi: window yang dikelola, sinkronisasi, dan penataan.

mod actions;
mod anim;
mod events;
mod overview_ctl;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use layout::{Strip, WindowId};

use windows::Win32::Foundation::{HWND, LPARAM};

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use self::anim::Anim;
use crate::consts::DEFAULT_ANIM_MS;
use crate::hook::{CHEAT_VK, OVERVIEW_VK};
use crate::winutil::{
    animations_enabled, bounds, collect, ext_bounds, place, set_border, title, wid, work_area,
    Bounds,
};
use crate::{config, safety};

/// Overview yang sedang terbuka.
pub(crate) struct OverviewSession {
    /// Kursor overview (belum tentu window yang fokus).
    pub(crate) selected: WindowId,
    /// Window yang foreground sebelum overview dibuka, untuk dikembalikan saat batal.
    pub(crate) prev_fg: HWND,
}

pub(crate) struct App {
    pub(crate) strip: Strip,
    pub(crate) map: HashMap<WindowId, HWND>,
    pub(crate) last_fg: Option<WindowId>,
    /// Batas frame tiap window hasil penempatan terakhir kita; kalau berubah
    /// tanpa kita, berarti window digeser dari luar (Win+Arrow, drag, dll).
    pub(crate) actual: HashMap<WindowId, Bounds>,
    pub(crate) anim: HashMap<WindowId, Anim>,
    /// Window yang masih ada tapi sedang tidak tampil (di-minimize, cloaked, dll).
    /// Slotnya tetap diingat di strip.
    pub(crate) hidden: HashSet<WindowId>,
    /// Window yang sengaja dibiarkan mengambang (tidak ditata) lewat Win+Alt+F.
    pub(crate) floating: HashMap<WindowId, HWND>,
    /// Window yang menolak dikendalikan (hak akses lebih tinggi). Dibiarkan mengambang.
    pub(crate) blocked: HashMap<WindowId, HWND>,
    /// 0 = timer animasi tidak jalan (tidak ada beban saat idle).
    pub(crate) anim_timer: usize,
    pub(crate) anim_ms: f32,
    /// Warna border window fokus (COLORREF), dari konfigurasi.
    pub(crate) focus_border: Option<u32>,
    /// Window yang sedang diwarnai bordernya, supaya bisa dikembalikan.
    pub(crate) border_applied: Option<(HWND, u32)>,
    pub(crate) overview: Option<OverviewSession>,
    pub(crate) cfg: config::Config,
    /// Timer sekali jalan untuk menata ulang setelah event (0 = tidak ada).
    pub(crate) sync_timer: usize,
    /// Ada window baru: jadwalkan satu pengecekan ulang.
    pub(crate) recheck: bool,
}

impl App {
    pub(crate) unsafe fn new() -> Self {
        Self {
            strip: Strip::new(work_area()),
            map: HashMap::new(),
            last_fg: None,
            actual: HashMap::new(),
            anim: HashMap::new(),
            hidden: HashSet::new(),
            floating: HashMap::new(),
            blocked: HashMap::new(),
            anim_timer: 0,
            sync_timer: 0,
            recheck: false,
            anim_ms: DEFAULT_ANIM_MS,
            focus_border: None,
            border_applied: None,
            overview: None,
            cfg: config::Config::default(),
        }
    }

    /// Samakan state strip dengan window yang ada sekarang. True kalau ada perubahan.
    pub(crate) unsafe fn sync(&mut self) -> bool {
        let mut list: Vec<HWND> = Vec::new();
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut Vec<HWND> as isize));
        let live: HashMap<WindowId, HWND> = list.iter().map(|&h| (wid(h), h)).collect();
        let mut changed = false;
        self.floating.retain(|_, h| safety::alive(*h)); // yang ditutup tidak perlu diingat
        self.blocked.retain(|_, h| safety::alive(*h));

        let gone: Vec<WindowId> = self
            .map
            .keys()
            .filter(|k| !live.contains_key(k))
            .copied()
            .collect();
        for id in gone {
            let h = self.map[&id];
            if safety::alive(h) {
                // di-minimize / disembunyikan / pindah virtual desktop:
                // window masih ada, jadi ingat slotnya, jangan hapus dari strip
                if self.hidden.insert(id) {
                    self.strip.set_hidden(id, true);
                    self.anim.remove(&id);
                    changed = true;
                }
                continue;
            }
            // benar-benar ditutup
            self.strip.remove_window(id);
            self.map.remove(&id);
            self.hidden.remove(&id);
            safety::forget(h);
            self.actual.remove(&id);
            self.anim.remove(&id);
            changed = true;
        }
        let mut added = false;
        let work = work_area();
        for &h in &list {
            let id = wid(h);
            if self.floating.contains_key(&id) || self.blocked.contains_key(&id) {
                continue;
            }
            if !self.map.contains_key(&id) {
                safety::remember(h, work); // posisi asli, untuk dipulihkan saat keluar
                self.map.insert(id, h);
                self.strip.add_window(id);
                added = true;
            } else if self.hidden.remove(&id) {
                // muncul lagi (mis. setelah Win+D): kembali ke slotnya semula
                self.strip.set_hidden(id, false);
                added = true;
            }
        }
        changed |= added;
        if added {
            self.recheck = true;
        }

        // ikuti window yang difokuskan user (klik, alt-tab)
        let fg = wid(GetForegroundWindow());
        if added || self.last_fg != Some(fg) {
            self.last_fg = Some(fg);
            if self.map.contains_key(&fg) && self.strip.focused() != Some(fg) {
                self.strip.set_focus(fg);
                changed = true;
            }
        }
        // foreground bisa berubah tanpa mengubah layout (mis. setelah Win+Shift+Arrow)
        self.refresh_border();
        changed || self.drifted()
    }

    /// True kalau ada window yang posisinya bukan hasil penempatan kita.
    /// Diabaikan saat tombol kiri mouse ditekan (user sedang drag).
    pub(crate) unsafe fn drifted(&self) -> bool {
        if !self.anim.is_empty() {
            return false; // posisi sedang bergerak karena animasi kita
        }
        if GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 {
            return false;
        }
        self.map
            .iter()
            .filter(|(id, _)| !self.hidden.contains(id))
            .any(|(id, &h)| self.actual.get(id) != Some(&bounds(ext_bounds(h))))
    }

    pub(crate) unsafe fn apply(&mut self) {
        let work = work_area();
        self.strip.set_viewport(work);
        let now = Instant::now();
        // Pass kedua hanya kalau ada window yang menolak dikecilkan
        // (tinggi minimum) sehingga layout perlu dihitung ulang.
        for _ in 0..2 {
            let mut grew = false;
            for p in self.strip.layout() {
                let Some(&h) = self.map.get(&p.id) else {
                    continue;
                };
                let t = p.rect;
                let cur = ext_bounds(h);
                let same_size = cur.right - cur.left == t.w
                    && cur.bottom - cur.top == t.h
                    && !IsZoomed(h).as_bool();
                if same_size {
                    // hanya posisi yang berubah -> boleh dianimasikan
                    self.move_only(p.id, h, (cur.left, cur.top), t, work, now);
                    continue;
                }
                // ukuran berubah -> langsung ke ukuran akhir, tanpa animasi
                self.anim.remove(&p.id);
                let (mut a, denied) = place(h, t);
                if denied {
                    self.mark_blocked(p.id, h);
                    grew = true; // hitung ulang layout tanpa window ini
                    continue;
                }
                let mut got = a.bottom - a.top;
                let mut got_w = a.right - a.left;
                if got > t.h + 2 || got_w > t.w + 2 {
                    // Sebagian app (Electron/UWP) menerapkan ukuran dengan jeda;
                    // ukur ulang sebelum menyimpulkan ada tinggi minimum.
                    std::thread::sleep(Duration::from_millis(40));
                    a = ext_bounds(h);
                    got = a.bottom - a.top;
                    got_w = a.right - a.left;
                }
                self.actual.insert(p.id, bounds(a));
                if got > t.h + 2 && self.strip.set_min_height(p.id, got) {
                    eprintln!(
                        "[min-height] \"{}\" menolak lebih pendek dari {} px (diminta {})",
                        title(h),
                        got,
                        t.h
                    );
                    grew = true;
                }
                if got_w > t.w + 2 && self.strip.set_min_width(p.id, got_w) {
                    eprintln!(
                        "[min-width] \"{}\" menolak lebih sempit dari {} px (diminta {})",
                        title(h),
                        got_w,
                        t.w
                    );
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        if !self.anim.is_empty() && self.anim_timer == 0 {
            // 10 ms = minimum Windows; praktisnya berdetak tiap ~15,6 ms (~64 fps)
            self.anim_timer = SetTimer(None, 0, 10, None);
        }
        self.refresh_border();
    }

    /// Warnai border window yang SEDANG foreground (kalau ditata dan focus_border
    /// diatur), dan kembalikan border window sebelumnya.
    ///
    /// Sengaja berdasar foreground sebenarnya, bukan tujuan fokus di strip: warna
    /// yang dipasang sebelum window selesai diaktifkan bisa hilang saat aktivasi.
    pub(crate) unsafe fn refresh_border(&mut self) {
        let want = match self.focus_border {
            Some(c) => {
                let fg = GetForegroundWindow();
                self.map.contains_key(&wid(fg)).then_some((fg, c))
            }
            None => None,
        };
        if want == self.border_applied {
            return;
        }
        if let Some((old, _)) = self.border_applied.take() {
            if want.map_or(true, |(h, _)| h != old) && safety::alive(old) {
                set_border(old, safety::DWM_COLOR_DEFAULT);
            }
        }
        if let Some((h, c)) = want {
            set_border(h, c);
            self.border_applied = Some((h, c));
        }
    }

    /// Terapkan konfigurasi (gap, animasi, warna border).
    pub(crate) unsafe fn apply_config(&mut self, cfg: &config::Config) {
        self.cfg = cfg.clone();
        OVERVIEW_VK.store(cfg.overview_key, Ordering::Relaxed);
        CHEAT_VK.store(cfg.cheatsheet_key, Ordering::Relaxed);
        self.strip.set_gap(cfg.gap);
        self.focus_border = cfg.focus_border;
        self.refresh_border();
        self.anim_ms = cfg.anim_ms.unwrap_or(if animations_enabled() {
            DEFAULT_ANIM_MS
        } else {
            0.0
        });
    }

    /// Muat ulang file konfigurasi (menu tray). Aturan ignore butuh restart.
    pub(crate) unsafe fn reload_config(&mut self) {
        let cfg = config::load();
        self.apply_config(&cfg);
        self.apply();
    }
}
