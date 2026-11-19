//! OSD workspace: panel flyout ringkas khas Windows 11 di bagian bawah layar yang
//! muncul sebentar setiap pindah workspace ("Workspace 2" + indikator pil membulat),
//! meluncur & memudar masuk/keluar dengan kurva Fluent Motion.
//!
//! Tidak pernah merebut fokus dan tidak menangkap klik. Mengikuti tema gelap/terang
//! & warna aksen Windows 11 (atau warna kustom di konfigurasi). Dimatikan lewat
//! `workspace_osd = off`.

use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use layout::Rect;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    HFONT, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::OverviewColors;
use crate::theme::{self, Canvas, Palette};
use crate::winutil::animations_enabled;

/// Berapa lama panel bertahan tampil penuh (ms).
const SHOW_MS: u32 = 900;
const FADE_IN_MS: f32 = 140.0;
const FADE_OUT_MS: f32 = 180.0;
const MAX_ALPHA: u8 = 244;
const SLIDE_IN_PX: i32 = 12;
const SLIDE_OUT_PX: i32 = 8;

const HOLD_TIMER_ID: usize = 1;
const ANIM_TIMER_ID: usize = 2;
const ANIM_TICK_MS: u32 = 16;

const PANEL_H_DOTS: i32 = 82;
const PANEL_H_COMPACT: i32 = 54;
/// Jarak panel dari tepi bawah area kerja (piksel).
const BOTTOM_MARGIN: i32 = 44;
const MIN_PANEL_W: i32 = 224;
const COMPACT_PANEL_W: i32 = 196;
const PILL: i32 = 22;
const DOT: i32 = 7;
const PILL_GAP: i32 = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Hidden,
    Entering,
    Holding,
    Exiting,
}

struct OsdState {
    active: usize,
    total: usize,
    colors: OverviewColors,
    show_dots: bool,
    phase: Phase,
    anim_start: Instant,
    start_alpha: u8,
    target_rect: Rect,
}

static OSD_HWND: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);
/// Workspace yang terakhir diumumkan; panel hanya muncul kalau berubah.
static LAST: AtomicUsize = AtomicUsize::new(0);
static STATE: Mutex<Option<OsdState>> = Mutex::new(None);

