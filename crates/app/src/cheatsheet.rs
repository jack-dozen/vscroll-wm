//! Cheatsheet: daftar pintasan keyboard dalam panel di tengah layar (Win+/).
//!
//! Daftarnya dihasilkan dari konfigurasi yang sedang berlaku, jadi tombol yang
//! bisa diubah (overview_key, cheatsheet_key, resize_step) selalu tertulis benar.
//!
//! PERHATIAN: kalau menambah atau mengubah pintasan di main.rs, perbarui `groups()`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use layout::Rect;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HGDIOBJ, LOGFONTW, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{rgb, Config, HeightMode, OverviewColors};
use crate::overview::luminance;

const TITLE_TEXT: &str = "Pintasan keyboard";
const FOOTER_TEXT: &str = "tekan tombol apa saja, atau klik, untuk menutup";

const PAD: i32 = 28;
const KEYS_W: i32 = 250;
const DESC_W: i32 = 520;
const COL_GAP: i32 = 48;
const ROW_H: i32 = 34;
const HEADER_H: i32 = 46;
const GROUP_GAP: i32 = 12;
const TITLE_H: i32 = 66;
const FOOTER_H: i32 = 50;

pub struct Group {
    pub title: String,
    /// (tombol, keterangan)
    pub rows: Vec<(String, String)>,
}

/// Nama tombol untuk ditampilkan, dari kode virtual-key.
pub fn key_name(vk: u16) -> String {
    match vk {
        0x09 => "Tab".to_string(),
        0x20 => "Space".to_string(),
        0xC0 => "`".to_string(),
        0xBF => "/".to_string(),
        0x70..=0x7B => format!("F{}", vk - 0x6F),
        0x30..=0x39 | 0x41..=0x5A => ((vk as u8) as char).to_string(),
        _ => format!("0x{vk:02X}"),
    }
}

