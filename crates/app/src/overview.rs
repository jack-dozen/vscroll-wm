//! Overview: peta seluruh strip dalam panel ringkas di tengah layar, seperti
//! Alt+Tab (Win+Tab / Win+Alt+Space).
//!
//! Jendela overlay sendiri. Data layout dan logika pilihan ada di thread utama;
//! di sini hanya menggambar dan meneruskan input (panah / Enter / Esc / klik)
//! sebagai pesan ke thread utama. Tiap kotak bisa menampilkan thumbnail langsung
//! dari window aslinya lewat DWM.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;

use layout::{Overview, Rect};
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmSetWindowAttribute,
    DwmUnregisterThumbnail, DwmUpdateThumbnailProperties, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HGDIOBJ, LOGFONTW, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{rgb, OverviewColors};

/// wParam = 0 kiri, 1 kanan, 2 atas, 3 bawah.
pub const WM_OV_MOVE: u32 = WM_APP + 7;
pub const WM_OV_ACCEPT: u32 = WM_APP + 8;
pub const WM_OV_CANCEL: u32 = WM_APP + 9;
/// wParam = id window yang diklik.
pub const WM_OV_PICK: u32 = WM_APP + 10;
/// wParam = 1 window berikutnya, 0 sebelumnya (urutan baca).
pub const WM_OV_CYCLE: u32 = WM_APP + 11;

/// Pengaturan tampilan panel.
pub struct Options {
    /// Tampilkan petunjuk tombol di bagian bawah.
    pub hint: bool,
    /// Ukuran maksimum panel, persen dari area kerja (30-100).
    pub size_pct: i32,
    pub colors: OverviewColors,
}

pub struct DrawItem {
    pub id: u64,
    pub rect: RECT,
    pub title: String,
    /// HWND window asli (0 = tanpa thumbnail).
    pub source: isize,
    /// Kalau ada thumbnail: pita di bawah kotak tempat label digambar.
    pub label_band: Option<RECT>,
}

pub struct Draw {
    /// Posisi dan ukuran panel di layar (terpusat di area kerja).
    pub panel: Rect,
    /// Semua koordinat di bawah relatif terhadap sudut kiri atas panel.
    pub items: Vec<DrawItem>,
    /// Bingkai bagian strip yang sedang tampil di layar.
    pub view: RECT,
    pub selected: u64,
    /// Window yang sedang aktif sekarang.
    pub focused: u64,
    pub hint: bool,
    pub colors: OverviewColors,
}

static DRAW: Mutex<Option<Draw>> = Mutex::new(None);
/// Handle thumbnail DWM yang sedang terpasang.
static THUMBS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static OV_HWND: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const MARGIN: i32 = 3;
const LABEL_BAND: i32 = 30;
const PAD: i32 = 20;
const HINT_H: i32 = 44;
/// Lebar minimum panel supaya teks petunjuk tidak terpotong.
const MIN_PANEL_W_WITH_HINT: i32 = 460;

fn channels(c: u32) -> (u32, u32, u32) {
    (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF)
}

/// Campur warna `c` ke arah `toward` sebesar `pct` persen.
fn mix(c: u32, toward: u32, pct: u32) -> u32 {
    let (r, g, b) = channels(c);
    let (tr, tg, tb) = channels(toward);
    let f = |a: u32, t: u32| (a * (100 - pct) + t * pct) / 100;
    rgb(f(r, tr), f(g, tg), f(b, tb))
}

/// Terang gelapnya warna, 0-255.
pub(crate) fn luminance(c: u32) -> u32 {
    let (r, g, b) = channels(c);
    (r * 299 + g * 587 + b * 114) / 1000
}

