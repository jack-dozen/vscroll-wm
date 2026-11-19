//! Mode angkat-dan-jatuhkan (`pick`) langsung di desktop:
//!  - Daftar slot mendarat (`build_slots`) dan navigasi panah (`step_slot`), murni tanpa Win32.
//!  - Jendela overlay semi-transparan (`show` / `hide`) yang menggambar marker tempat
//!    mendarat di atas desktop tanpa merebut fokus dan tembus klik.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use layout::{Dir, InsertPosition, Overview, Rect, WindowId};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::OverviewColors;
use crate::theme::{self, Canvas, Palette};
use crate::winutil::animations_enabled;

/// Tinggi pita horizontal untuk slot `FullBefore` / `FullAfter` (di antara window).
const BAR_H: i32 = 30;
/// Toleransi piksel untuk menyamakan tepi / lebar kolom.
const SLACK: i32 = 4;
/// Opasitas overlay (0..=255), menyerupai pratinjau Aero Snap Windows 11.
const ALPHA: u8 = 195;
const ANIM_TIMER_ID: usize = 1;
const ANIM_TICK_MS: u32 = 16;
const GLIDE_MS: f32 = 130.0;
const FADE_MS: f32 = 120.0;

/// Jenis tempat mendarat yang bisa dipilih di mode `pick`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickTarget {
    /// Sisipkan penuh lebar di atas blok window ini.
    FullBefore(WindowId),
    /// Sisipkan penuh lebar di bawah blok window ini.
    FullAfter(WindowId),
    /// Bagi dua window ini (`first`: kiri untuk penuh lebar, atas untuk setengah lebar).
    Split(WindowId, bool),
    /// Isi sisi kosong di sebelah window setengah lebar ini.
    Beside(WindowId),
    /// Taruh di workspace kosong (atau satu-satunya window di workspace ini).
    EmptyWorkspace,
}

impl PickTarget {
    pub fn to_insert_position(self) -> Option<InsertPosition> {
        match self {
            PickTarget::FullBefore(id) => Some(InsertPosition::FullBefore(id)),
            PickTarget::FullAfter(id) => Some(InsertPosition::FullAfter(id)),
            PickTarget::Split(id, first) => Some(InsertPosition::Split(id, first)),
            PickTarget::Beside(id) => Some(InsertPosition::Beside(id)),
            PickTarget::EmptyWorkspace => None,
        }
    }
}

/// Satu kandidat tempat mendarat di koordinat strip (belum dikurangi scroll).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickSlot {
    pub target: PickTarget,
    /// Kotak marker di koordinat strip (0..ov.width, 0..ov.height).
    pub rect: Rect,
    /// Window acuan untuk menggulung viewport agar slot ini terlihat.
    pub anchor: Option<WindowId>,
}

