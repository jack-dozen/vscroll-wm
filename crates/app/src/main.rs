//! v1: satu monitor. Tracking window lewat event Windows (SetWinEventHook) plus
//! polling lambat 1 detik sebagai jaring pengaman.
//!
//! Win+Arrow             -> snap window aktif (hook, menelan Win+Arrow bawaan)
//! Win+Shift+Arrow       -> pindah fokus
//! Win+Enter             -> kembalikan window aktif ke penuh
//! Win+Alt+Arrow         -> tukar posisi dengan window tetangga
//! Win+Alt+F             -> float / tile window aktif
//! Win+Alt+I             -> cetak info window aktif (class / exe / title) di console
//! Win+Shift+Q           -> keluar dan kembalikan semua window (juga Ctrl+C)
//! Ctrl+Alt+... (cadangan, aksi yang sama)
//!
//! Aslinya:
//! Ctrl+Alt+Arrow        -> snap window aktif
//! Ctrl+Alt+Shift+Arrow  -> pindah fokus
//! Ctrl+Alt+Enter        -> kembalikan window aktif ke penuh (lebar + tinggi)
//! Tutup dengan Ctrl+C di console.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod rules;
mod safety;
mod tray;

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use layout::{Dir, Rect, Strip, WindowId};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const DIRS: [(VIRTUAL_KEY, Dir); 4] = [
    (VK_LEFT, Dir::Left),
    (VK_RIGHT, Dir::Right),
    (VK_UP, Dir::Up),
    (VK_DOWN, Dir::Down),
];

const HOTKEY_MAXIMIZE: i32 = 9;
const HOTKEY_QUIT: i32 = 14;
const HOTKEY_FLOAT: i32 = 15;
const HOTKEY_INSPECT: i32 = 16;
/// Pesan dari callback event Windows ke thread utama.
const WM_WINEVENT: u32 = WM_APP + 3;
/// Jeda (ms) mengumpulkan rentetan event sebelum menata ulang.
const EVENT_DEBOUNCE_MS: u32 = 40;
/// Cek ulang setelah window baru ditata (sebagian app mengubah ukurannya sendiri
/// beberapa saat setelah muncul).
const RECHECK_MS: u32 = 250;
/// Gap default (piksel fisik). Ubah tanpa kompilasi ulang lewat env VSCROLL_GAP.
const DEFAULT_GAP: i32 = 16;
/// Durasi animasi (ms). 0 = mati. Ubah lewat env VSCROLL_ANIM_MS.
const DEFAULT_ANIM_MS: f32 = 160.0;

fn wid(h: HWND) -> WindowId {
    WindowId(h.0 as usize as u64)
}

/// Kenapa window ini TIDAK ditata (None = boleh ditata).
unsafe fn unmanaged_reason(hwnd: HWND) -> Option<&'static str> {
    if !IsWindowVisible(hwnd).as_bool() {
        return Some("tidak terlihat");
    }
    if IsIconic(hwnd).as_bool() {
        return Some("sedang di-minimize");
    }
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if style & WS_CAPTION.0 != WS_CAPTION.0 {
        return Some("tidak punya title bar (WS_CAPTION)");
    }
    // tidak bisa di-resize (dialog, installer, utilitas kecil): biarkan mengambang
    if style & WS_THICKFRAME.0 == 0 {
        return Some("tidak bisa di-resize (tanpa WS_THICKFRAME)");
    }
    if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
        return Some("tool window / tidak bisa diaktifkan");
    }
    if GetWindow(hwnd, GW_OWNER).map_or(false, |o| o != HWND::default()) {
        return Some("dialog / popup milik window lain");
    }
    if GetWindowTextLengthW(hwnd) == 0 {
        return Some("judul kosong");
    }
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut c_void,
        4,
    );
    if cloaked != 0 {
        return Some("cloaked (desktop virtual lain / app UWP tersembunyi)");
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == GetCurrentProcessId() {
        return Some("jendela app ini sendiri");
    }
    rules::ignored(hwnd)
}

