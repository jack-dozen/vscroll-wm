//! Window list for empty snap layout zones (Snap Assist): one panel above each
//! empty zone, containing cards for all open windows (DWM thumbnail + title).
//! Clicking a card places that window in the zone.
//!
//! The panel never takes focus (WS_EX_NOACTIVATE): clicks reach it, but the
//! active window does not change. Esc and outside clicks are handled by the app
//! (keyboard hook / focus tracker). The renderer only forwards clicks as messages
//! to the main thread: `WM_SNAP_PICK` (card) and `WM_SNAP_CANCEL` (close button / right-click).
//!
//! Several panels are created once at startup (one per maximum zone count) and reused.

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use layout::Rect;
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY,
    DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HFONT, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::OverviewColors;
use crate::consts::{WM_SNAP_CANCEL, WM_SNAP_PICK};
use crate::theme::{self, Canvas, Palette};
use crate::winutil::animations_enabled;

/// Number of prepared panels = the largest zone count in any preset.
const POOL: usize = layout::MAX_ZONES;
const PAD: i32 = 12;
const CARD_GAP: i32 = 10;
const LABEL_BAND: i32 = 26;
const MIN_CARD_W: i32 = 70;
const MIN_CARD_H: i32 = 54;

/// Empty zone needing a window list: `slot` = slot ID in the strip, `rect` = its
/// screen position.
#[derive(Debug, Clone, Copy)]
pub struct SlotView {
    pub slot: u64,
    pub rect: Rect,
}

/// A selectable window.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: u64,
    pub label: String,
    /// Original window HWND (for the thumbnail); 0 = no thumbnail.
    pub source: isize,
}

struct Card {
    id: u64,
    label: String,
    rect: RECT,
    /// Label band below the thumbnail; None = no thumbnail (label centered in the card).
    band: Option<RECT>,
}

const ANIM_TIMER_ID: usize = 1;
const ANIM_TICK_MS: u32 = 16;
const FADE_IN_MS: f32 = 130.0;

struct Panel {
    hwnd: isize,
    slot: u64,
    cards: Vec<Card>,
    hover: Option<usize>,
    thumbs: Vec<isize>,
    anim_start: Option<Instant>,
}

static HANDLES: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static PANELS: Mutex<Vec<Panel>> = Mutex::new(Vec::new());
static COLORS: Mutex<OverviewColors> = Mutex::new(OverviewColors {
    selected: 0,
    active: 0,
    view: 0,
    background: 0,
    box_fill: 0,
});
static FONT: AtomicIsize = AtomicIsize::new(0);
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

/// Arrange `n` cards in a `w` x `h` panel (panel-relative coordinates): choose the
/// number of rows and columns to make the cards as large as possible. If they cannot
/// all fit at the minimum size, place only those that fit (fewer than `n` may fit).
pub(crate) fn card_grid(n: usize, w: i32, h: i32) -> Vec<Rect> {
    let aw = w - 2 * PAD;
    let ah = h - 2 * PAD;
    if n == 0 || aw < 1 || ah < 1 {
        return Vec::new();
    }
    for count in (1..=n as i32).rev() {
        let mut best: Option<(i32, i32, i32, i32)> = None; // (score, columns, width, height)
        for cols in 1..=count {
            let rows = (count + cols - 1) / cols;
            let cw = (aw - (cols - 1) * CARD_GAP) / cols;
            let ch = (ah - (rows - 1) * CARD_GAP) / rows;
            if cw < MIN_CARD_W || ch < MIN_CARD_H {
                continue;
            }
            // Thumbnails have an approximate 4:3 aspect ratio: score by the shortest side.
            let score = cw.min(ch * 4 / 3);
            if best.map_or(true, |b| score > b.0) {
                best = Some((score, cols, cw, ch));
            }
        }
        if let Some((_, cols, cw, ch)) = best {
            return (0..count)
                .map(|k| {
                    let (c, r) = (k % cols, k / cols);
                    Rect {
                        x: PAD + c * (cw + CARD_GAP),
                        y: PAD + r * (ch + CARD_GAP),
                        w: cw,
                        h: ch,
                    }
                })
                .collect();
        }
    }
    // Panel is too small for regular cards: let one card fill the available space.
    vec![Rect {
        x: PAD,
        y: PAD,
        w: aw,
        h: ah,
    }]
}