/// Bangun semua slot mendarat untuk satu workspace, mengecualikan `src` sendiri.
/// Urutan vertikal: celah atas blok -> isi blok (split / beside) -> celah bawah blok -> ...
pub fn build_slots(ov: &Overview, src: WindowId) -> Vec<PickSlot> {
    let others: Vec<_> = ov.items.iter().copied().filter(|it| it.id != src).collect();
    if others.is_empty() {
        let m = 24;
        return vec![PickSlot {
            target: PickTarget::EmptyWorkspace,
            rect: Rect {
                x: m,
                y: m,
                w: (ov.width - 2 * m).max(40),
                h: (ov.view_h - 2 * m).max(40),
            },
            anchor: None,
        }];
    }

    // Kelompokkan window lain jadi blok vertikal (rentang y yang saling tumpang tindih).
    let mut sorted = others;
    sorted.sort_by_key(|it| (it.rect.y, it.rect.x));
    let mut blocks: Vec<(i32, i32, Vec<layout::OverviewItem>)> = Vec::new();
    for it in sorted {
        let top = it.rect.y;
        let bot = it.rect.y + it.rect.h;
        if let Some(last) = blocks.last_mut() {
            if top < last.1 - SLACK {
                last.1 = last.1.max(bot);
                last.2.push(it);
                continue;
            }
        }
        blocks.push((top, bot, vec![it]));
    }

    let mut slots = Vec::new();
    let w = ov.width.max(1);

    // 1. Celah paling atas strip (sebelum blok pertama)
    if let Some((top, _, items)) = blocks.first() {
        let first_id = items[0].id;
        slots.push(PickSlot {
            target: PickTarget::FullBefore(first_id),
            rect: Rect {
                x: 0,
                y: *top,
                w,
                h: BAR_H,
            },
            anchor: Some(first_id),
        });
    }

    // 2. Untuk setiap blok: isi blok (split / beside), lalu celah di bawah blok
    for (bi, (b_top, b_bot, items)) in blocks.iter().enumerate() {
        let is_full = |r: Rect| r.w >= w - SLACK;
        // Cek apakah ini blok setengah lebar yang hanya punya satu sisi
        let has_left = items.iter().any(|it| !is_full(it.rect) && it.rect.x <= SLACK);
        let has_right = items
            .iter()
            .any(|it| !is_full(it.rect) && it.rect.x + it.rect.w >= w - SLACK);

        for it in items {
            let r = it.rect;
            if is_full(r) {
                let half_w = r.w / 2;
                slots.push(PickSlot {
                    target: PickTarget::Split(it.id, true),
                    rect: Rect {
                        x: r.x,
                        y: r.y,
                        w: half_w,
                        h: r.h,
                    },
                    anchor: Some(it.id),
                });
                slots.push(PickSlot {
                    target: PickTarget::Split(it.id, false),
                    rect: Rect {
                        x: r.x + half_w,
                        y: r.y,
                        w: r.w - half_w,
                        h: r.h,
                    },
                    anchor: Some(it.id),
                });
            } else {
                let half_h = r.h / 2;
                slots.push(PickSlot {
                    target: PickTarget::Split(it.id, true),
                    rect: Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: half_h,
                    },
                    anchor: Some(it.id),
                });
                slots.push(PickSlot {
                    target: PickTarget::Split(it.id, false),
                    rect: Rect {
                        x: r.x,
                        y: r.y + half_h,
                        w: r.w,
                        h: r.h - half_h,
                    },
                    anchor: Some(it.id),
                });
            }
        }

        // Kalau cuma satu lajur yang terisi di blok ini, tawarkan setengah kosong `Beside`
        if has_left ^ has_right {
            if let Some(first) = items.first() {
                let r = first.rect;
                let (bx, bw) = if has_left {
                    (r.x + r.w, (w - (r.x + r.w)).max(1))
                } else {
                    (0, r.x.max(1))
                };
                slots.push(PickSlot {
                    target: PickTarget::Beside(first.id),
                    rect: Rect {
                        x: bx,
                        y: *b_top,
                        w: bw,
                        h: (*b_bot - *b_top).max(1),
                    },
                    anchor: Some(first.id),
                });
            }
        }

        // Celah di bawah blok (sekaligus pemisah ke blok berikutnya)
        let last_id = items[0].id;
        let is_last = bi + 1 == blocks.len();
        let bar_y = if is_last {
            (*b_bot - BAR_H).max(*b_top)
        } else {
            *b_bot - BAR_H / 2
        };
        slots.push(PickSlot {
            target: PickTarget::FullAfter(last_id),
            rect: Rect {
                x: 0,
                y: bar_y,
                w,
                h: BAR_H,
            },
            anchor: Some(last_id),
        });
    }

    slots
}