/// Ubah data layout jadi panel siap gambar. `work` adalah area kerja layar;
/// panel dipusatkan di sana dan tidak melebihi `size_pct` persennya.
pub fn build(
    ov: &Overview,
    titles: &HashMap<u64, String>,
    sources: &HashMap<u64, isize>,
    selected: u64,
    focused: u64,
    work: Rect,
    opts: &Options,
) -> Draw {
    let pct = opts.size_pct.clamp(30, 100);
    let max_w = work.w * pct / 100;
    let max_h = work.h * pct / 100;
    let hint_h = if opts.hint { HINT_H } else { 0 };
    let avail_w = (max_w - 2 * PAD).max(1);
    let avail_h = (max_h - 2 * PAD - hint_h).max(1);

    let (sw, sh) = (ov.width.max(1) as f32, ov.height.max(1) as f32);
    let scale = (avail_w as f32 / sw).min(avail_h as f32 / sh);
    let (map_w, map_h) = ((sw * scale) as i32, (sh * scale) as i32);

    let min_w = if opts.hint { MIN_PANEL_W_WITH_HINT } else { 0 };
    let panel_w = (map_w + 2 * PAD).max(min_w);
    let panel_h = map_h + 2 * PAD + hint_h;
    let panel = Rect {
        x: work.x + (work.w - panel_w) / 2,
        y: work.y + (work.h - panel_h) / 2,
        w: panel_w,
        h: panel_h,
    };

    let ox = (panel_w - map_w) / 2;
    let oy = PAD;
    let sc = |v: i32| (v as f32 * scale).round() as i32;

    let items = ov
        .items
        .iter()
        .map(|it| {
            let l = ox + sc(it.rect.x) + MARGIN;
            let t = oy + sc(it.rect.y) + MARGIN;
            let r = ox + sc(it.rect.x + it.rect.w) - MARGIN;
            let b = oy + sc(it.rect.y + it.rect.h) - MARGIN;
            DrawItem {
                id: it.id.0,
                rect: RECT { left: l, top: t, right: r.max(l + 4), bottom: b.max(t + 4) },
                title: titles.get(&it.id.0).cloned().unwrap_or_default(),
                source: sources.get(&it.id.0).copied().unwrap_or(0),
                label_band: None,
            }
        })
        .collect();
    let vt = oy + sc(ov.view_y);
    let view = RECT { left: ox, top: vt, right: ox + map_w, bottom: vt + sc(ov.view_h) };
    Draw {
        panel,
        items,
        view,
        selected,
        focused,
        hint: opts.hint,
        colors: opts.colors,
    }
}

/// Muat sebuah sumber berukuran (sw x sh) ke `area` tanpa mengubah rasio, di tengah.
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
    RECT { left: l, top: t, right: l + w, bottom: t + h }
}

/// Pasang thumbnail DWM untuk tiap kotak yang cukup besar. Kotak yang dapat
/// thumbnail mendapat `label_band` untuk labelnya; yang lain tetap berlabel di tengah.
unsafe fn register_thumbs(dest: HWND, draw: &mut Draw) -> Vec<isize> {
    let mut handles = Vec::new();
    for it in draw.items.iter_mut() {
        if it.source == 0 {
            continue;
        }
        let inner = RECT {
            left: it.rect.left + 3,
            top: it.rect.top + 3,
            right: it.rect.right - 3,
            bottom: it.rect.bottom - 3,
        };
        let area = RECT {
            left: inner.left + 4,
            top: inner.top + 4,
            right: inner.right - 4,
            bottom: inner.bottom - LABEL_BAND,
        };
        if area.right - area.left < 40 || area.bottom - area.top < 30 {
            continue; // terlalu kecil: cukup label teks
        }
        let Ok(thumb) = DwmRegisterThumbnail(dest, HWND(it.source as *mut _)) else {
            continue;
        };
        let size = DwmQueryThumbnailSourceSize(thumb).unwrap_or(SIZE { cx: 0, cy: 0 });
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
            rcDestination: fit(area, size.cx, size.cy),
            rcSource: RECT::default(),
            opacity: 255,
            fVisible: BOOL(1),
            fSourceClientAreaOnly: BOOL(0),
        };
        if DwmUpdateThumbnailProperties(thumb, &props).is_err() {
            let _ = DwmUnregisterThumbnail(thumb);
            continue;
        }
        handles.push(thumb);
        it.label_band = Some(RECT {
            left: inner.left,
            top: inner.bottom - LABEL_BAND,
            right: inner.right,
            bottom: inner.bottom,
        });
    }
    handles
}

