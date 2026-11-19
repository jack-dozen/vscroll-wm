//! Menu snap layouts: panel preset di tengah tepi atas layar yang muncul selama window
//! diseret ke atas (seperti Snap Layouts di Windows 11). Tiap preset digambar sebagai
//! kotak kecil berisi zonanya; zona di bawah kursor disorot. Window yang dilepas di
//! sebuah zona mengisi zona itu (lihat app/snap.rs).
//!
//! Selama seretan, kursor ditangkap oleh window yang sedang diseret, jadi panel ini
//! tidak menerima mouse sama sekali: letak kursor dibaca oleh app dan dicocokkan ke
//! `MenuLayout` lewat `hit` / `inside`. Panel tidak pernah merebut fokus. Geometri
//! (`layout_menu`, `hit`, `inside`) murni hitungan dan dipakai bersama oleh penggambar
//! dan pencocok klik, jadi yang tampak selalu sama dengan yang dikenai.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use layout::{Rect, SnapPreset};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, HFONT, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::OverviewColors;
use crate::theme::{self, Canvas, Palette};
use crate::winutil::animations_enabled;

/// Ukuran dasar (piksel pada 96 dpi); dikalikan skala dpi layar.
const THUMB_W: i32 = 128;
const THUMB_H: i32 = 80;
const LABEL_H: i32 = 24;
const PAD: i32 = 14;
const GAP: i32 = 12;
/// Jarak panel dari tepi atas area kerja.
const TOP_MARGIN: i32 = 8;
/// Jarak isi kotak preset dari tepinya.
const INSET: i32 = 6;
/// Jarak antar zona di dalam satu kotak preset.
const ZONE_GAP: i32 = 4;

/// Satu zona di menu: `cell` = daerah yang dikenai kursor, `draw` = yang digambar
/// (sedikit lebih kecil supaya batas antar zona terlihat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneBox {
    pub cell: Rect,
    pub draw: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    /// Kotak preset (koordinat layar).
    pub rect: Rect,
    /// Tempat nama preset.
    pub label: Rect,
    pub zones: Vec<ZoneBox>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuLayout {
    /// Panel seluruhnya (koordinat layar).
    pub panel: Rect,
    pub thumbs: Vec<Thumb>,
}

fn contains(r: &Rect, x: i32, y: i32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// Susun panel untuk `presets` di tengah tepi atas `work`. `scale` = dpi / 96.
pub fn layout_menu(work: Rect, presets: &[SnapPreset], scale: f32) -> MenuLayout {
    let s = |v: i32| ((v as f32) * scale).round() as i32;
    let (tw, th, lh, pad, gap) = (s(THUMB_W), s(THUMB_H), s(LABEL_H), s(PAD), s(GAP));
    let (inset, zgap) = (s(INSET), s(ZONE_GAP));
    let n = presets.len() as i32;
    if n == 0 {
        return MenuLayout {
            panel: Rect {
                x: work.x,
                y: work.y,
                w: 0,
                h: 0,
            },
            thumbs: Vec::new(),
        };
    }
    // sebanyak mungkin kotak per baris dalam 90% lebar layar
    let max_w = work.w * 9 / 10;
    let cols = ((max_w - 2 * pad + gap) / (tw + gap)).clamp(1, n);
    let rows = (n + cols - 1) / cols;
    let panel_w = cols * tw + (cols - 1) * gap + 2 * pad;
    let panel_h = rows * (th + lh) + (rows - 1) * gap + 2 * pad;
    let panel = Rect {
        x: work.x + (work.w - panel_w) / 2,
        y: work.y + s(TOP_MARGIN),
        w: panel_w,
        h: panel_h,
    };
    let half = zgap / 2;
    let thumbs = presets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (col, row) = (i as i32 % cols, i as i32 / cols);
            let x = panel.x + pad + col * (tw + gap);
            let y = panel.y + pad + row * (th + lh + gap);
            let rect = Rect { x, y, w: tw, h: th };
            let label = Rect {
                x,
                y: y + th,
                w: tw,
                h: lh,
            };
            let zones = p
                .preview((tw - 2 * inset).max(1), (th - 2 * inset).max(1))
                .into_iter()
                .map(|r| {
                    let cell = Rect {
                        x: x + inset + r.x,
                        y: y + inset + r.y,
                        w: r.w,
                        h: r.h,
                    };
                    let draw = Rect {
                        x: cell.x + half,
                        y: cell.y + half,
                        w: (cell.w - 2 * half).max(1),
                        h: (cell.h - 2 * half).max(1),
                    };
                    ZoneBox { cell, draw }
                })
                .collect();
            Thumb { rect, label, zones }
        })
        .collect();
    MenuLayout { panel, thumbs }
}