/// Semua pintasan, dikelompokkan, sesuai konfigurasi sekarang.
pub fn groups(cfg: &Config) -> Vec<Group> {
    let row = |k: &str, d: &str| (k.to_string(), d.to_string());

    let mut moving = vec![
        row("Win + Shift + panah", "pindah fokus ke tetangga"),
        row("Win + Alt + panah", "tukar posisi dengan tetangga"),
    ];
    if cfg.overview_key != 0 {
        moving.push((
            format!("Win + {}", key_name(cfg.overview_key)),
            "overview: peta seluruh strip".to_string(),
        ));
        moving.push(row("Win + Alt + Space", "overview (cadangan)"));
    } else {
        moving.push(row("Win + Alt + Space", "overview: peta seluruh strip"));
    }

    let help_key = if cfg.cheatsheet_key != 0 {
        format!("Win + {}", key_name(cfg.cheatsheet_key))
    } else {
        "menu tray".to_string()
    };

    let height_row = if cfg.height_mode == HeightMode::Step {
        (
            "Win + ↑ / ↓".to_string(),
            format!("tinggi lebih pendek / lebih tinggi (±{} px)", cfg.height_step),
        )
    } else {
        row("Win + ↑ / ↓", "tinggi lebih pendek / lebih tinggi")
    };

    vec![
        Group {
            title: "Susunan window".to_string(),
            rows: vec![
                height_row,
                row("Win + ← / →", "masuk lajur kiri / kanan"),
                (
                    "Win + ] / [".to_string(),
                    format!("geser pembagi lajur (±{} px)", cfg.resize_step),
                ),
                row("Win + \\", "pembagi lajur kembali 50/50"),
                row("Win + Enter", "kembali penuh (lebar + tinggi)"),
            ],
        },
        Group {
            title: "Berpindah".to_string(),
            rows: moving,
        },
        Group {
            title: "Window".to_string(),
            rows: vec![
                row("Win + Q", "tutup window"),
                row("Win + M", "minimize window"),
                row("Win + Alt + F", "float / tile window"),
                row("Win + Alt + I", "info window (class, exe, judul)"),
            ],
        },
        Group {
            title: "Aplikasi".to_string(),
            rows: vec![
                (help_key, "cheatsheet ini".to_string()),
                row("Win + Ctrl + Esc", "keluar dan pulihkan semua window"),
                row("Win + Ctrl + ← / →", "virtual desktop bawaan Windows"),
                row("ikon tray", "jeda, muat ulang konfigurasi, autostart"),
            ],
        },
        Group {
            title: "Di dalam overview".to_string(),
            rows: vec![
                row("panah atau Tab", "pilih window"),
                row("Enter / lepas Win", "loncat ke window yang dipilih"),
                row("Esc / klik di luar", "batal"),
            ],
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Title,
    Header,
    /// Garis tipis di bawah judul kelompok.
    Rule,
    Keys,
    Desc,
    Footer,
}

pub struct Cell {
    pub kind: CellKind,
    pub text: String,
    /// Relatif terhadap sudut kiri atas panel.
    pub rect: RECT,
}

pub struct Sheet {
    pub panel: Rect,
    pub columns: i32,
    pub cells: Vec<Cell>,
}

fn group_height(g: &Group) -> i32 {
    HEADER_H + g.rows.len() as i32 * ROW_H + GROUP_GAP
}

/// Bagi kelompok ke `n` kolom (yang terpendek dulu). Mengembalikan tinggi tiap kolom
/// dan isinya (indeks kelompok).
fn assign(groups: &[Group], n: i32) -> (Vec<i32>, Vec<Vec<usize>>) {
    let n = n.max(1) as usize;
    let mut heights = vec![0; n];
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, g) in groups.iter().enumerate() {
        let col = (0..n).min_by_key(|&c| heights[c]).unwrap_or(0);
        heights[col] += group_height(g);
        members[col].push(i);
    }
    (heights, members)
}

/// Hitung panel dan posisi semua teks. Dua kolom kalau lebar layar cukup
/// (tampil lebih ringkas); satu kolom kalau tidak.
pub fn compute(groups: &[Group], work: Rect) -> Sheet {
    let col_w = KEYS_W + DESC_W;
    let max_w = work.w * 92 / 100;
    let max_h = work.h * 92 / 100;
    let by_width =
        ((max_w - 2 * PAD + COL_GAP) / (col_w + COL_GAP)).clamp(1, groups.len().max(1) as i32);

    let mut columns = by_width;
    for n in by_width.min(2)..=by_width {
        let (heights, _) = assign(groups, n);
        let h = TITLE_H + heights.iter().copied().max().unwrap_or(0) + FOOTER_H + PAD;
        if h <= max_h {
            columns = n;
            break;
        }
    }

    let (heights, members) = assign(groups, columns);
    let body_h = heights.iter().copied().max().unwrap_or(0);
    let panel_w = columns * col_w + (columns - 1) * COL_GAP + 2 * PAD;
    let panel_h = TITLE_H + body_h + FOOTER_H + PAD;
    let panel = Rect {
        x: work.x + (work.w - panel_w) / 2,
        y: work.y + (work.h - panel_h) / 2,
        w: panel_w,
        h: panel_h,
    };

    let mut cells = vec![Cell {
        kind: CellKind::Title,
        text: TITLE_TEXT.to_string(),
        rect: RECT {
            left: PAD,
            top: 14,
            right: panel_w - PAD,
            bottom: TITLE_H - 6,
        },
    }];
    for (c, idxs) in members.iter().enumerate() {
        let x0 = PAD + c as i32 * (col_w + COL_GAP);
        let mut y = TITLE_H;
        for &gi in idxs {
            let g = &groups[gi];
            cells.push(Cell {
                kind: CellKind::Header,
                text: g.title.clone(),
                rect: RECT {
                    left: x0,
                    top: y,
                    right: x0 + col_w,
                    bottom: y + HEADER_H - 8,
                },
            });
            cells.push(Cell {
                kind: CellKind::Rule,
                text: String::new(),
                rect: RECT {
                    left: x0,
                    top: y + HEADER_H - 8,
                    right: x0 + col_w,
                    bottom: y + HEADER_H - 6,
                },
            });
            y += HEADER_H;
            for (keys, desc) in &g.rows {
                cells.push(Cell {
                    kind: CellKind::Keys,
                    text: keys.clone(),
                    rect: RECT {
                        left: x0,
                        top: y,
                        right: x0 + KEYS_W - 12,
                        bottom: y + ROW_H,
                    },
                });
                cells.push(Cell {
                    kind: CellKind::Desc,
                    text: desc.clone(),
                    rect: RECT {
                        left: x0 + KEYS_W,
                        top: y,
                        right: x0 + col_w,
                        bottom: y + ROW_H,
                    },
                });
                y += ROW_H;
            }
            y += GROUP_GAP;
        }
    }
    cells.push(Cell {
        kind: CellKind::Footer,
        text: FOOTER_TEXT.to_string(),
        rect: RECT {
            left: PAD,
            top: TITLE_H + body_h,
            right: panel_w - PAD,
            bottom: TITLE_H + body_h + FOOTER_H,
        },
    });
    Sheet {
        panel,
        columns,
        cells,
    }
}

// ------------------------------------------------------------------- jendela

static SHEET: Mutex<Option<(Sheet, OverviewColors)>> = Mutex::new(None);
static CS_HWND: AtomicIsize = AtomicIsize::new(0);
static FONT_NORMAL: AtomicIsize = AtomicIsize::new(0);
static FONT_BOLD: AtomicIsize = AtomicIsize::new(0);
static FONT_TITLE: AtomicIsize = AtomicIsize::new(0);
static PREV_FG: AtomicIsize = AtomicIsize::new(0);
static VISIBLE: AtomicBool = AtomicBool::new(false);

fn hwnd() -> HWND {
    HWND(CS_HWND.load(Ordering::Relaxed) as *mut _)
}

pub fn available() -> bool {
    CS_HWND.load(Ordering::Relaxed) != 0
}

unsafe fn make_font(height: i32, weight: i32) -> isize {
    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfHeight = height;
    lf.lfWeight = weight;
    for (i, ch) in "Segoe UI".encode_utf16().enumerate() {
        lf.lfFaceName[i] = ch;
    }
    CreateFontIndirectW(&lf).0 as isize
}

unsafe fn close(restore_focus: bool) {
    if !VISIBLE.swap(false, Ordering::Relaxed) {
        return;
    }
    let _ = ShowWindow(hwnd(), SW_HIDE);
    if let Ok(mut g) = SHEET.lock() {
        *g = None;
    }
    if restore_focus {
        let prev = HWND(PREV_FG.load(Ordering::Relaxed) as *mut _);
        if !prev.0.is_null() {
            let _ = SetForegroundWindow(prev);
        }
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    if let Ok(guard) = SHEET.lock() {
        if let Some((sheet, colors)) = guard.as_ref() {
            let bg = CreateSolidBrush(COLORREF(colors.background));
            let rule = CreateSolidBrush(COLORREF(colors.box_fill));
            FillRect(hdc, &client, bg);
            SetBkMode(hdc, TRANSPARENT);

            let dark_bg = luminance(colors.background) <= 150;
            let text = if dark_bg {
                rgb(232, 235, 243)
            } else {
                rgb(25, 28, 36)
            };
            let dim = if dark_bg {
                rgb(140, 148, 166)
            } else {
                rgb(100, 106, 120)
            };
            let font = |a: &AtomicIsize| HGDIOBJ(a.load(Ordering::Relaxed) as *mut _);

            for cell in &sheet.cells {
                if cell.kind == CellKind::Rule {
                    FillRect(hdc, &cell.rect, rule);
                    continue;
                }
                let (f, color, flags) = match cell.kind {
                    CellKind::Title => (&FONT_TITLE, text, DT_LEFT | DT_VCENTER | DT_SINGLELINE),
                    CellKind::Header => (
                        &FONT_BOLD,
                        colors.view,
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    ),
                    CellKind::Keys => (
                        &FONT_BOLD,
                        colors.active,
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    ),
                    CellKind::Desc => (
                        &FONT_NORMAL,
                        text,
                        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                    ),
                    _ => (&FONT_NORMAL, dim, DT_LEFT | DT_VCENTER | DT_SINGLELINE),
                };
                SelectObject(hdc, font(f));
                SetTextColor(hdc, COLORREF(color));
                let mut t: Vec<u16> = cell.text.encode_utf16().collect();
                let mut r = cell.rect;
                DrawTextW(hdc, &mut t, &mut r, flags | DT_NOPREFIX);
            }
            for b in [bg, rule] {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
        }
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn cs_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wparam.0 as u16;
            let modifier = vk == VK_SHIFT.0
                || vk == VK_CONTROL.0
                || vk == VK_MENU.0
                || vk == VK_LWIN.0
                || vk == VK_RWIN.0;
            if !modifier {
                close(true);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            close(true);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            // pindah ke window lain: tutup tanpa merebut fokus kembali
            if (wparam.0 & 0xFFFF) == 0 {
                close(false);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Buat jendela cheatsheet (tersembunyi) sekali di awal.
pub unsafe fn init() -> bool {
    let Ok(hmod) = GetModuleHandleW(None) else {
        return false;
    };
    let hinst = HINSTANCE(hmod.0);
    let class = w!("VScrollWmCheatsheet");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(cs_proc),
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
        w!("vscroll-wm cheatsheet"),
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
    CS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

    let round: u32 = 2; // DWMWCP_ROUND (Windows 11)
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &round as *const u32 as *const c_void,
        4,
    );
    FONT_NORMAL.store(make_font(-20, 400), Ordering::Relaxed);
    FONT_BOLD.store(make_font(-20, 700), Ordering::Relaxed);
    FONT_TITLE.store(make_font(-30, 700), Ordering::Relaxed);
    true
}

/// Buka cheatsheet, atau tutup kalau sedang terbuka.
pub unsafe fn toggle(work: Rect, cfg: &Config) {
    if VISIBLE.load(Ordering::Relaxed) {
        close(true);
        return;
    }
    if !available() {
        return;
    }
    let sheet = compute(&groups(cfg), work);
    let p = sheet.panel;
    PREV_FG.store(GetForegroundWindow().0 as isize, Ordering::Relaxed);
    if let Ok(mut g) = SHEET.lock() {
        *g = Some((sheet, cfg.overview_colors));
    }
    VISIBLE.store(true, Ordering::Relaxed);
    let h = hwnd();
    let _ = SetWindowPos(h, Some(HWND_TOPMOST), p.x, p.y, p.w, p.h, SWP_SHOWWINDOW);
    let _ = SetForegroundWindow(h);
    let _ = InvalidateRect(Some(h), None, false);
}

/// Tutup tanpa mengembalikan fokus (mis. saat dijeda).
pub unsafe fn hide() {
    close(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const WORK: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    fn all_rows(cfg: &Config) -> Vec<(String, String)> {
        groups(cfg).into_iter().flat_map(|g| g.rows).collect()
    }

    #[test]
    fn key_names_are_readable() {
        assert_eq!(key_name(0x09), "Tab");
        assert_eq!(key_name(0x5A), "Z");
        assert_eq!(key_name(0x33), "3");
        assert_eq!(key_name(0xBF), "/");
        assert_eq!(key_name(0x70), "F1");
        assert_eq!(key_name(0x7B), "F12");
    }

    #[test]
    fn rows_reflect_the_current_configuration() {
        let mut cfg = Config::default();
        cfg.overview_key = 0x5A;
        cfg.resize_step = 120;
        let rows = all_rows(&cfg);
        assert!(rows.iter().any(|(k, _)| k == "Win + Z"));
        assert!(rows.iter().any(|(_, d)| d.contains("120")));
        assert!(rows
            .iter()
            .any(|(k, d)| k == "Win + /" && d.contains("cheatsheet")));
    }

    #[test]
    fn disabled_keys_fall_back_gracefully() {
        let mut cfg = Config::default();
        cfg.overview_key = 0;
        cfg.cheatsheet_key = 0;
        let rows = all_rows(&cfg);
        let alt_space = rows
            .iter()
            .filter(|(k, _)| k == "Win + Alt + Space")
            .count();
        assert_eq!(alt_space, 1); // tidak dobel
        assert!(rows
            .iter()
            .any(|(k, d)| k == "menu tray" && d.contains("cheatsheet")));
    }

    #[test]
    fn no_duplicate_key_labels() {
        let rows = all_rows(&Config::default());
        let keys: HashSet<&String> = rows.iter().map(|(k, _)| k).collect();
        assert_eq!(keys.len(), rows.len());
        assert!(rows.len() >= 18);
    }

    #[test]
    fn wide_screens_get_two_columns_and_fit_inside_the_screen() {
        let groups = groups(&Config::default());
        let s = compute(&groups, WORK);
        assert_eq!(s.columns, 2);
        assert!(s.panel.w <= WORK.w * 92 / 100 && s.panel.h <= WORK.h * 92 / 100);
        let (cx, cy) = (s.panel.x + s.panel.w / 2, s.panel.y + s.panel.h / 2);
        assert!((cx - WORK.w / 2).abs() <= 1 && (cy - WORK.h / 2).abs() <= 1);
    }

    #[test]
    fn narrow_screens_fall_back_to_one_column() {
        let groups = groups(&Config::default());
        let s = compute(
            &groups,
            Rect {
                x: 0,
                y: 0,
                w: 1280,
                h: 1024,
            },
        );
        assert_eq!(s.columns, 1);
    }

    #[test]
    fn every_cell_stays_inside_the_panel() {
        let groups = groups(&Config::default());
        let s = compute(&groups, WORK);
        for c in &s.cells {
            assert!(c.rect.left >= 0 && c.rect.top >= 0, "{:?}", c.kind);
            assert!(
                c.rect.right <= s.panel.w && c.rect.bottom <= s.panel.h,
                "{:?}",
                c.kind
            );
            assert!(c.rect.right > c.rect.left && c.rect.bottom > c.rect.top);
        }
        let rows = groups.iter().map(|g| g.rows.len()).sum::<usize>();
        let keys = s.cells.iter().filter(|c| c.kind == CellKind::Keys).count();
        let descs = s.cells.iter().filter(|c| c.kind == CellKind::Desc).count();
        assert_eq!((keys, descs), (rows, rows)); // semua baris tampil
    }

    #[test]
    fn key_and_description_cells_of_a_row_do_not_overlap() {
        let groups = groups(&Config::default());
        let s = compute(&groups, WORK);
        let keys: Vec<&Cell> = s
            .cells
            .iter()
            .filter(|c| c.kind == CellKind::Keys)
            .collect();
        let descs: Vec<&Cell> = s
            .cells
            .iter()
            .filter(|c| c.kind == CellKind::Desc)
            .collect();
        for (k, d) in keys.iter().zip(descs.iter()) {
            assert!(k.rect.right <= d.rect.left);
            assert_eq!(k.rect.top, d.rect.top);
        }
    }

    #[test]
    fn height_row_follows_the_mode() {
        let mut cfg = Config::default();
        cfg.height_mode = HeightMode::Step;
        cfg.height_step = 120;
        assert!(all_rows(&cfg)
            .iter()
            .any(|(k, d)| k == "Win + ↑ / ↓" && d.contains("120")));
        assert!(!all_rows(&Config::default())
            .iter()
            .any(|(k, d)| k == "Win + ↑ / ↓" && d.contains("px")));
    }
}