fn point_in(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn lparam_xy(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

/// Fit a source of size (sw x sh) into `area`, preserving its aspect ratio and centering it.
fn fit(area: RECT, sw: i32, sh: i32) -> RECT {
    let (aw, ah) = (area.right - area.left, area.bottom - area.top);
    if sw <= 0 || sh <= 0 || aw <= 0 || ah <= 0 {
        return area;
    }
    let scale = (aw as f32 / sw as f32).min(ah as f32 / sh as f32);
    let w = ((sw as f32 * scale) as i32).max(1);
    let h = ((sh as f32 * scale) as i32).max(1);
    let l = area.left + (aw - w) / 2;
    let t = area.top + (ah - h) / 2;
    RECT {
        left: l,
        top: t,
        right: l + w,
        bottom: t + h,
    }
}

/// Attach the DWM thumbnail for `source` to card `card`. Returns (thumbnail handle,
/// label band) if the card is large enough.
unsafe fn register_thumb(
    dest: HWND,
    source: isize,
    card: &RECT,
    init_opacity: u8,
) -> Option<(isize, RECT)> {
    let inner = RECT {
        left: card.left + 4,
        top: card.top + 4,
        right: card.right - 4,
        bottom: card.bottom - 4,
    };
    let area = RECT {
        left: inner.left + 4,
        top: inner.top + 4,
        right: inner.right - 4,
        bottom: inner.bottom - LABEL_BAND,
    };
    if area.right - area.left < 40 || area.bottom - area.top < 30 {
        return None; // Too small: use text label only.
    }
    let thumb = DwmRegisterThumbnail(dest, HWND(source as *mut _)).ok()?;
    let size = DwmQueryThumbnailSourceSize(thumb).unwrap_or(SIZE { cx: 0, cy: 0 });
    let props = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
        rcDestination: fit(area, size.cx, size.cy),
        rcSource: RECT::default(),
        opacity: init_opacity,
        fVisible: BOOL(1),
        fSourceClientAreaOnly: BOOL(0),
    };
    if DwmUpdateThumbnailProperties(thumb, &props).is_err() {
        let _ = DwmUnregisterThumbnail(thumb);
        return None;
    }
    Some((
        thumb,
        RECT {
            left: inner.left,
            top: inner.bottom - LABEL_BAND,
            right: inner.right,
            bottom: inner.bottom,
        },
    ))
}

unsafe fn post(msg: u32, wparam: usize, lparam: isize) {
    let _ = PostThreadMessageW(
        MAIN_THREAD.load(Ordering::Relaxed),
        msg,
        WPARAM(wparam),
        LPARAM(lparam),
    );
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let colors = COLORS.lock().map(|c| *c).unwrap_or_default();

    if let Ok(g) = PANELS.lock() {
        if let Some(p) = g.iter().find(|p| p.hwnd == hwnd.0 as isize) {
            let pal = Palette::resolve(colors);
            let mut canvas = Canvas::new(hdc, client);
            canvas.fill_rect(client, pal.surface, 255);
            let font = HFONT(FONT.load(Ordering::Relaxed) as *mut _);

            for (i, c) in p.cards.iter().enumerate() {
                let hot = p.hover == Some(i);
                let fill = if hot {
                    theme::mix(pal.card_hover, pal.accent, 12)
                } else {
                    pal.card
                };
                canvas.fill_round_rect(c.rect, 8.0, fill, 255);
                if hot {
                    canvas.stroke_round_rect(c.rect, 8.0, pal.accent, 255, 2.0);
                } else {
                    canvas.stroke_round_rect(c.rect, 8.0, pal.stroke, 255, 1.0);
                }
                match c.band {
                    Some(band) => {
                        let r = RECT {
                            left: band.left + 8,
                            top: band.top,
                            right: band.right - 8,
                            bottom: band.bottom,
                        };
                        canvas.draw_text(
                            &c.label,
                            r,
                            font,
                            pal.text,
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                        );
                    }
                    None => {
                        let r = RECT {
                            left: c.rect.left + 10,
                            top: c.rect.top,
                            right: c.rect.right - 10,
                            bottom: c.rect.bottom,
                        };
                        canvas.draw_text(
                            &c.label,
                            r,
                            font,
                            pal.text,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                        );
                    }
                }
            }
            canvas.present();
        }
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn tick_anim(hwnd: HWND) {
    let mut thumbs_step: Option<(Vec<isize>, u8, bool)> = None;
    if let Ok(mut g) = PANELS.lock() {
        if let Some(p) = g.iter_mut().find(|p| p.hwnd == hwnd.0 as isize) {
            if let Some(start) = p.anim_start {
                let t = (start.elapsed().as_secs_f32() * 1000.0 / FADE_IN_MS).min(1.0);
                let alpha = (255.0 * theme::ease_fluent_out(t)).round() as u8;
                let done = t >= 1.0;
                if done {
                    p.anim_start = None;
                }
                thumbs_step = Some((p.thumbs.clone(), alpha, done));
            }
        }
    }
    if let Some((thumbs, alpha, done)) = thumbs_step {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_OPACITY,
            opacity: alpha,
            ..Default::default()
        };
        for th in thumbs {
            let _ = DwmUpdateThumbnailProperties(th, &props);
        }
        if done {
            let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
        }
    } else {
        let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
    }
}

/// Update the highlight under the cursor; redraw if it changed.
unsafe fn hover_at(hwnd: HWND, x: i32, y: i32) {
    let mut changed = false;
    if let Ok(mut g) = PANELS.lock() {
        if let Some(p) = g.iter_mut().find(|p| p.hwnd == hwnd.0 as isize) {
            let hover = p.cards.iter().position(|c| point_in(&c.rect, x, y));
            if hover != p.hover {
                p.hover = hover;
                changed = true;
            }
        }
    }
    if changed {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe fn click_at(hwnd: HWND, x: i32, y: i32) {
    let mut action: Option<(u64, u64)> = None;
    if let Ok(g) = PANELS.lock() {
        if let Some(p) = g.iter().find(|p| p.hwnd == hwnd.0 as isize) {
            if let Some(c) = p.cards.iter().find(|c| point_in(&c.rect, x, y)) {
                action = Some((c.id, p.slot));
            }
        }
    }
    if let Some((id, slot)) = action {
        post(WM_SNAP_PICK, id as usize, slot as isize);
    }
}

unsafe extern "system" fn slot_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
        WM_MOUSEACTIVATE => LRESULT(3), // MA_NOACTIVATE: clicking does not change the active window.
        WM_MOUSEMOVE => {
            let (x, y) = lparam_xy(lparam);
            hover_at(hwnd, x, y);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_xy(lparam);
            click_at(hwnd, x, y);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            post(WM_SNAP_CANCEL, 0, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Create the (hidden) panels once at startup. `scale` = dpi / 96 for text.
pub unsafe fn init(main_thread: u32, scale: f32) -> bool {
    MAIN_THREAD.store(main_thread, Ordering::Relaxed);
    let Ok(hmod) = GetModuleHandleW(None) else {
        return false;
    };
    let hinst = HINSTANCE(hmod.0);
    let class = w!("VScrollWmSnapSlot");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(slot_proc),
        hInstance: hinst,
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return false;
    }
    let pal = Palette::resolve(OverviewColors::default());
    let mut handles = Vec::new();
    for _ in 0..POOL {
        // Deliberately omit WS_EX_LAYERED so DWM thumbnails stay sharp.
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("vscroll-wm snap assist"),
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
        theme::apply_window_chrome(hwnd, &pal, false);
        handles.push(hwnd.0 as isize);
    }
    if let Ok(mut g) = HANDLES.lock() {
        *g = handles;
    }
    let fh = -((14.0 * scale).round() as i32);
    FONT.store(theme::create_font(fh, 400).0 as isize, Ordering::Relaxed);
    true
}

/// True if `h` is one of this module's panels.
pub fn is_ours(h: HWND) -> bool {
    HANDLES
        .lock()
        .map_or(false, |g| g.contains(&(h.0 as isize)))
}

/// Show one panel above each empty zone in `slots`, each containing `cands`.
/// Calling again replaces the previous display. `thumbnails` = live thumbnails
/// of the original windows (otherwise, titles only).
pub unsafe fn show(
    slots: &[SlotView],
    cands: &[Candidate],
    colors: OverviewColors,
    thumbnails: bool,
) {
    hide();
    let handles: Vec<isize> = HANDLES.lock().map(|g| g.clone()).unwrap_or_default();
    if let Ok(mut c) = COLORS.lock() {
        *c = colors;
    }
    let pal = Palette::resolve(colors);
    let anim = animations_enabled();
    let now = Instant::now();
    let mut panels: Vec<Panel> = Vec::new();
    for (view, &hw) in slots.iter().zip(handles.iter()) {
        let r = view.rect;
        let (w, h) = (r.w.max(1), r.h.max(1));
        let dest = HWND(hw as *mut _);
        theme::apply_window_chrome(dest, &pal, false);
        let mut cards = Vec::new();
        let mut thumbs = Vec::new();
        for (cand, cr) in cands.iter().zip(card_grid(cands.len(), w, h)) {
            let rect = RECT {
                left: cr.x,
                top: cr.y,
                right: cr.x + cr.w,
                bottom: cr.y + cr.h,
            };
            let mut band = None;
            if thumbnails && cand.source != 0 {
                if let Some((t, b)) =
                    register_thumb(dest, cand.source, &rect, if anim { 0 } else { 255 })
                {
                    thumbs.push(t);
                    band = Some(b);
                }
            }
            cards.push(Card {
                id: cand.id,
                label: cand.label.clone(),
                rect,
                band,
            });
        }
        panels.push(Panel {
            hwnd: hw,
            slot: view.slot,
            cards,
            hover: None,
            thumbs,
            anim_start: anim.then_some(now),
        });
    }
    // Before showing it, so the first WM_PAINT can access the data.
    let shown: Vec<(isize, Rect)> = panels
        .iter()
        .zip(slots.iter())
        .map(|(p, v)| (p.hwnd, v.rect))
        .collect();
    if let Ok(mut g) = PANELS.lock() {
        *g = panels;
    }
    for (hw, r) in shown {
        let h = HWND(hw as *mut _);
        let _ = SetWindowPos(
            h,
            Some(HWND_TOPMOST),
            r.x,
            r.y,
            r.w.max(1),
            r.h.max(1),
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        let _ = InvalidateRect(Some(h), None, false);
        if anim {
            let _ = SetTimer(Some(h), ANIM_TIMER_ID, ANIM_TICK_MS, None);
        }
    }
}

/// Hide all panels and release their thumbnails.
pub unsafe fn hide() {
    let old: Vec<Panel> = PANELS
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    for p in &old {
        let h = HWND(p.hwnd as *mut _);
        let _ = KillTimer(Some(h), ANIM_TIMER_ID);
        for t in &p.thumbs {
            let _ = DwmUnregisterThumbnail(*t);
        }
        let _ = ShowWindow(h, SW_HIDE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(outer: (i32, i32), r: &Rect) -> bool {
        r.x >= PAD && r.y >= PAD && r.x + r.w <= outer.0 - PAD && r.y + r.h <= outer.1 - PAD
    }

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn cards_fit_inside_the_panel_below_the_header_without_overlapping() {
        for n in 1..=12usize {
            let cards = card_grid(n, 900, 700);
            assert_eq!(cards.len(), n, "n = {n}");
            for (i, c) in cards.iter().enumerate() {
                assert!(inside((900, 700), c), "n = {n}: {c:?}");
                assert!(c.w >= MIN_CARD_W && c.h >= MIN_CARD_H);
                for d in &cards[i + 1..] {
                    assert!(!overlap(c, d), "n = {n}: {c:?} vs {d:?}");
                }
            }
        }
    }

    #[test]
    fn a_wide_panel_prefers_columns_and_a_tall_panel_prefers_rows() {
        let wide = card_grid(2, 1200, 300);
        assert_eq!(wide[0].y, wide[1].y); // Same row.
        let tall = card_grid(2, 300, 1000);
        assert_eq!(tall[0].x, tall[1].x); // sekolom
    }

    #[test]
    fn too_many_windows_for_a_small_panel_are_truncated_not_squeezed() {
        let cards = card_grid(30, 400, 300);
        assert!(!cards.is_empty() && cards.len() < 30);
        assert!(cards.iter().all(|c| c.w >= MIN_CARD_W && c.h >= MIN_CARD_H));
    }

    #[test]
    fn a_tiny_panel_still_gets_one_card() {
        let cards = card_grid(3, 90, 70);
        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn nothing_to_show_gives_no_cards() {
        assert!(card_grid(0, 900, 700).is_empty());
    }
}