/// Zona di titik (x, y): (nomor preset, nomor zona).
pub fn hit(l: &MenuLayout, x: i32, y: i32) -> Option<(usize, usize)> {
    if !contains(&l.panel, x, y) {
        return None;
    }
    for (pi, t) in l.thumbs.iter().enumerate() {
        for (zi, z) in t.zones.iter().enumerate() {
            if contains(&z.cell, x, y) {
                return Some((pi, zi));
            }
        }
    }
    None
}

/// Titik (x, y) di dalam panel yang diperlebar `margin` piksel?
pub fn inside(l: &MenuLayout, x: i32, y: i32, margin: i32) -> bool {
    let p = l.panel;
    x >= p.x - margin && x < p.x + p.w + margin && y >= p.y - margin && y < p.y + p.h + margin
}

const ANIM_TIMER_ID: usize = 1;
const ANIM_TICK_MS: u32 = 16;
const FADE_IN_MS: f32 = 150.0;
const FADE_OUT_MS: f32 = 110.0;
const MAX_ALPHA: u8 = 246;
const SLIDE_PX: i32 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Entering,
    Visible,
    Exiting,
}

struct State {
    layout: MenuLayout,
    names: Vec<String>,
    hover: Option<(usize, usize)>,
    colors: OverviewColors,
    phase: Phase,
    anim_start: Instant,
    start_alpha: u8,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static MENU_HWND: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);

