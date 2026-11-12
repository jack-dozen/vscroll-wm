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
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HGDIOBJ, LOGFONTW, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
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
/// wParam = 0 workspace sebelumnya, 1 berikutnya, n >= 2 workspace nomor n - 2 (mulai 0).
pub const WM_OV_WS: u32 = WM_APP + 13;

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

/// Satu workspace di panel.
pub struct DrawColumn {
    /// Nomor untuk label (mulai 1).
    pub number: usize,
    pub rect: RECT,
    /// Workspace yang sedang aktif.
    pub active: bool,
    /// Bingkai bagian strip yang sedang tampil di layar untuk workspace ini.
    pub view: RECT,
}

pub struct Draw {
    /// Posisi dan ukuran panel di layar (terpusat di area kerja).
    pub panel: Rect,
    /// Semua koordinat di bawah relatif terhadap sudut kiri atas panel.
    pub items: Vec<DrawItem>,
    /// Satu kolom per workspace yang terlihat, dari kiri ke kanan.
    pub columns: Vec<DrawColumn>,
    /// Lebih dari satu workspace: kolom diberi nomor dan petunjuk menyebut workspace.
    pub multi: bool,
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
/// Petunjuk dengan keterangan workspace lebih panjang.
const MIN_PANEL_W_WITH_HINT_WS: i32 = 600;
/// Jarak antar kolom workspace dan tinggi baris nomor di atasnya (piksel layar).
const COL_GAP: i32 = 16;
const COL_LABEL_H: i32 = 26;
/// Kolom workspace yang terlihat sekaligus; sisanya digulir mengikuti kursor.
pub const MAX_VISIBLE: usize = 5;
/// Lebar (dan skala) kolom yang bukan kolom kursor, persen dari kolom kursor.
const SIDE_PCT: i32 = 60;

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

/// Data panel: peta tiap workspace yang berisi window dan bagian mana yang terlihat.
pub struct Model {
    /// (nomor workspace mulai 0, petanya), urut dari kiri.
    pub maps: Vec<(usize, Overview)>,
    /// Kolom terlihat: `maps[first .. first + visible]`. Jumlahnya tetap selama overview
    /// terbuka, jadi ukuran panel tidak berubah saat kursor pindah workspace.
    pub first: usize,
    pub visible: usize,
    /// Indeks di `maps` kolom tempat kursor berada; digambar paling besar.
    pub big: usize,
    /// Nomor workspace yang aktif (disorot).
    pub active_ws: usize,
}

/// Kolom yang terlihat untuk `columns` kolom dengan kursor di `big`: `(first, visible)`.
/// Kolom kursor ditaruh di tengah jendela kalau memungkinkan; tidak ada slot kosong.
pub fn window_for(columns: usize, big: usize) -> (usize, usize) {
    let visible = columns.min(MAX_VISIBLE);
    let first = big.saturating_sub(visible / 2).min(columns - visible);
    (first, visible)
}

/// `first` baru supaya kolom `sel` terlihat: digeser seperlunya, tidak bergerak kalau
/// kolomnya sudah terlihat.
pub fn scroll(first: usize, visible: usize, sel: usize) -> usize {
    if sel < first {
        sel
    } else if sel >= first + visible {
        sel + 1 - visible
    } else {
        first
    }
}

/// Ubah data layout jadi panel siap gambar. `work` adalah area kerja layar;
/// panel dipusatkan di sana dan tidak melebihi `size_pct` persennya. Kolom kursor
/// (`model.big`) paling besar, kolom lain `SIDE_PCT` persennya, semua rata atas dan
/// tanpa ruang kosong; skala tidak bergantung pada kolom mana yang besar.
pub fn build(
    model: &Model,
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
    let multi = model.maps.len() > 1;
    let label_h = if multi { COL_LABEL_H } else { 0 };
    let k = model.visible.max(1) as i32;
    let gap = if k > 1 { COL_GAP } else { 0 };
    let side = SIDE_PCT as f32 / 100.0;
    let avail_w = (max_w - 2 * PAD).max(1);
    let avail_h = (max_h - 2 * PAD - hint_h - label_h).max(1);

    let sw = model.maps.iter().map(|(_, o)| o.width).max().unwrap_or(1).max(1) as f32;
    let sh = model.maps.iter().map(|(_, o)| o.height).max().unwrap_or(1).max(1) as f32;
    let units = sw * (1.0 + side * (k - 1) as f32);
    let cols_w = (avail_w - gap * (k - 1)).max(k) as f32;
    let scale = (cols_w / units).min(avail_h as f32 / sh);
    let (big_w, side_w) = ((sw * scale) as i32, (sw * scale * side) as i32);
    let (map_w, map_h) = (big_w + (k - 1) * (side_w + gap), (sh * scale) as i32);

    let min_w = if !opts.hint {
        0
    } else if multi {
        MIN_PANEL_W_WITH_HINT_WS
    } else {
        MIN_PANEL_W_WITH_HINT
    };
    let panel_w = (map_w + 2 * PAD).max(min_w);
    let panel_h = map_h + 2 * PAD + hint_h + label_h;
    let panel = Rect {
        x: work.x + (work.w - panel_w) / 2,
        y: work.y + (work.h - panel_h) / 2,
        w: panel_w,
        h: panel_h,
    };

    let top = PAD + label_h;
    let mut cx = (panel_w - map_w) / 2;
    let mut items = Vec::new();
    let mut columns = Vec::new();
    let end = (model.first + model.visible).min(model.maps.len());
    for i in model.first.min(end)..end {
        let (ws, ov) = &model.maps[i];
        let is_big = i == model.big;
        let cs = if is_big { scale } else { scale * side };
        let col_w = if is_big { big_w } else { side_w };
        let sc = |v: i32| (v as f32 * cs).round() as i32;
        for it in &ov.items {
            let l = cx + sc(it.rect.x) + MARGIN;
            let t = top + sc(it.rect.y) + MARGIN;
            let r = cx + sc(it.rect.x + it.rect.w) - MARGIN;
            let b = top + sc(it.rect.y + it.rect.h) - MARGIN;
            items.push(DrawItem {
                id: it.id.0,
                rect: RECT {
                    left: l,
                    top: t,
                    right: r.max(l + 4),
                    bottom: b.max(t + 4),
                },
                title: titles.get(&it.id.0).cloned().unwrap_or_default(),
                source: sources.get(&it.id.0).copied().unwrap_or(0),
                label_band: None,
            });
        }
        let vt = top + sc(ov.view_y);
        columns.push(DrawColumn {
            number: ws + 1,
            rect: RECT {
                left: cx,
                top,
                right: cx + col_w,
                bottom: top + sc(ov.height),
            },
            active: *ws == model.active_ws,
            view: RECT {
                left: cx,
                top: vt,
                right: cx + col_w,
                bottom: vt + sc(ov.view_h),
            },
        });
        cx += col_w + gap;
    }
    Draw {
        panel,
        items,
        columns,
        multi,
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
    RECT {
        left: l,
        top: t,
        right: l + w,
        bottom: t + h,
    }
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
    let _ = PostThreadMessageW(
        MAIN_THREAD.load(Ordering::Relaxed),
        msg,
        WPARAM(wparam),
        LPARAM(0),
    );
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    if let Ok(guard) = DRAW.lock() {
        let colors = guard
            .as_ref()
            .map_or_else(OverviewColors::default, |d| d.colors);
        let brush = |c: u32| CreateSolidBrush(COLORREF(c));
        let bg = brush(colors.background);
        let fill = brush(colors.box_fill);
        let fill_sel = brush(mix(colors.box_fill, 0xFF_FFFF, 14));
        let edge = brush(mix(colors.box_fill, 0xFF_FFFF, 18));
        let edge_active = brush(colors.active);
        let edge_sel = brush(colors.selected);
        let frame = brush(colors.view);
        let col_bg = brush(mix(colors.background, 0xFF_FFFF, 5));
        let col_bg_active = brush(mix(colors.background, 0xFF_FFFF, 11));

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
            // kolom workspace: latar tipis + nomor di atasnya (hanya kalau ada > 1 workspace)
            if d.multi {
                let dim = if luminance(colors.background) > 150 {
                    rgb(90, 96, 110)
                } else {
                    rgb(150, 158, 175)
                };
                for c in &d.columns {
                    FillRect(hdc, &c.rect, if c.active { col_bg_active } else { col_bg });
                    let mut label: Vec<u16> = c.number.to_string().encode_utf16().collect();
                    let mut lr = RECT {
                        left: c.rect.left,
                        top: c.rect.top - COL_LABEL_H,
                        right: c.rect.right,
                        bottom: c.rect.top,
                    };
                    SetTextColor(hdc, COLORREF(if c.active { colors.active } else { dim }));
                    DrawTextW(
                        hdc,
                        &mut label,
                        &mut lr,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                }
                SetTextColor(hdc, COLORREF(text_color));
            }
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
                                DT_LEFT
                                    | DT_VCENTER
                                    | DT_SINGLELINE
                                    | DT_END_ELLIPSIS
                                    | DT_NOPREFIX,
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

            // bingkai bagian (baris) yang sedang tampil di layar, per workspace
            for c in &d.columns {
                let v = c.view;
                let t = 3;
                let brush = if c.active { frame } else { edge };
                for edge_rect in [
                    RECT {
                        left: v.left,
                        top: v.top,
                        right: v.right,
                        bottom: v.top + t,
                    },
                    RECT {
                        left: v.left,
                        top: v.bottom - t,
                        right: v.right,
                        bottom: v.bottom,
                    },
                    RECT {
                        left: v.left,
                        top: v.top,
                        right: v.left + t,
                        bottom: v.bottom,
                    },
                    RECT {
                        left: v.right - t,
                        top: v.top,
                        right: v.right,
                        bottom: v.bottom,
                    },
                ] {
                    FillRect(hdc, &edge_rect, brush);
                }
            }

            if d.hint {
                let hint_color = if luminance(colors.background) > 150 {
                    rgb(90, 96, 110)
                } else {
                    rgb(150, 158, 175)
                };
                SetTextColor(hdc, COLORREF(hint_color));
                let hint_text = if d.multi {
                    "←↑↓→ select (←→ at an edge = other workspace)  ·  Enter jump  ·  Esc cancel"
                } else {
                    "←↑↓→ select  ·  Enter jump  ·  Esc cancel"
                };
                let mut hint: Vec<u16> = hint_text.encode_utf16().collect();
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

        for b in [
            bg,
            fill,
            fill_sel,
            edge,
            edge_active,
            edge_sel,
            frame,
            col_bg,
            col_bg_active,
        ] {
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
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            if ctrl && vk == VK_LEFT.0 {
                post(WM_OV_WS, 0);
            } else if ctrl && vk == VK_RIGHT.0 {
                post(WM_OV_WS, 1);
            } else if (0x31..=0x39).contains(&vk) {
                post(WM_OV_WS, 2 + (vk - 0x31) as usize); // tombol 1..9 = workspace n
            } else if vk == VK_LEFT.0 {
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
    let Ok(hmod) = GetModuleHandleW(None) else {
        return false;
    };
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
    let handles = if thumbnails {
        register_thumbs(h, &mut draw)
    } else {
        Vec::new()
    };
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
        let item = |id, x, y, w, h| OverviewItem {
            id: WindowId(id),
            rect: Rect { x, y, w, h },
        };
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

    const WORK: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    fn opts(hint: bool, size_pct: i32) -> Options {
        Options {
            hint,
            size_pct,
            colors: OverviewColors::default(),
        }
    }

    fn single(o: &Overview) -> Model {
        Model {
            maps: vec![(0, o.clone())],
            first: 0,
            visible: 1,
            big: 0,
            active_ws: 0,
        }
    }

    /// `n` workspace berisi peta yang sama; kursor dan workspace aktif di `big`.
    fn model(n: usize, big: usize) -> Model {
        let (first, visible) = window_for(n, big);
        Model {
            maps: (0..n).map(|i| (i, ov())).collect(),
            first,
            visible,
            big,
            active_ws: big,
        }
    }

    fn build_with(o: &Options) -> Draw {
        build(&single(&ov()), &HashMap::new(), &HashMap::new(), 1, 1, WORK, o)
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
        let d = build(
            &single(&o),
            &HashMap::new(),
            &HashMap::new(),
            1,
            1,
            WORK,
            &opts(true, 70),
        );
        assert!(d.panel.w >= MIN_PANEL_W_WITH_HINT);
        let d2 = build(
            &single(&o),
            &HashMap::new(),
            &HashMap::new(),
            1,
            1,
            WORK,
            &opts(false, 70),
        );
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
        let d = build(&single(&ov()), &titles, &sources, 2, 3, WORK, &o);
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
        let d = build(
            &single(&o),
            &HashMap::new(),
            &HashMap::new(),
            1,
            1,
            WORK,
            &opts(true, 70),
        );
        let view = d.columns[0].view;
        assert!(view.bottom > view.top);
        assert!(view.bottom <= d.panel.h);
        assert!(view.top > d.items[0].rect.top); // bingkai tidak di puncak peta
    }

    #[test]
    fn fit_keeps_aspect_ratio_and_stays_inside() {
        let area = RECT {
            left: 100,
            top: 100,
            right: 500,
            bottom: 300,
        }; // 400x200
        let r = fit(area, 1920, 1080);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        assert!(r.left >= area.left && r.right <= area.right);
        assert!(r.top >= area.top && r.bottom <= area.bottom);
        assert!((w as f32 / h as f32 - 1920.0 / 1080.0).abs() < 0.05);
        assert_eq!(h, 200); // dibatasi tinggi
    }

    #[test]
    fn fit_falls_back_to_the_area_for_unknown_source_size() {
        let area = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 50,
        };
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

    fn build_model(m: &Model) -> Draw {
        build(m, &HashMap::new(), &HashMap::new(), 1, 1, WORK, &opts(true, 70))
    }

    fn width(c: &DrawColumn) -> i32 {
        c.rect.right - c.rect.left
    }

    #[test]
    fn cursor_column_is_bigger_than_the_others() {
        let d = build_model(&model(3, 1));
        assert_eq!(d.columns.len(), 3);
        let (l, m, r) = (width(&d.columns[0]), width(&d.columns[1]), width(&d.columns[2]));
        assert!(m > l && m > r);
        assert_eq!(l, r);
    }

    #[test]
    fn panel_size_does_not_depend_on_which_column_is_big() {
        let a = build_model(&model(4, 0));
        let b = build_model(&model(4, 3));
        assert_eq!((a.panel.w, a.panel.h), (b.panel.w, b.panel.h));
        let c = build_model(&model(9, 0));
        let d = build_model(&model(9, 8));
        assert_eq!((c.panel.w, c.panel.h), (d.panel.w, d.panel.h));
    }

    #[test]
    fn columns_touch_no_empty_slots_and_margins_are_equal() {
        let o = opts(false, 70); // tanpa petunjuk: tidak ada lebar minimum
        for n in 1..=9 {
            for big in 0..n {
                let d = build(&model(n, big), &HashMap::new(), &HashMap::new(), 1, 1, WORK, &o);
                let (first, last) = (d.columns.first().unwrap(), d.columns.last().unwrap());
                assert!(
                    (first.rect.left - (d.panel.w - last.rect.right)).abs() <= 1,
                    "n={n} big={big}"
                );
                assert!(first.rect.left >= 0 && last.rect.right <= d.panel.w);
                for pair in d.columns.windows(2) {
                    assert_eq!(pair[1].rect.left - pair[0].rect.right, COL_GAP);
                    assert_eq!(pair[0].rect.top, pair[1].rect.top); // rata atas
                }
            }
        }
    }

    #[test]
    fn cursor_column_is_centered_whenever_both_sides_have_the_same_count() {
        for n in 1..=9usize {
            for big in 0..n {
                let (first, visible) = window_for(n, big);
                let (left, right) = (big - first, first + visible - 1 - big);
                if left != right {
                    continue;
                }
                let d = build_model(&model(n, big));
                let c = d.columns.iter().find(|c| c.active).unwrap();
                let mid = (c.rect.left + c.rect.right) / 2;
                assert!((mid - d.panel.w / 2).abs() <= 1, "n={n} big={big}");
            }
        }
    }

    #[test]
    fn window_for_never_leaves_slots_empty_and_contains_the_cursor() {
        assert_eq!(window_for(1, 0), (0, 1));
        assert_eq!(window_for(3, 1), (0, 3));
        assert_eq!(window_for(9, 0), (0, MAX_VISIBLE));
        assert_eq!(window_for(9, 4), (2, MAX_VISIBLE));
        assert_eq!(window_for(9, 8), (4, MAX_VISIBLE));
        for n in 1..=12usize {
            for big in 0..n {
                let (first, visible) = window_for(n, big);
                assert_eq!(visible, n.min(MAX_VISIBLE));
                assert!(first + visible <= n);
                assert!(big >= first && big < first + visible);
            }
        }
    }

    #[test]
    fn scroll_keeps_the_selected_column_visible_and_stays_put_otherwise() {
        for n in 1..=12usize {
            let visible = n.min(MAX_VISIBLE);
            for first in 0..=(n - visible) {
                for sel in 0..n {
                    let f = scroll(first, visible, sel);
                    assert!(f + visible <= n, "n={n} first={first} sel={sel}");
                    assert!(sel >= f && sel < f + visible);
                    if sel >= first && sel < first + visible {
                        assert_eq!(f, first);
                    }
                }
            }
        }
    }

    #[test]
    fn only_the_visible_window_of_columns_is_drawn() {
        let mut m = model(7, 3);
        m.first = 1;
        m.visible = 5;
        let d = build_model(&m);
        assert_eq!(d.columns.len(), 5);
        assert_eq!(d.items.len(), 15); // 5 kolom x 3 window
        assert_eq!(d.columns[0].number, 2);
    }

    #[test]
    fn columns_use_workspace_numbers_and_flag_only_the_active_one() {
        let m = Model {
            maps: vec![(0, ov()), (2, ov()), (3, ov())],
            first: 0,
            visible: 3,
            big: 1,
            active_ws: 2,
        };
        let d = build_model(&m);
        let nums: Vec<usize> = d.columns.iter().map(|c| c.number).collect();
        let act: Vec<bool> = d.columns.iter().map(|c| c.active).collect();
        assert_eq!(nums, vec![1, 3, 4]);
        assert_eq!(act, vec![false, true, false]);
        assert!(d.multi);
    }

    #[test]
    fn single_workspace_has_no_label_row() {
        let d = build_with(&opts(true, 70));
        assert!(!d.multi);
        assert_eq!(d.columns.len(), 1);
    }
}