unsafe fn unregister_thumbs() {
    if let Ok(mut t) = THUMBS.lock() {
        for th in t.drain(..) {
            let _ = DwmUnregisterThumbnail(th);
        }
    }
}

unsafe fn post(msg: u32, wparam: usize) {
    let _ = PostThreadMessageW(MAIN_THREAD.load(Ordering::Relaxed), msg, WPARAM(wparam), LPARAM(0));
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    if let Ok(guard) = DRAW.lock() {
        let colors = guard.as_ref().map_or_else(OverviewColors::default, |d| d.colors);
        let brush = |c: u32| CreateSolidBrush(COLORREF(c));
        let bg = brush(colors.background);
        let fill = brush(colors.box_fill);
        let fill_sel = brush(mix(colors.box_fill, 0xFF_FFFF, 14));
        let edge = brush(mix(colors.box_fill, 0xFF_FFFF, 18));
        let edge_active = brush(colors.active);
        let edge_sel = brush(colors.selected);
        let frame = brush(colors.view);

        FillRect(hdc, &client, bg);
        let font = FONT.load(Ordering::Relaxed);
        if font != 0 {
            SelectObject(hdc, HGDIOBJ(font as *mut _));
        }
        SetBkMode(hdc, TRANSPARENT);

        if let Some(d) = guard.as_ref() {
            let text_color = if luminance(colors.box_fill) > 150 {
                rgb(25, 28, 36)
            } else {
                rgb(235, 238, 245)
            };
            SetTextColor(hdc, COLORREF(text_color));
            for it in &d.items {
                let selected = it.id == d.selected;
                let active = it.id == d.focused;
                let r = it.rect;
                let border = if selected {
                    edge_sel
                } else if active {
                    edge_active
                } else {
                    edge
                };
                FillRect(hdc, &r, border);
                let inner = RECT {
                    left: r.left + 3,
                    top: r.top + 3,
                    right: (r.right - 3).max(r.left + 4),
                    bottom: (r.bottom - 3).max(r.top + 4),
                };
                FillRect(hdc, &inner, if selected { fill_sel } else { fill });
                if !it.title.is_empty() {
                    let mut text: Vec<u16> = it.title.encode_utf16().collect();
                    match it.label_band {
                        // ada thumbnail: label di pita bawah, rata kiri
                        Some(band) => {
                            let mut tr = RECT {
                                left: band.left + 8,
                                top: band.top,
                                right: (band.right - 8).max(band.left + 9),
                                bottom: band.bottom,
                            };
                            DrawTextW(
                                hdc,
                                &mut text,
                                &mut tr,
                                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                            );
                        }
                        // tanpa thumbnail: label di tengah kotak
                        None => {
                            let mut tr = RECT {
                                left: inner.left + 8,
                                top: inner.top,
                                right: (inner.right - 8).max(inner.left + 9),
                                bottom: inner.bottom,
                            };
                            DrawTextW(
                                hdc,
                                &mut text,
                                &mut tr,
                                DT_CENTER
                                    | DT_VCENTER
                                    | DT_SINGLELINE
                                    | DT_END_ELLIPSIS
                                    | DT_NOPREFIX,
                            );
                        }
                    }
                }
            }

            // bingkai bagian (baris) yang sedang tampil di layar
            let v = d.view;
            let t = 3;
            for edge_rect in [
                RECT { left: v.left, top: v.top, right: v.right, bottom: v.top + t },
                RECT { left: v.left, top: v.bottom - t, right: v.right, bottom: v.bottom },
                RECT { left: v.left, top: v.top, right: v.left + t, bottom: v.bottom },
                RECT { left: v.right - t, top: v.top, right: v.right, bottom: v.bottom },
            ] {
                FillRect(hdc, &edge_rect, frame);
            }

            if d.hint {
                let hint_color = if luminance(colors.background) > 150 {
                    rgb(90, 96, 110)
                } else {
                    rgb(150, 158, 175)
                };
                SetTextColor(hdc, COLORREF(hint_color));
                let mut hint: Vec<u16> = "←↑↓→ pilih  ·  Enter loncat  ·  Esc batal"
                    .encode_utf16()
                    .collect();
                let mut hr = RECT {
                    left: client.left,
                    top: client.bottom - HINT_H,
                    right: client.right,
                    bottom: client.bottom - 4,
                };
                DrawTextW(
                    hdc,
                    &mut hint,
                    &mut hr,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
            }
        }

        for b in [bg, fill, fill_sel, edge, edge_active, edge_sel, frame] {
            let _ = DeleteObject(HGDIOBJ(b.0));
        }
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn ov_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_LEFT.0 {
                post(WM_OV_MOVE, 0);
            } else if vk == VK_RIGHT.0 {
                post(WM_OV_MOVE, 1);
            } else if vk == VK_UP.0 {
                post(WM_OV_MOVE, 2);
            } else if vk == VK_DOWN.0 {
                post(WM_OV_MOVE, 3);
            } else if vk == VK_RETURN.0 {
                post(WM_OV_ACCEPT, 0);
            } else if vk == VK_ESCAPE.0 {
                post(WM_OV_CANCEL, 0);
            } else if vk == VK_TAB.0 {
                let back = GetKeyState(VK_SHIFT.0 as i32) < 0;
                post(WM_OV_CYCLE, if back { 0 } else { 1 });
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let hit = DRAW.lock().ok().and_then(|g| {
                g.as_ref().and_then(|d| {
                    d.items
                        .iter()
                        .find(|it| {
                            x >= it.rect.left
                                && x < it.rect.right
                                && y >= it.rect.top
                                && y < it.rect.bottom
                        })
                        .map(|it| it.id)
                })
            });
            match hit {
                Some(id) => post(WM_OV_PICK, id as usize),
                None => post(WM_OV_CANCEL, 0),
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            post(WM_OV_CANCEL, 0);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            // klik di luar panel / pindah ke app lain: batalkan
            if (wparam.0 & 0xFFFF) == 0 && VISIBLE.load(Ordering::Relaxed) {
                post(WM_OV_CANCEL, 0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Buat jendela overlay (tersembunyi) sekali di awal. Sengaja bukan jendela
/// berlapis (layered): thumbnail DWM tidak tampil benar di jendela berlapis.
pub unsafe fn init(main_thread: u32) -> bool {
    MAIN_THREAD.store(main_thread, Ordering::Relaxed);
    let Ok(hmod) = GetModuleHandleW(None) else { return false };
    let hinst = HINSTANCE(hmod.0);
    let class = w!("VScrollWmOverview");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(ov_proc),
        hInstance: hinst,
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return false;
    }
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class,
        w!("vscroll-wm overview"),
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
    OV_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

    // sudut membulat (Windows 11; di Windows 10 diabaikan)
    let round: u32 = 2; // DWMWCP_ROUND
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &round as *const u32 as *const c_void,
        4,
    );

    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfHeight = -22;
    lf.lfWeight = 400;
    for (i, ch) in "Segoe UI".encode_utf16().enumerate() {
        lf.lfFaceName[i] = ch;
    }
    let font = CreateFontIndirectW(&lf);
    FONT.store(font.0 as isize, Ordering::Relaxed);
    true
}

/// True kalau jendela overlay berhasil dibuat.
pub fn available() -> bool {
    OV_HWND.load(Ordering::Relaxed) != 0
}

fn hwnd() -> HWND {
    HWND(OV_HWND.load(Ordering::Relaxed) as *mut _)
}

/// Tampilkan panel di posisinya dan ambil fokus keyboard.
pub unsafe fn show(mut draw: Draw, thumbnails: bool) {
    if OV_HWND.load(Ordering::Relaxed) == 0 {
        return;
    }
    let h = hwnd();
    unregister_thumbs(); // jaga-jaga kalau sisa dari tampilan sebelumnya
    let handles = if thumbnails { register_thumbs(h, &mut draw) } else { Vec::new() };
    if let Ok(mut t) = THUMBS.lock() {
        *t = handles;
    }
    let p = draw.panel;
    if let Ok(mut g) = DRAW.lock() {
        *g = Some(draw);
    }
    VISIBLE.store(true, Ordering::Relaxed);
    let _ = SetWindowPos(h, Some(HWND_TOPMOST), p.x, p.y, p.w, p.h, SWP_SHOWWINDOW);
    let _ = SetForegroundWindow(h);
    let _ = InvalidateRect(Some(h), None, false);
}

/// Ganti window yang sedang dipilih (kursor overview).
pub unsafe fn set_selected(id: u64) {
    if let Ok(mut g) = DRAW.lock() {
        if let Some(d) = g.as_mut() {
            d.selected = id;
        }
    }
    let _ = InvalidateRect(Some(hwnd()), None, false);
}

pub unsafe fn hide() {
    VISIBLE.store(false, Ordering::Relaxed);
    unregister_thumbs();
    if OV_HWND.load(Ordering::Relaxed) != 0 {
        let _ = ShowWindow(hwnd(), SW_HIDE);
    }
    if let Ok(mut g) = DRAW.lock() {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::{OverviewItem, WindowId};

    fn ov() -> Overview {
        let item = |id, x, y, w, h| OverviewItem { id: WindowId(id), rect: Rect { x, y, w, h } };
        Overview {
            items: vec![
                item(1, 0, 0, 960, 540),
                item(2, 960, 0, 960, 540),
                item(3, 0, 540, 1920, 540),
            ],
            width: 1920,
            height: 1080,
            view_y: 0,
            view_h: 1080,
        }
    }

    const WORK: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    fn opts(hint: bool, size_pct: i32) -> Options {
        Options { hint, size_pct, colors: OverviewColors::default() }
    }

    fn build_with(o: &Options) -> Draw {
        build(&ov(), &HashMap::new(), &HashMap::new(), 1, 1, WORK, o)
    }

    #[test]
    fn panel_is_centered_and_within_the_requested_size() {
        let d = build_with(&opts(true, 70));
        assert!(d.panel.w <= WORK.w * 70 / 100 + 1);
        assert!(d.panel.h <= WORK.h * 70 / 100 + 1);
        let (cx, cy) = (d.panel.x + d.panel.w / 2, d.panel.y + d.panel.h / 2);
        assert!((cx - WORK.w / 2).abs() <= 1 && (cy - WORK.h / 2).abs() <= 1);
    }

    #[test]
    fn boxes_fit_inside_the_panel_and_do_not_overlap() {
        let d = build_with(&opts(true, 70));
        assert_eq!(d.items.len(), 3);
        for it in &d.items {
            assert!(it.rect.left >= 0 && it.rect.top >= 0);
            assert!(it.rect.right <= d.panel.w && it.rect.bottom <= d.panel.h);
            assert!(it.rect.right > it.rect.left && it.rect.bottom > it.rect.top);
        }
        let (a, b, c) = (d.items[0].rect, d.items[1].rect, d.items[2].rect);
        assert!(a.right <= b.left);
        assert!(a.bottom <= c.top && b.bottom <= c.top);
    }

    #[test]
    fn bigger_size_setting_gives_bigger_panel() {
        let (small, big) = (build_with(&opts(true, 40)), build_with(&opts(true, 90)));
        assert!(small.panel.w < big.panel.w && small.panel.h < big.panel.h);
    }

    #[test]
    fn hiding_the_hint_shrinks_the_space_below_the_map() {
        let (on, off) = (build_with(&opts(true, 70)), build_with(&opts(false, 70)));
        let below = |d: &Draw| d.panel.h - d.items[2].rect.bottom;
        assert!(below(&off) < below(&on));
        assert!(!off.hint && on.hint);
    }

    #[test]
    fn narrow_strip_still_leaves_room_for_the_hint_text() {
        let mut o = ov();
        o.height = 8000; // strip sangat tinggi -> peta sangat sempit
        let d = build(&o, &HashMap::new(), &HashMap::new(), 1, 1, WORK, &opts(true, 70));
        assert!(d.panel.w >= MIN_PANEL_W_WITH_HINT);
        let d2 = build(&o, &HashMap::new(), &HashMap::new(), 1, 1, WORK, &opts(false, 70));
        assert!(d2.panel.w < MIN_PANEL_W_WITH_HINT);
    }

    #[test]
    fn titles_sources_selection_and_colors_are_carried_over() {
        let mut titles = HashMap::new();
        titles.insert(2u64, "Claude".to_string());
        let mut sources = HashMap::new();
        sources.insert(2u64, 1234isize);
        let mut o = opts(true, 70);
        o.colors.selected = rgb(1, 2, 3);
        let d = build(&ov(), &titles, &sources, 2, 3, WORK, &o);
        assert_eq!(d.items[1].title, "Claude");
        assert_eq!(d.items[1].source, 1234);
        assert_eq!(d.items[0].title, "");
        assert_eq!(d.items[0].source, 0);
        assert_eq!((d.selected, d.focused), (2, 3));
        assert_eq!(d.colors.selected, rgb(1, 2, 3));
    }

    #[test]
    fn viewport_frame_tracks_the_visible_part() {
        let mut o = ov();
        o.height = 2160; // dua layar tinggi; yang tampil: layar bawah
        o.view_y = 1080;
        let d = build(&o, &HashMap::new(), &HashMap::new(), 1, 1, WORK, &opts(true, 70));
        assert!(d.view.bottom > d.view.top);
        assert!(d.view.bottom <= d.panel.h);
        assert!(d.view.top > d.items[0].rect.top); // bingkai tidak di puncak peta
    }

    #[test]
    fn fit_keeps_aspect_ratio_and_stays_inside() {
        let area = RECT { left: 100, top: 100, right: 500, bottom: 300 }; // 400x200
        let r = fit(area, 1920, 1080);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        assert!(r.left >= area.left && r.right <= area.right);
        assert!(r.top >= area.top && r.bottom <= area.bottom);
        assert!((w as f32 / h as f32 - 1920.0 / 1080.0).abs() < 0.05);
        assert_eq!(h, 200); // dibatasi tinggi
    }

    #[test]
    fn fit_falls_back_to_the_area_for_unknown_source_size() {
        let area = RECT { left: 0, top: 0, right: 100, bottom: 50 };
        let r = fit(area, 0, 0);
        assert_eq!((r.left, r.top, r.right, r.bottom), (0, 0, 100, 50));
    }

    #[test]
    fn color_helpers_behave() {
        assert!(luminance(rgb(255, 255, 255)) > 250);
        assert!(luminance(rgb(0, 0, 0)) < 5);
        assert!(luminance(rgb(20, 22, 28)) < 150); // latar gelap bawaan -> teks terang
        let lighter = mix(rgb(40, 40, 40), 0xFF_FFFF, 20);
        assert!(luminance(lighter) > luminance(rgb(40, 40, 40)));
        assert_eq!(mix(rgb(10, 20, 30), rgb(200, 200, 200), 0), rgb(10, 20, 30));
    }
}