fn local(r: Rect, origin: Rect) -> RECT {
    RECT {
        left: r.x - origin.x,
        top: r.y - origin.y,
        right: r.x - origin.x + r.w,
        bottom: r.y - origin.y + r.h,
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    if let Ok(g) = STATE.lock() {
        if let Some(st) = g.as_ref() {
            let pal = Palette::resolve(st.colors);
            let mut canvas = Canvas::new(hdc, client);
            canvas.fill_rect(client, pal.surface, 255);
            let font = HFONT(FONT.load(Ordering::Relaxed) as *mut _);

            let origin = st.layout.panel;
            for (pi, t) in st.layout.thumbs.iter().enumerate() {
                let r = local(t.rect, origin);
                let hovered_group = st.hover.map_or(false, |(hp, _)| hp == pi);
                canvas.fill_round_rect(
                    r,
                    8.0,
                    if hovered_group { pal.card_hover } else { pal.layer },
                    255,
                );
                canvas.stroke_round_rect(
                    r,
                    8.0,
                    if hovered_group { pal.accent } else { pal.stroke },
                    if hovered_group { 210 } else { 255 },
                    if hovered_group { 1.5 } else { 1.0 },
                );

                for (zi, z) in t.zones.iter().enumerate() {
                    let zr = local(z.draw, origin);
                    let on = st.hover == Some((pi, zi));
                    if on {
                        canvas.fill_round_rect(zr, 4.0, pal.accent, 255);
                    } else {
                        canvas.fill_round_rect(zr, 4.0, pal.control, 255);
                        canvas.stroke_round_rect(zr, 4.0, pal.stroke_strong, 180, 1.0);
                    }
                }
                let name = st.names.get(pi).map_or("", |n| n.as_str());
                let lr = local(t.label, origin);
                canvas.draw_text(
                    name,
                    lr,
                    font,
                    if hovered_group {
                        pal.text
                    } else {
                        pal.text_secondary
                    },
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
            canvas.present();
        }
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn tick_anim(hwnd: HWND) {
    let mut action: Option<(u8, i32, i32, bool, bool)> = None;
    if let Ok(mut g) = STATE.lock() {
        if let Some(st) = g.as_mut() {
            let elapsed = st.anim_start.elapsed().as_secs_f32() * 1000.0;
            let p = st.layout.panel;
            match st.phase {
                Phase::Entering => {
                    let t = (elapsed / FADE_IN_MS).min(1.0);
                    let e = theme::ease_fluent_out(t);
                    let alpha = (st.start_alpha as f32
                        + (MAX_ALPHA as f32 - st.start_alpha as f32) * e)
                        .round() as u8;
                    let y = p.y - ((SLIDE_PX as f32) * (1.0 - e)).round() as i32;
                    let done = t >= 1.0;
                    if done {
                        st.phase = Phase::Visible;
                    }
                    action = Some((alpha, p.x, y, done, false));
                }
                Phase::Exiting => {
                    let t = (elapsed / FADE_OUT_MS).min(1.0);
                    let e = theme::ease_fluent_in(t);
                    let alpha = ((st.start_alpha as f32) * (1.0 - e)).round() as u8;
                    let y = p.y - ((SLIDE_PX as f32 * 0.75) * e).round() as i32;
                    let done = t >= 1.0;
                    action = Some((alpha, p.x, y, done, done));
                }
                Phase::Visible => {
                    let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
                }
            }
        }
        if let Some((_, _, _, _, true)) = action {
            *g = None;
        }
    }
    if let Some((alpha, x, y, stop, hide_win)) = action {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE,
        );
        if stop {
            let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
        }
        if hide_win {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

unsafe extern "system" fn menu_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT: klik menembus panel
        WM_MOUSEACTIVATE => LRESULT(3), // MA_NOACTIVATE
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Buat jendela panel (tersembunyi) sekali di awal. `scale` = dpi / 96 untuk huruf.
pub unsafe fn init(scale: f32) -> bool {
    let Ok(hmod) = GetModuleHandleW(None) else {
        return false;
    };
    let hinst = HINSTANCE(hmod.0);
    let class = w!("VScrollWmSnapMenu");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(menu_proc),
        hInstance: hinst,
        lpszClassName: class,
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return false;
    }
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
        class,
        w!("vscroll-wm snap layouts"),
        WS_POPUP,
        0,
        0,
        100,
        100,
        None,
        None,
        Some(hinst),
        None,
    ) else {
        return false;
    };
    MENU_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

    let pal = Palette::resolve(OverviewColors::default());
    theme::apply_window_chrome(hwnd, &pal, false);
    let fh = -((13.0 * scale).round() as i32);
    FONT.store(theme::create_font(fh, 500).0 as isize, Ordering::Relaxed);
    true
}

fn hwnd() -> HWND {
    HWND(MENU_HWND.load(Ordering::Relaxed) as *mut _)
}

/// Tampilkan panel `layout` dengan nama-nama preset `names`.
pub unsafe fn show(layout: MenuLayout, names: Vec<String>, colors: OverviewColors) {
    if MENU_HWND.load(Ordering::Relaxed) == 0 {
        return;
    }
    let h = hwnd();
    let p = layout.panel;
    let pal = Palette::resolve(colors);
    theme::apply_window_chrome(h, &pal, false);

    let anim = animations_enabled();
    let (init_alpha, init_y, start_timer) = if anim {
        (0u8, p.y - SLIDE_PX, true)
    } else {
        (MAX_ALPHA, p.y, false)
    };
    if let Ok(mut g) = STATE.lock() {
        *g = Some(State {
            layout,
            names,
            hover: None,
            colors,
            phase: if anim { Phase::Entering } else { Phase::Visible },
            anim_start: Instant::now(),
            start_alpha: init_alpha,
        });
    }
    let _ = SetLayeredWindowAttributes(h, COLORREF(0), init_alpha, LWA_ALPHA);
    let _ = SetWindowPos(
        h,
        Some(HWND_TOPMOST),
        p.x,
        init_y,
        p.w,
        p.h,
        SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );
    let _ = InvalidateRect(Some(h), None, false);
    if start_timer {
        let _ = SetTimer(Some(h), ANIM_TIMER_ID, ANIM_TICK_MS, None);
    } else {
        let _ = KillTimer(Some(h), ANIM_TIMER_ID);
    }
}

/// Sorot zona (nomor preset, nomor zona) di bawah kursor; None = tidak ada.
pub unsafe fn set_hover(hover: Option<(usize, usize)>) {
    if let Ok(mut g) = STATE.lock() {
        if let Some(st) = g.as_mut() {
            if st.hover == hover {
                return;
            }
            st.hover = hover;
        }
    }
    if MENU_HWND.load(Ordering::Relaxed) != 0 {
        let _ = InvalidateRect(Some(hwnd()), None, false);
    }
}

pub unsafe fn hide() {
    if MENU_HWND.load(Ordering::Relaxed) == 0 {
        return;
    }
    let h = hwnd();
    if animations_enabled() {
        let start_exit = if let Ok(mut g) = STATE.lock() {
            if let Some(st) = g.as_mut() {
                if st.phase != Phase::Exiting {
                    st.phase = Phase::Exiting;
                    st.anim_start = Instant::now();
                    st.start_alpha = MAX_ALPHA;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if start_exit {
            let _ = SetTimer(Some(h), ANIM_TIMER_ID, ANIM_TICK_MS, None);
            return;
        }
    }
    let _ = KillTimer(Some(h), ANIM_TIMER_ID);
    let _ = ShowWindow(h, SW_HIDE);
    if let Ok(mut g) = STATE.lock() {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1040,
        }
    }

    fn presets() -> Vec<SnapPreset> {
        layout::builtin_presets()
    }

    #[test]
    fn the_panel_is_centered_at_the_top_and_inside_the_screen() {
        let l = layout_menu(work(), &presets(), 1.0);
        let p = l.panel;
        assert_eq!(p.y, TOP_MARGIN);
        assert_eq!(p.x * 2 + p.w, 1920); // simetris
        assert!(p.x >= 0 && p.x + p.w <= 1920);
        assert_eq!(l.thumbs.len(), presets().len());
    }

    #[test]
    fn many_presets_wrap_into_rows_on_a_narrow_screen() {
        let narrow = Rect {
            x: 0,
            y: 0,
            w: 500,
            h: 800,
        };
        let l = layout_menu(narrow, &presets(), 1.0);
        assert!(l.panel.w <= 500);
        let rows: std::collections::BTreeSet<i32> = l.thumbs.iter().map(|t| t.rect.y).collect();
        assert!(rows.len() > 1);
        // tidak ada kotak yang keluar dari panel
        for t in &l.thumbs {
            assert!(t.rect.x >= l.panel.x && t.rect.x + t.rect.w <= l.panel.x + l.panel.w);
            assert!(t.label.y + t.label.h <= l.panel.y + l.panel.h);
        }
    }

    #[test]
    fn dpi_scale_grows_the_panel() {
        let a = layout_menu(work(), &presets(), 1.0).panel;
        let b = layout_menu(work(), &presets(), 2.0).panel;
        assert!(b.h > a.h);
    }

    #[test]
    fn hit_finds_the_zone_under_the_cursor() {
        let ps = presets();
        let l = layout_menu(work(), &ps, 1.0);
        for (pi, t) in l.thumbs.iter().enumerate() {
            for (zi, z) in t.zones.iter().enumerate() {
                let (cx, cy) = (z.draw.x + z.draw.w / 2, z.draw.y + z.draw.h / 2);
                assert_eq!(hit(&l, cx, cy), Some((pi, zi)), "{} zona {zi}", ps[pi].name);
            }
        }
        // di luar panel
        assert_eq!(hit(&l, 0, 1000), None);
        assert_eq!(hit(&l, l.panel.x - 5, l.panel.y + 5), None);
    }

    #[test]
    fn halves_has_two_side_by_side_zones() {
        let ps = vec![SnapPreset::parse("halves", "L, R").unwrap()];
        let l = layout_menu(work(), &ps, 1.0);
        let z = &l.thumbs[0].zones;
        assert_eq!(z.len(), 2);
        assert!(z[0].cell.x + z[0].cell.w <= z[1].cell.x + 1);
        assert_eq!(z[0].cell.y, z[1].cell.y);
    }

    #[test]
    fn inside_includes_the_margin() {
        let l = layout_menu(work(), &presets(), 1.0);
        let p = l.panel;
        assert!(inside(&l, p.x + 1, p.y + 1, 0));
        assert!(!inside(&l, p.x - 3, p.y + 1, 0));
        assert!(inside(&l, p.x - 3, p.y + 1, 10));
    }

    #[test]
    fn no_presets_no_panel() {
        let l = layout_menu(work(), &[], 1.0);
        assert!(l.thumbs.is_empty());
        assert_eq!(hit(&l, 10, 10), None);
    }
}