/// Lebar panel untuk `total` workspace.
pub fn panel_width(total: usize) -> i32 {
    let row = total as i32 * (PILL + PILL_GAP);
    (row + 56).max(MIN_PANEL_W)
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    let snapshot = STATE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| (s.active, s.total, s.colors, s.show_dots)));
    if let Some((active, total, colors, show_dots)) = snapshot {
        let pal = Palette::resolve(colors);
        let mut canvas = Canvas::new(hdc, client);
        canvas.fill_rect(client, pal.surface, 255);

        let font = HFONT(FONT.load(Ordering::Relaxed) as *mut _);
        let label = format!("Workspace {}", active + 1);
        let title_r = if show_dots {
            RECT {
                left: 12,
                top: 12,
                right: client.right - 12,
                bottom: 46,
            }
        } else {
            RECT {
                left: 12,
                top: 0,
                right: client.right - 12,
                bottom: client.bottom,
            }
        };
        canvas.draw_text(
            &label,
            title_r,
            font,
            pal.text,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        if show_dots {
            // Indikator halaman ala Windows 11: pil memanjang untuk workspace aktif, titik halus untuk lainnya
            let n = total as i32;
            let row_w = if n > 0 {
                PILL + (n - 1) * DOT + (n - 1) * PILL_GAP
            } else {
                0
            };
            let mut x = (client.right - row_w) / 2;
            let cy = 58;
            for i in 0..n {
                if i as usize == active {
                    let r = RECT {
                        left: x,
                        top: cy - DOT / 2,
                        right: x + PILL,
                        bottom: cy - DOT / 2 + DOT,
                    };
                    canvas.fill_round_rect(r, (DOT as f32) * 0.5, pal.accent, 255);
                    x += PILL + PILL_GAP;
                } else {
                    let r = RECT {
                        left: x,
                        top: cy - DOT / 2,
                        right: x + DOT,
                        bottom: cy - DOT / 2 + DOT,
                    };
                    canvas.fill_ellipse(r, pal.stroke_strong, 210);
                    x += DOT + PILL_GAP;
                }
            }
        }
        canvas.present();
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn tick_anim(hwnd: HWND) {
    let mut action: Option<(u8, i32, i32, i32, i32, bool, bool)> = None;
    if let Ok(mut g) = STATE.lock() {
        if let Some(st) = g.as_mut() {
            let elapsed = st.anim_start.elapsed().as_secs_f32() * 1000.0;
            let tr = st.target_rect;
            match st.phase {
                Phase::Entering => {
                    let t = (elapsed / FADE_IN_MS).min(1.0);
                    let e = theme::ease_fluent_out(t);
                    let alpha = (st.start_alpha as f32
                        + (MAX_ALPHA as f32 - st.start_alpha as f32) * e)
                        .round() as u8;
                    let y = tr.y + ((SLIDE_IN_PX as f32) * (1.0 - e)).round() as i32;
                    let done = t >= 1.0;
                    if done {
                        st.phase = Phase::Holding;
                    }
                    action = Some((alpha, tr.x, y, tr.w, tr.h, done, false));
                }
                Phase::Exiting => {
                    let t = (elapsed / FADE_OUT_MS).min(1.0);
                    let e = theme::ease_fluent_in(t);
                    let alpha = ((MAX_ALPHA as f32) * (1.0 - e)).round() as u8;
                    let y = tr.y + ((SLIDE_OUT_PX as f32) * e).round() as i32;
                    let done = t >= 1.0;
                    if done {
                        st.phase = Phase::Hidden;
                    }
                    action = Some((alpha, tr.x, y, tr.w, tr.h, done, done));
                }
                _ => {
                    let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
                }
            }
        }
    }
    if let Some((alpha, x, y, w, h, stop_anim, hide_win)) = action {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOSIZE,
        );
        if stop_anim {
            let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
        }
        if hide_win {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

unsafe extern "system" fn osd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            if wparam.0 == HOLD_TIMER_ID {
                let _ = KillTimer(Some(hwnd), HOLD_TIMER_ID);
                if animations_enabled() {
                    if let Ok(mut g) = STATE.lock() {
                        if let Some(st) = g.as_mut() {
                            st.phase = Phase::Exiting;
                            st.anim_start = Instant::now();
                        }
                    }
                    let _ = SetTimer(Some(hwnd), ANIM_TIMER_ID, ANIM_TICK_MS, None);
                } else {
                    if let Ok(mut g) = STATE.lock() {
                        if let Some(st) = g.as_mut() {
                            st.phase = Phase::Hidden;
                        }
                    }
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            } else if wparam.0 == ANIM_TIMER_ID {
                tick_anim(hwnd);
            }
            LRESULT(0)
        }
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT: klik menembus panel
        WM_MOUSEACTIVATE => LRESULT(3), // MA_NOACTIVATE
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Buat jendela panel (tersembunyi) sekali di awal.
pub unsafe fn init() -> bool {
    let Ok(hmod) = GetModuleHandleW(None) else {
        return false;
    };
    let hinst = HINSTANCE(hmod.0);
    let class = w!("VScrollWmOsd");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(osd_proc),
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
        w!("vscroll-wm workspace"),
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
    OSD_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

    let pal = Palette::resolve(OverviewColors::default());
    theme::apply_window_chrome(hwnd, &pal, false);
    FONT.store(theme::create_font(-22, 600).0 as isize, Ordering::Relaxed);
    true
}

/// Umumkan workspace aktif. Tidak melakukan apa-apa kalau workspace-nya sama dengan
/// yang terakhir diumumkan. `total` = jumlah workspace yang ditampilkan sebagai indikator
/// kalau `show_dots` aktif.
pub unsafe fn notify(
    active: usize,
    total: usize,
    colors: OverviewColors,
    work: Rect,
    show_dots: bool,
) {
    let hw = OSD_HWND.load(Ordering::Relaxed);
    if hw == 0 || LAST.swap(active, Ordering::Relaxed) == active {
        return;
    }
    let h = HWND(hw as *mut _);
    let total = total.max(active + 1);
    let w = if show_dots {
        panel_width(total)
    } else {
        COMPACT_PANEL_W
    };
    let panel_h = if show_dots {
        PANEL_H_DOTS
    } else {
        PANEL_H_COMPACT
    };
    let x = work.x + (work.w - w) / 2;
    let y = work.y + work.h - panel_h - BOTTOM_MARGIN;
    let target = Rect {
        x,
        y,
        w,
        h: panel_h,
    };

    let pal = Palette::resolve(colors);
    theme::apply_window_chrome(h, &pal, false);

    let anim = animations_enabled();
    let (init_alpha, init_y, start_anim) = if let Ok(mut g) = STATE.lock() {
        let already_visible = g
            .as_ref()
            .map_or(false, |s| s.phase == Phase::Holding || s.phase == Phase::Entering);
        let phase = if !anim || already_visible {
            Phase::Holding
        } else {
            Phase::Entering
        };
        let start_alpha = if already_visible || !anim {
            MAX_ALPHA
        } else {
            0
        };
        *g = Some(OsdState {
            active,
            total,
            colors,
            show_dots,
            phase,
            anim_start: Instant::now(),
            start_alpha,
            target_rect: target,
        });
        let sy = if phase == Phase::Entering {
            y + SLIDE_IN_PX
        } else {
            y
        };
        (start_alpha, sy, phase == Phase::Entering)
    } else {
        (MAX_ALPHA, y, false)
    };

    let _ = SetLayeredWindowAttributes(h, COLORREF(0), init_alpha, LWA_ALPHA);
    let _ = SetWindowPos(
        h,
        Some(HWND_TOPMOST),
        x,
        init_y,
        w,
        panel_h,
        SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );
    let _ = InvalidateRect(Some(h), None, false);
    if start_anim {
        let _ = SetTimer(Some(h), ANIM_TIMER_ID, ANIM_TICK_MS, None);
    } else {
        let _ = KillTimer(Some(h), ANIM_TIMER_ID);
    }
    let _ = SetTimer(Some(h), HOLD_TIMER_ID, SHOW_MS, None);
}

/// Sembunyikan segera (mis. saat dijeda).
pub unsafe fn hide() {
    let hw = OSD_HWND.load(Ordering::Relaxed);
    if hw != 0 {
        let h = HWND(hw as *mut _);
        let _ = KillTimer(Some(h), HOLD_TIMER_ID);
        let _ = KillTimer(Some(h), ANIM_TIMER_ID);
        if let Ok(mut g) = STATE.lock() {
            if let Some(st) = g.as_mut() {
                st.phase = Phase::Hidden;
            }
        }
        let _ = ShowWindow(h, SW_HIDE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_grows_with_the_workspace_count_but_has_a_minimum() {
        assert_eq!(panel_width(1), MIN_PANEL_W);
        assert!(panel_width(9) > panel_width(5));
        assert!(panel_width(9) >= 9 * (PILL + PILL_GAP));
    }
}