unsafe fn manageable(hwnd: HWND) -> bool {
    unmanaged_reason(hwnd).is_none()
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<HWND>);
    if manageable(hwnd) {
        list.push(hwnd);
    }
    BOOL(1)
}

unsafe fn work_area() -> Rect {
    let mut r = RECT::default();
    let _ = SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        Some(&mut r as *mut RECT as *mut c_void),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    );
    Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
}

unsafe fn ext_bounds(hwnd: HWND) -> RECT {
    let mut ext = RECT::default();
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut ext as *mut RECT as *mut c_void,
        std::mem::size_of::<RECT>() as u32,
    );
    ext
}

/// Pindahkan window dengan kompensasi invisible border Windows 10/11.
/// Mengembalikan batas frame yang terlihat SETELAH dipindah (bisa beda dari
/// yang diminta kalau app punya ukuran minimum).
unsafe fn place(hwnd: HWND, r: Rect) -> RECT {
    if IsZoomed(hwnd).as_bool() {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }
    let mut win = RECT::default();
    let _ = GetWindowRect(hwnd, &mut win);
    let ext = ext_bounds(hwnd);
    let (l, t) = (ext.left - win.left, ext.top - win.top);
    let (rr, b) = (win.right - ext.right, win.bottom - ext.bottom);
    let _ = SetWindowPos(
        hwnd,
        None,
        r.x - l,
        r.y - t,
        r.w + l + rr,
        r.h + t + b,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
    ext_bounds(hwnd)
}

unsafe fn title(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = GetWindowTextW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

type Bounds = (i32, i32, i32, i32);

fn bounds(r: RECT) -> Bounds {
    (r.left, r.top, r.right, r.bottom)
}

const WM_ACTION: u32 = WM_APP + 1;
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static WIN_DOWN: AtomicBool = AtomicBool::new(false);

fn arrow_index(vk: u16) -> Option<i32> {
    DIRS.iter().position(|(k, _)| k.0 == vk).map(|i| i as i32)
}

/// Selipkan tombol dummy supaya Windows tidak membuka Start Menu saat Win
/// dilepas (arrow-nya kita telan, jadi Windows mengira Win ditekan sendirian).
unsafe fn cancel_start_menu() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0xE8),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

/// Low-level keyboard hook. Harus cepat: kerjanya cuma menelan tombol dan
/// mengirim pesan ke thread utama, eksekusinya di message loop.
unsafe extern "system" fn kb_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && !tray::PAUSED.load(Ordering::Relaxed) {
        let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if k.flags.0 & LLKHF_INJECTED.0 == 0 {
            let m = wparam.0 as u32;
            let down = m == WM_KEYDOWN || m == WM_SYSKEYDOWN;
            let vk = k.vkCode as u16;
            if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
                WIN_DOWN.store(down, Ordering::Relaxed);
            } else if WIN_DOWN.load(Ordering::Relaxed)
                && (GetAsyncKeyState(VK_LWIN.0 as i32) < 0
                    || GetAsyncKeyState(VK_RWIN.0 as i32) < 0)
            {
                let shift = GetAsyncKeyState(VK_SHIFT.0 as i32) < 0;
                let alt = GetAsyncKeyState(VK_MENU.0 as i32) < 0;
                let action = if let Some(i) = arrow_index(vk) {
                    // Win+Alt: tukar posisi, Win+Shift: fokus, Win: snap
                    Some(if alt {
                        i + 10
                    } else if shift {
                        i + 5
                    } else {
                        i + 1
                    })
                } else if vk == VK_RETURN.0 {
                    Some(HOTKEY_MAXIMIZE)
                } else if vk == VK_Q.0 && shift {
                    Some(HOTKEY_QUIT)
                } else if vk == VK_F.0 && alt {
                    Some(HOTKEY_FLOAT)
                } else if vk == VK_I.0 && alt {
                    Some(HOTKEY_INSPECT)
                } else {
                    None
                };
                if let Some(id) = action {
                    if down {
                        let _ = PostThreadMessageW(
                            MAIN_THREAD.load(Ordering::Relaxed),
                            WM_ACTION,
                            WPARAM(id as usize),
                            LPARAM(0),
                        );
                        cancel_start_menu();
                    }
                    return LRESULT(1); // telan tombolnya
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Callback event Windows. Dipanggil di thread utama lewat message loop;
/// kerjanya sengaja minimal: teruskan sebagai pesan, penyaringan di `on_event`.
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != 0 || id_child != 0 || hwnd.0.is_null() {
        return; // hanya window itu sendiri, bukan elemen di dalamnya
    }
    let _ = PostThreadMessageW(
        MAIN_THREAD.load(Ordering::Relaxed),
        WM_WINEVENT,
        WPARAM(event as usize),
        LPARAM(hwnd.0 as isize),
    );
}

/// Animasi posisi satu window (ukuran tidak ikut dianimasikan).
struct Anim {
    hwnd: HWND,
    from: (i32, i32),
    to: (i32, i32),
    /// Selisih rect window dengan frame yang terlihat (invisible border).
    inset: (i32, i32),
    start: Instant,
    dur_ms: f32,
}

impl Anim {
    /// Posisi frame pada waktu `now` dan apakah animasi sudah selesai.
    /// Kurva ease-out cubic: cepat di awal, melambat di akhir.
    fn pos_at(&self, now: Instant) -> ((i32, i32), bool) {
        let t = if self.dur_ms <= 0.0 {
            1.0
        } else {
            (now.duration_since(self.start).as_secs_f32() * 1000.0 / self.dur_ms).min(1.0)
        };
        let e = 1.0 - (1.0 - t).powi(3);
        let f = |a: i32, b: i32| (a as f32 + (b - a) as f32 * e).round() as i32;
        ((f(self.from.0, self.to.0), f(self.from.1, self.to.1)), t >= 1.0)
    }
}

fn hits(area: Rect, x: i32, y: i32, w: i32, h: i32) -> bool {
    x < area.x + area.w && x + w > area.x && y < area.y + area.h && y + h > area.y
}

unsafe fn insets(hwnd: HWND) -> (i32, i32) {
    let mut win = RECT::default();
    let _ = GetWindowRect(hwnd, &mut win);
    let ext = ext_bounds(hwnd);
    (ext.left - win.left, ext.top - win.top)
}

/// Hormati pengaturan Windows "Animation effects".
unsafe fn animations_enabled() -> bool {
    let mut on = BOOL(1);
    let r = SystemParametersInfoW(
        SPI_GETCLIENTAREAANIMATION,
        0,
        Some(&mut on as *mut BOOL as *mut c_void),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    );
    r.is_err() || on.as_bool()
}

struct App {
    strip: Strip,
    map: HashMap<WindowId, HWND>,
    last_fg: Option<WindowId>,
    /// Batas frame tiap window hasil penempatan terakhir kita; kalau berubah
    /// tanpa kita, berarti window digeser dari luar (Win+Arrow, drag, dll).
    actual: HashMap<WindowId, Bounds>,
    anim: HashMap<WindowId, Anim>,
    /// Window yang masih ada tapi sedang tidak tampil (di-minimize, cloaked, dll).
    /// Slotnya tetap diingat di strip.
    hidden: HashSet<WindowId>,
    /// Window yang sengaja dibiarkan mengambang (tidak ditata) lewat Win+Alt+F.
    floating: HashMap<WindowId, HWND>,
    /// 0 = timer animasi tidak jalan (tidak ada beban saat idle).
    anim_timer: usize,
    anim_ms: f32,
    /// Timer sekali jalan untuk menata ulang setelah event (0 = tidak ada).
    sync_timer: usize,
    /// Ada window baru: jadwalkan satu pengecekan ulang.
    recheck: bool,
}

impl App {
    unsafe fn new() -> Self {
        Self {
            strip: {
                let mut s = Strip::new(work_area());
                let gap = std::env::var("VSCROLL_GAP")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_GAP);
                s.set_gap(gap);
                s
            },
            map: HashMap::new(),
            last_fg: None,
            actual: HashMap::new(),
            anim: HashMap::new(),
            hidden: HashSet::new(),
            floating: HashMap::new(),
            anim_timer: 0,
            sync_timer: 0,
            recheck: false,
            anim_ms: std::env::var("VSCROLL_ANIM_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if animations_enabled() { DEFAULT_ANIM_MS } else { 0.0 }),
        }
    }

    /// Samakan state strip dengan window yang ada sekarang. True kalau ada perubahan.
    unsafe fn sync(&mut self) -> bool {
        let mut list: Vec<HWND> = Vec::new();
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut Vec<HWND> as isize));
        let live: HashMap<WindowId, HWND> = list.iter().map(|&h| (wid(h), h)).collect();
        let mut changed = false;
        self.floating.retain(|_, h| safety::alive(*h)); // yang ditutup tidak perlu diingat

        let gone: Vec<WindowId> =
            self.map.keys().filter(|k| !live.contains_key(k)).copied().collect();
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
            if self.floating.contains_key(&id) {
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
        changed || self.drifted()
    }

    /// True kalau ada window yang posisinya bukan hasil penempatan kita.
    /// Diabaikan saat tombol kiri mouse ditekan (user sedang drag).
    unsafe fn drifted(&self) -> bool {
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

    unsafe fn apply(&mut self) {
        let work = work_area();
        self.strip.set_viewport(work);
        let now = Instant::now();
        // Pass kedua hanya kalau ada window yang menolak dikecilkan
        // (tinggi minimum) sehingga layout perlu dihitung ulang.
        for _ in 0..2 {
            let mut grew = false;
            for p in self.strip.layout() {
                let Some(&h) = self.map.get(&p.id) else { continue };
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
                let mut a = place(h, t);
                let mut got = a.bottom - a.top;
                if got > t.h + 2 {
                    // Sebagian app (Electron/UWP) menerapkan ukuran dengan jeda;
                    // ukur ulang sebelum menyimpulkan ada tinggi minimum.
                    std::thread::sleep(Duration::from_millis(40));
                    a = ext_bounds(h);
                    got = a.bottom - a.top;
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
            }
            if !grew {
                break;
            }
        }
        if !self.anim.is_empty() && self.anim_timer == 0 {
            // 10 ms = minimum Windows; praktisnya berdetak tiap ~15,6 ms (~64 fps)
            self.anim_timer = SetTimer(None, 0, 10, None);
        }
    }

    /// Pindahkan window yang ukurannya sudah benar ke `t`: dianimasikan kalau
    /// terlihat di layar, langsung kalau di luar layar (tanpa kerja sia-sia).
    unsafe fn move_only(
        &mut self,
        id: WindowId,
        hwnd: HWND,
        cur: (i32, i32),
        t: Rect,
        work: Rect,
        now: Instant,
    ) {
        let to = (t.x, t.y);
        if self.anim.get(&id).map_or(false, |a| a.to == to) {
            return; // sudah menuju ke sana
        }
        // animasi yang sedang berjalan dilanjutkan dari posisi saat ini (mulus)
        let from = self.anim.get(&id).map_or(cur, |a| a.pos_at(now).0);
        self.actual.insert(id, (t.x, t.y, t.x + t.w, t.y + t.h));
        if from == to {
            self.anim.remove(&id);
            return;
        }
        let inset = insets(hwnd);
        let visible =
            hits(work, from.0, from.1, t.w, t.h) || hits(work, t.x, t.y, t.w, t.h);
        if self.anim_ms > 0.0 && visible {
            self.anim.insert(
                id,
                Anim { hwnd, from, to, inset, start: now, dur_ms: self.anim_ms },
            );
        } else {
            self.anim.remove(&id);
            let _ = SetWindowPos(
                hwnd,
                None,
                to.0 - inset.0,
                to.1 - inset.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Jeda / lanjutkan (menu tray). Saat dijeda semua window dikembalikan ke
    /// posisi asli dan hotkey diteruskan ke Windows.
    unsafe fn toggle_pause(&mut self) {
        let paused = !tray::PAUSED.load(Ordering::Relaxed);
        tray::PAUSED.store(paused, Ordering::Relaxed);
        tray::set_tip(if paused { "vscroll-wm (dijeda)" } else { "vscroll-wm" });
        if paused {
            self.finish_all_anims();
            self.stop_sync_timer();
            safety::restore_all();
        } else {
            self.sync();
            self.apply();
        }
    }

    /// Win+Alt+I: tampilkan class, exe, dan title window yang sedang aktif, plus
    /// alasan ditata atau tidaknya. Berguna untuk menulis aturan di vscroll-wm.ignore.
    unsafe fn inspect(&self) {
        let h = GetForegroundWindow();
        let id = wid(h);
        let class = rules::class_name(h);
        let title = rules::window_title(h);
        let exe = rules::exe_name(h).unwrap_or_else(|| "(tidak terbaca)".into());
        let status = if self.floating.contains_key(&id) {
            "mengambang (Win+Alt+F)".to_string()
        } else if self.map.contains_key(&id) {
            "ditata".to_string()
        } else {
            match unmanaged_reason(h) {
                Some(why) => format!("tidak ditata: {why}"),
                None => "belum ditata (menunggu sinkronisasi)".to_string(),
            }
        };
        let text = format!(
            "judul : {title}\nclass : {class}\nexe   : {exe}\nstatus: {status}\n\nBaris untuk vscroll-wm.ignore:\n  class:{class}\n  exe:{exe}\n  title:{title}"
        );
        println!("---------- window aktif ----------\n{text}\n----------------------------------");
        if !cfg!(debug_assertions) {
            // Build release tidak punya console: tampilkan di kotak pesan, di thread
            // terpisah supaya message loop tidak terblokir.
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            std::thread::spawn(move || {
                MessageBoxW(
                    None,
                    PCWSTR(wide.as_ptr()),
                    w!("vscroll-wm: info window"),
                    MB_OK | MB_ICONINFORMATION,
                );
            });
        }
    }

    /// Win+Alt+F: window yang sedang di foreground dikeluarkan dari strip
    /// (mengambang, bebas digeser/di-resize), atau dimasukkan kembali.
    unsafe fn toggle_float(&mut self) {
        let fg = GetForegroundWindow();
        let id = wid(fg);
        if self.floating.remove(&id).is_some() {
            println!("tile: {}", title(fg)); // masuk strip lagi lewat sync berikutnya
            return;
        }
        let Some(h) = self.map.remove(&id) else { return };
        self.strip.remove_window(id);
        self.hidden.remove(&id);
        self.actual.remove(&id);
        if let Some(a) = self.anim.remove(&id) {
            // selesaikan gerakan dulu supaya berhenti di posisi akhirnya
            let _ = SetWindowPos(
                h,
                None,
                a.to.0 - a.inset.0,
                a.to.1 - a.inset.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        safety::forget(h); // tidak ikut dipindahkan saat app keluar
        self.floating.insert(id, h);
        println!("float: {}", title(h));
    }

    /// Jadwalkan sync + apply dalam `ms` milidetik (digabung kalau sudah terjadwal).
    unsafe fn schedule_sync(&mut self, ms: u32) {
        if self.sync_timer == 0 {
            self.sync_timer = SetTimer(None, 0, ms, None);
        }
    }

    unsafe fn stop_sync_timer(&mut self) {
        if self.sync_timer != 0 {
            let _ = KillTimer(None, self.sync_timer);
            self.sync_timer = 0;
        }
    }

    /// Event Windows: hanya yang menyangkut window yang kita kelola (atau
    /// kandidat baru) yang memicu penataan ulang.
    unsafe fn on_event(&mut self, event: u32, hwnd: HWND) {
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
        if relevant {
            self.schedule_sync(EVENT_DEBOUNCE_MS);
        }
    }

    unsafe fn stop_anim_timer(&mut self) {
        if self.anim_timer != 0 {
            let _ = KillTimer(None, self.anim_timer);
            self.anim_timer = 0;
        }
    }

    /// Satu frame animasi: semua window yang bergerak dipindah dalam satu batch.
    unsafe fn tick_anim(&mut self) {
        if self.anim.is_empty() {
            self.stop_anim_timer();
            return;
        }
        let now = Instant::now();
        let Ok(mut hdwp) = BeginDeferWindowPos(self.anim.len() as i32) else {
            self.finish_all_anims();
            return;
        };
        let mut done = Vec::new();
        let mut ok = true;
        for (id, a) in &self.anim {
            let ((x, y), finished) = a.pos_at(now);
            match DeferWindowPos(
                hdwp,
                a.hwnd,
                None,
                x - a.inset.0,
                y - a.inset.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) {
                Ok(next) => hdwp = next,
                Err(_) => {
                    ok = false;
                    break;
                }
            }
            if finished {
                done.push(*id);
            }
        }
        if !ok {
            self.finish_all_anims(); // mis. window sudah ditutup: langsung ke akhir
            return;
        }
        let _ = EndDeferWindowPos(hdwp);
        for id in done {
            if let Some(a) = self.anim.remove(&id) {
                self.actual.insert(id, bounds(ext_bounds(a.hwnd)));
            }
        }
        if self.anim.is_empty() {
            self.stop_anim_timer();
        }
    }

    /// Lompat semua animasi ke posisi akhir (jalur darurat).
    unsafe fn finish_all_anims(&mut self) {
        for (id, a) in std::mem::take(&mut self.anim) {
            let _ = SetWindowPos(
                a.hwnd,
                None,
                a.to.0 - a.inset.0,
                a.to.1 - a.inset.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            self.actual.insert(id, bounds(ext_bounds(a.hwnd)));
        }
        self.stop_anim_timer();
    }

    unsafe fn on_hotkey(&mut self, id: i32) {
        if tray::PAUSED.load(Ordering::Relaxed) {
            return;
        }
        self.sync();
        match id {
            1..=4 => self.strip.snap(DIRS[(id - 1) as usize].1),
            5..=8 => self.strip.focus(DIRS[(id - 5) as usize].1),
            HOTKEY_MAXIMIZE => self.strip.maximize(),
            HOTKEY_INSPECT => {
                self.inspect();
                return;
            }
            HOTKEY_FLOAT => {
                self.toggle_float();
                self.sync(); // window yang baru di-tile masuk strip
            }
            10..=13 => self.strip.move_window(DIRS[(id - 10) as usize].1),
            _ => return,
        }
        self.apply();
        if (5..=8).contains(&id) {
            if let Some(h) = self.strip.focused().and_then(|f| self.map.get(&f).copied()) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(wid(h));
            }
        }
    }
}

/// Cegah dua salinan berjalan bersamaan (mis. autostart + dijalankan manual):
/// dua hook keyboard dan dua penata window akan saling berebut.
unsafe fn single_instance() -> bool {
    match CreateMutexW(None, true, w!("Local\\vscroll-wm-single-instance")) {
        Ok(_handle) => GetLastError() != ERROR_ALREADY_EXISTS, // handle sengaja tidak ditutup
        Err(_) => true,
    }
}

fn main() {
    unsafe {
        if !single_instance() {
            eprintln!("vscroll-wm sudah berjalan.");
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut app = App::new();

        for (i, (vk, _)) in DIRS.iter().enumerate() {
            let base = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
            if RegisterHotKey(None, i as i32 + 1, base, vk.0 as u32).is_err() {
                eprintln!("gagal daftar Ctrl+Alt+{:?} (dipakai app lain?)", vk);
            }
            if RegisterHotKey(None, i as i32 + 5, base | MOD_SHIFT, vk.0 as u32).is_err() {
                eprintln!("gagal daftar Ctrl+Alt+Shift+{:?} (dipakai app lain?)", vk);
            }
        }
        if RegisterHotKey(
            None,
            HOTKEY_MAXIMIZE,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_RETURN.0 as u32,
        )
        .is_err()
        {
            eprintln!("gagal daftar Ctrl+Alt+Enter (dipakai app lain?)");
        }
        if RegisterHotKey(
            None,
            HOTKEY_QUIT,
            MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
            VK_Q.0 as u32,
        )
        .is_err()
        {
            eprintln!("gagal daftar Ctrl+Alt+Shift+Q (dipakai app lain?)");
        }
        if RegisterHotKey(
            None,
            HOTKEY_FLOAT,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_F.0 as u32,
        )
        .is_err()
        {
            eprintln!("gagal daftar Ctrl+Alt+F (dipakai app lain?)");
        }
        if RegisterHotKey(
            None,
            HOTKEY_INSPECT,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_I.0 as u32,
        )
        .is_err()
        {
            eprintln!("gagal daftar Ctrl+Alt+I (dipakai app lain?)");
        }
        MAIN_THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
        safety::install(GetCurrentThreadId());
        if !tray::create(GetCurrentThreadId()) {
            eprintln!("gagal membuat ikon tray");
        }
        let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), None, 0) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("gagal pasang keyboard hook (Win+Arrow tidak aktif): {e}");
                None
            }
        };
        let mut event_hooks: Vec<HWINEVENTHOOK> = Vec::new();
        for (lo, hi) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ] {
            let h = SetWinEventHook(
                lo,
                hi,
                None,
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if h.is_invalid() {
                eprintln!("gagal pasang event hook {lo:#x}..{hi:#x}");
            } else {
                event_hooks.push(h);
            }
        }
        // Dengan event hook, polling cuma jaring pengaman (mis. window yang baru
        // diberi judul setelah muncul). Tanpa hook, kembali ke polling cepat.
        let poll_ms = if event_hooks.is_empty() { 300 } else { 1000 };
        SetTimer(None, 1, poll_ms, None);

        println!("vscroll-wm jalan. Win+Arrow = snap, +Shift = fokus, Win+Enter = penuh, Win+Alt = tukar posisi, Win+Alt+F = float/tile, Win+Alt+I = info window (cadangan: Ctrl+Alt+...). Win+Shift+Q atau Ctrl+C = keluar dan pulihkan semua window.");
        if app.sync() {
            app.apply();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                safety::WM_QUIT_REQUEST => break,
                WM_HOTKEY | WM_ACTION => {
                    let id = msg.wParam.0 as i32;
                    if id == HOTKEY_QUIT {
                        break;
                    }
                    app.on_hotkey(id);
                }
                tray::WM_TRAY_PAUSE => app.toggle_pause(),
                WM_WINEVENT => {
                    if !tray::PAUSED.load(Ordering::Relaxed) {
                        app.on_event(msg.wParam.0 as u32, HWND(msg.lParam.0 as *mut c_void));
                    }
                }
                WM_TIMER => {
                    let id = msg.wParam.0;
                    let paused = tray::PAUSED.load(Ordering::Relaxed);
                    if app.anim_timer != 0 && id == app.anim_timer {
                        app.tick_anim();
                    } else if app.sync_timer != 0 && id == app.sync_timer {
                        app.stop_sync_timer();
                        if !paused {
                            if app.sync() {
                                app.apply();
                            }
                            if std::mem::take(&mut app.recheck) {
                                app.schedule_sync(RECHECK_MS);
                            }
                        }
                    } else if !paused && app.sync() {
                        app.apply(); // jaring pengaman
                    }
                }
                _ => {
                    // pesan untuk window tray (menu, dll)
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        // Keluar: kembalikan semua window ke posisi aslinya.
        app.stop_anim_timer();
        app.stop_sync_timer();
        for h in event_hooks {
            let _ = UnhookWinEvent(h);
        }
        if let Some(h) = hook {
            let _ = UnhookWindowsHookEx(h);
        }
        tray::remove();
        safety::restore_all();
        safety::EXIT_DONE.store(true, Ordering::Relaxed);
        println!("semua window dikembalikan. sampai jumpa.");
    }
}