/// Pilih slot awal yang paling dekat dengan posisi `src_rect` (atau 0 kalau tidak ada).
pub fn initial_slot(slots: &[PickSlot], src_rect: Option<Rect>) -> usize {
    let Some(sr) = src_rect else { return 0 };
    let (sx, sy) = (sr.x + sr.w / 2, sr.y + sr.h / 2);
    slots
        .iter()
        .enumerate()
        .min_by_key(|(_, s)| {
            let cx = s.rect.x + s.rect.w / 2;
            let cy = s.rect.y + s.rect.h / 2;
            (cy - sy).abs() * 2 + (cx - sx).abs()
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Pilih slot masuk saat menyeberang dari workspace sebelah (`from_left` = masuk dari
/// sisi kiri, artinya menekan Kanan).
pub fn entry_slot(slots: &[PickSlot], from_left: bool, y: i32) -> usize {
    if slots.is_empty() {
        return 0;
    }
    let edge = if from_left {
        slots.iter().map(|s| s.rect.x).min().unwrap_or(0)
    } else {
        slots.iter().map(|s| s.rect.x + s.rect.w).max().unwrap_or(0)
    };
    slots
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            if from_left {
                s.rect.x - edge <= SLACK
            } else {
                edge - (s.rect.x + s.rect.w) <= SLACK
            }
        })
        .min_by_key(|(_, s)| ((s.rect.y + s.rect.h / 2 - y).abs(), s.rect.y))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Geser pilihan slot ke arah `dir`. None kalau tidak ada slot lagi di arah itu
/// (untuk Kiri / Kanan berarti saatnya menyeberang ke workspace sebelah).
pub fn step_slot(slots: &[PickSlot], cur: usize, dir: Dir) -> Option<usize> {
    let f = slots.get(cur)?;
    let (fx0, fx1) = (f.rect.x, f.rect.x + f.rect.w);
    let (fy0, fy1) = (f.rect.y, f.rect.y + f.rect.h);
    let (fcx, fcy) = ((fx0 + fx1) / 2, (fy0 + fy1) / 2);

    let mut best: Option<((bool, i32, i32, i32), usize)> = None;
    for (i, c) in slots.iter().enumerate() {
        if i == cur {
            continue;
        }
        let (cx0, cx1) = (c.rect.x, c.rect.x + c.rect.w);
        let (cy0, cy1) = (c.rect.y, c.rect.y + c.rect.h);
        let (ccx, ccy) = ((cx0 + cx1) / 2, (cy0 + cy1) / 2);
        let h_ov = (fx1.min(cx1) - fx0.max(cx0)).max(0);
        let v_ov = (fy1.min(cy1) - fy0.max(cy0)).max(0);

        let is_bar = matches!(c.target, PickTarget::FullBefore(_) | PickTarget::FullAfter(_));
        let key = match dir {
            Dir::Up if ccy < fcy - 1 => Some((h_ov == 0, fcy - ccy, (ccx - fcx).abs(), c.rect.x)),
            Dir::Down if ccy > fcy + 1 => {
                Some((h_ov == 0, ccy - fcy, (ccx - fcx).abs(), c.rect.x))
            }
            Dir::Left if !is_bar && ccx < fcx - 1 && v_ov > 0 => {
                Some((false, fcx - ccx, (ccy - fcy).abs(), c.rect.y))
            }
            Dir::Right if !is_bar && ccx > fcx + 1 && v_ov > 0 => {
                Some((false, ccx - fcx, (ccy - fcy).abs(), c.rect.y))
            }
            _ => None,
        };
        if let Some(k) = key {
            if best.map_or(true, |(bk, _)| k < bk) {
                best = Some((k, i));
            }
        }
    }
    best.map(|(_, i)| i)
}

// ------------------------------------------------------------------ Win32 overlay

struct OverlayState {
    accent: u32,
    icon: isize,
    visible: bool,
    from: Rect,
    to: Rect,
    cur: Rect,
    start_alpha: u8,
    cur_alpha: u8,
    anim_start: Instant,
    animating: bool,
}

static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);
static DRAW_STATE: Mutex<Option<OverlayState>> = Mutex::new(None);

/// Ambil ikon window (milik window / kelas, tidak perlu `DestroyIcon`).
/// Memakai `SendMessageTimeoutW` supaya tidak pernah macet kalau app sedang sibuk.
pub unsafe fn window_icon(hwnd: HWND) -> isize {
    for kind in [ICON_BIG, ICON_SMALL2, ICON_SMALL] {
        let mut res = 0usize;
        let ok = SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            WPARAM(kind as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            50,
            Some(&mut res),
        );
        if ok.0 != 0 && res != 0 {
            return res as isize;
        }
    }
    for idx in [GCLP_HICON, GCLP_HICONSM] {
        let h = GetClassLongPtrW(hwnd, idx);
        if h != 0 {
            return h as isize;
        }
    }
    0
}

fn lerp_rect(a: Rect, b: Rect, e: f32) -> Rect {
    let f = |u: i32, v: i32| (u as f32 + (v - u) as f32 * e).round() as i32;
    Rect {
        x: f(a.x, b.x),
        y: f(a.y, b.y),
        w: f(a.w, b.w).max(1),
        h: f(a.h, b.h).max(1),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    let snap = DRAW_STATE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| (s.accent, s.icon)));
    if let Some((accent, icon_raw)) = snap {
        let pal = Palette::resolve(OverviewColors::default());
        let mut canvas = Canvas::new(hdc, client);

        // Permukaan kaca halus bernuansa aksen (gaya pratinjau Snap Windows 11)
        let tint = theme::mix(pal.surface, accent, if pal.dark { 28 } else { 20 });
        canvas.fill_rect(client, tint, 255);
        let border_r = RECT {
            left: client.left + 1,
            top: client.top + 1,
            right: (client.right - 1).max(client.left + 2),
            bottom: (client.bottom - 1).max(client.top + 2),
        };
        canvas.stroke_round_rect(border_r, 8.0, accent, 255, 2.0);

        if icon_raw != 0 {
            let cw = client.right - client.left;
            let ch = client.bottom - client.top;
            let size = (cw.min(ch) - 16).clamp(16, 44);
            if cw >= size + 16 && ch >= size + 16 {
                let pad = 10;
                let bw = size + pad * 2;
                let bh = size + pad * 2;
                let bx = (cw - bw) / 2;
                let by = (ch - bh) / 2;
                let badge = RECT {
                    left: bx,
                    top: by,
                    right: bx + bw,
                    bottom: by + bh,
                };
                canvas.fill_round_rect(badge, 10.0, pal.surface, 230);
                canvas.stroke_round_rect(badge, 10.0, pal.stroke_strong, 200, 1.0);
            }
            let x = (cw - size) / 2;
            let y = (ch - size) / 2;
            let _ = DrawIconEx(
                canvas.hdc(),
                x,
                y,
                HICON(icon_raw as *mut c_void),
                size,
                size,
                0,
                None,
                DI_NORMAL,
            );
        }
        canvas.present();
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn tick_anim(hwnd: HWND) {
    let mut step: Option<(Rect, u8, bool)> = None;
    if let Ok(mut g) = DRAW_STATE.lock() {
        if let Some(st) = g.as_mut() {
            if !st.animating {
                let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
                return;
            }
            let elapsed = st.anim_start.elapsed().as_secs_f32() * 1000.0;
            let tg = (elapsed / GLIDE_MS).min(1.0);
            let tf = (elapsed / FADE_MS).min(1.0);
            let eg = theme::ease_fluent_out(tg);
            let ef = theme::ease_fluent_out(tf);
            st.cur = lerp_rect(st.from, st.to, eg);
            st.cur_alpha = (st.start_alpha as f32
                + (ALPHA as f32 - st.start_alpha as f32) * ef)
                .round() as u8;
            let done = tg >= 1.0 && tf >= 1.0;
            if done {
                st.animating = false;
                st.cur = st.to;
                st.cur_alpha = ALPHA;
            }
            step = Some((st.cur, st.cur_alpha, done));
        }
    }
    if let Some((r, alpha, done)) = step {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            r.x,
            r.y,
            r.w.max(1),
            r.h.max(1),
            SWP_NOACTIVATE,
        );
        let _ = InvalidateRect(Some(hwnd), None, false);
        if done {
            let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
        }
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER if wparam.0 == ANIM_TIMER_ID => {
            tick_anim(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Buat jendela overlay tersembunyi (dipanggil sekali di awal `main`).
pub unsafe fn init() -> bool {
    let hinst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
    let cls = w!("VScrollWmPickOverlay");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(overlay_proc),
        hInstance: hinst,
        lpszClassName: cls,
        ..Default::default()
    };
    RegisterClassW(&wc);
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_NOACTIVATE
            | WS_EX_LAYERED
            | WS_EX_TRANSPARENT,
        cls,
        w!(""),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        Some(hinst),
        None,
    ) else {
        return false;
    };
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), ALPHA, LWA_ALPHA);
    let pal = Palette::resolve(OverviewColors::default());
    theme::apply_window_chrome(hwnd, &pal, false);
    OVERLAY_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    true
}

/// Tampilkan marker overlay di `rect` (koordinat layar) dengan warna `color` (COLORREF)
/// dan ikon app `icon` (HICON sebagai isize; 0 = tanpa ikon).
pub unsafe fn show(rect: Rect, color: u32, icon: isize) {
    let raw = OVERLAY_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut c_void);
    let pal = Palette::resolve(OverviewColors::default());
    // Kalau warna sama dengan default lama, gunakan aksen Windows aktif
    let accent = if color == OverviewColors::default().active {
        pal.accent
    } else {
        color
    };
    theme::apply_window_chrome(hwnd, &pal, false);

    let anim = animations_enabled();
    let (init_r, init_alpha, start_timer) = if let Ok(mut g) = DRAW_STATE.lock() {
        let prev = g.as_ref().filter(|s| s.visible);
        let (from, start_alpha) = match prev {
            Some(p) if anim => (p.cur, p.cur_alpha),
            _ if anim => (rect, 0),
            _ => (rect, ALPHA),
        };
        let need_anim = anim && (from != rect || start_alpha != ALPHA);
        *g = Some(OverlayState {
            accent,
            icon,
            visible: true,
            from,
            to: rect,
            cur: from,
            start_alpha,
            cur_alpha: start_alpha,
            anim_start: Instant::now(),
            animating: need_anim,
        });
        (from, start_alpha, need_anim)
    } else {
        (rect, ALPHA, false)
    };

    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), init_alpha, LWA_ALPHA);
    let _ = SetWindowPos(
        hwnd,
        Some(HWND_TOPMOST),
        init_r.x,
        init_r.y,
        init_r.w.max(1),
        init_r.h.max(1),
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    let _ = InvalidateRect(Some(hwnd), None, false);
    if start_timer {
        let _ = SetTimer(Some(hwnd), ANIM_TIMER_ID, ANIM_TICK_MS, None);
    } else {
        let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
    }
}

/// Sembunyikan marker overlay.
pub unsafe fn hide() {
    let raw = OVERLAY_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut c_void);
    let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
    if let Ok(mut g) = DRAW_STATE.lock() {
        if let Some(st) = g.as_mut() {
            st.visible = false;
            st.animating = false;
        }
    }
    let _ = ShowWindow(hwnd, SW_HIDE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::{OverviewItem, Strip};

    const A: WindowId = WindowId(1);
    const B: WindowId = WindowId(2);
    const C: WindowId = WindowId(3);

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn vertical_navigation_visits_gaps_between_windows_and_inside_windows() {
        // Strip berisi A, B, C penuh lebar; kita angkat C.
        let mut s = Strip::new(rect(0, 0, 1920, 1080));
        s.add_window(A);
        s.add_window(B);
        s.add_window(C);
        let slots = build_slots(&s.overview(), C);

        // Mulai dari paling atas: FullBefore(A) -> Split(A, left) -> FullAfter(A) (antara A & B)
        // -> Split(B, left) -> FullAfter(B)
        let mut i = 0;
        assert_eq!(slots[i].target, PickTarget::FullBefore(A));
        i = step_slot(&slots, i, Dir::Down).unwrap();
        assert_eq!(slots[i].target, PickTarget::Split(A, true));
        // Kanan -> Split(A, right), Kiri -> balik ke Split(A, left)
        let right = step_slot(&slots, i, Dir::Right).unwrap();
        assert_eq!(slots[right].target, PickTarget::Split(A, false));
        assert_eq!(step_slot(&slots, right, Dir::Left), Some(i));

        i = step_slot(&slots, i, Dir::Down).unwrap();
        assert_eq!(slots[i].target, PickTarget::FullAfter(A));
        i = step_slot(&slots, i, Dir::Down).unwrap();
        assert_eq!(slots[i].target, PickTarget::Split(B, true));
        i = step_slot(&slots, i, Dir::Down).unwrap();
        assert_eq!(slots[i].target, PickTarget::FullAfter(B));
        assert_eq!(step_slot(&slots, i, Dir::Down), None);
    }

    #[test]
    fn lone_half_width_window_offers_beside_slot() {
        let ov = Overview {
            items: vec![OverviewItem {
                id: A,
                rect: rect(0, 0, 960, 1080),
            }],
            width: 1920,
            height: 1080,
            view_y: 0,
            view_h: 1080,
        };
        let slots = build_slots(&ov, B);
        assert!(slots.iter().any(|s| s.target == PickTarget::Beside(A)));
        let top_a = slots
            .iter()
            .position(|s| s.target == PickTarget::Split(A, true))
            .unwrap();
        let right = step_slot(&slots, top_a, Dir::Right).unwrap();
        assert_eq!(slots[right].target, PickTarget::Beside(A));
    }

    #[test]
    fn empty_workspace_produces_single_empty_slot() {
        let ov = Overview {
            items: Vec::new(),
            width: 1920,
            height: 1080,
            view_y: 0,
            view_h: 1080,
        };
        let slots = build_slots(&ov, A);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].target, PickTarget::EmptyWorkspace);
    }
}
