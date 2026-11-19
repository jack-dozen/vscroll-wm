//! Overview (gaya Windows 11 Task View + Minimap):
//!  - Bagian kiri/utama (stage): menampilkan satu workspace aktif/terpilih dalam skala
//!    besar & bisa di-scroll vertikal (mouse wheel atau otomatis mengikuti kursor).
//!  - Bagian kanan (minimap): peta utuh 100% seluruh strip dari atas sampai bawah lengkap
//!    dengan nama tiap window, bingkai `view`, sorotan `selected`, serta slider posisi
//!    scroll — bisa diklik/diseret untuk langsung melompat ke bagian mana pun di strip.
//!  - Bagian bawah (dock workspace, saat ada > 1 workspace): deretan kartu mini untuk
//!    tiap workspace (persis bar Desktop di Task View Windows 11). Klik untuk pindah,
//!    atau seret window ke kartu workspace untuk memindahkannya.

use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use layout::{Overview, Rect, Span};
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY,
    DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HFONT, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_MENU,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::OverviewColors;
use crate::theme::{self, Canvas, Palette};
use crate::winutil::animations_enabled;

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
/// Window diseret dan dilepas di overview; detailnya diambil lewat `take_drop()`.
pub const WM_OV_DROP: u32 = WM_APP + 14;
/// Zoom stage diubah (Ctrl+Wheel atau +/-/0): bangun ulang panel pada skala baru.
pub const WM_OV_ZOOM: u32 = WM_APP + 15;

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
    /// Koordinat konten (belum dikurangi `scroll_y`).
    pub rect: RECT,
    pub title: String,
    /// HWND window asli (0 = tanpa thumbnail).
    pub source: isize,
    /// Kalau ada thumbnail: pita di bawah kotak tempat label digambar (koordinat konten).
    pub label_band: Option<RECT>,
    /// Lebar window di strip: penuh, atau lajur kiri / kanan (untuk titik drop).
    pub span: Span,
}

/// Strip workspace yang sedang tampil di stage atas (koordinat konten).
pub struct DrawColumn {
    /// Nomor untuk label (mulai 1).
    pub number: usize,
    pub rect: RECT,
    /// Bingkai bagian strip yang sedang tampil di layar.
    pub view: RECT,
}

/// Satu kartu workspace di bilah bawah (gaya bar Desktop Task View Windows 11).
pub struct DrawDockCard {
    /// Nomor untuk label (mulai 1).
    pub number: usize,
    /// Indeks workspace (mulai 0) untuk pesan `WM_OV_WS`.
    pub ws_index: usize,
    /// Kotak kartu keseluruhan di panel.
    pub rect: RECT,
    /// Area pratinjau mini di dalam kartu.
    pub preview: RECT,
    /// Miniatur kotak window (wireframe) di dalam `preview`.
    pub mini_items: Vec<RECT>,
    /// Workspace yang sedang dibuka di stage atas.
    pub current: bool,
}

/// Satu window di dalam minimap samping (peta utuh 100% strip).
pub struct MinimapItem {
    pub id: u64,
    pub rect: RECT,
    pub title: String,
}

/// Peta utuh seluruh strip di sisi kanan stage (ala minimap VS Code).
pub struct DrawMinimap {
    /// Kotak panel minimap (koordinat panel).
    pub rect: RECT,
    /// Kotak strip yang terpetakan 100% di dalam minimap.
    pub strip_rect: RECT,
    /// Bingkai `view` (area yang tampil di desktop) pada skala minimap.
    pub view_rect: RECT,
    /// Semua window di strip pada skala minimap.
    pub items: Vec<MinimapItem>,
}

pub struct Draw {
    /// Posisi dan ukuran panel di layar (terpusat di area kerja).
    pub panel: Rect,
    /// Area stage atas tempat strip workspace saat ini digambar & di-scroll.
    pub stage: RECT,
    /// Offset scroll vertikal stage saat ini (`0 ..= max_scroll`).
    pub scroll_y: i32,
    /// Batas maksimum scroll vertikal (0 kalau seluruh strip muat di stage).
    pub max_scroll: i32,
    /// Semua window di workspace yang sedang dibuka (koordinat konten).
    pub items: Vec<DrawItem>,
    /// Satu kolom untuk workspace yang sedang dibuka di stage (koordinat konten).
    pub columns: Vec<DrawColumn>,
    /// Minimap penuh 100% strip di sisi kanan stage.
    pub minimap: Option<DrawMinimap>,
    /// Bar kartu workspace di bagian bawah (terisi kalau ada > 1 workspace).
    pub dock: Vec<DrawDockCard>,
    /// Lebih dari satu workspace.
    pub multi: bool,
    pub selected: u64,
    /// Window yang sedang aktif sekarang.
    #[allow(dead_code)]
    pub focused: u64,
    pub hint: bool,
    pub colors: OverviewColors,
}

static DRAW: Mutex<Option<Draw>> = Mutex::new(None);

/// Drag window di overview (seperti Task View): tombol kiri ditekan di sebuah kotak lalu
/// digeser melewati ambang.
struct Drag {
    /// Window yang diseret.
    id: u64,
    start: (i32, i32),
    /// Posisi kursor terakhir (koordinat panel), untuk auto-scroll & hover dock.
    pos: (i32, i32),
    /// Sudah melewati ambang gerak (bukan sekadar klik).
    active: bool,
    /// Titik tujuan di bawah kursor (hanya untuk penanda; dihitung ulang saat dilepas).
    slot: Option<DropSlot>,
    /// Kartu dock workspace yang sedang di-hover saat drag: (ws_index, mulai hover).
    hover_dock: Option<(usize, Instant)>,
}
static DRAG: Mutex<Option<Drag>> = Mutex::new(None);
/// Sedang menyeret / mengklik minimap untuk menggeser scroll stage.
static MINIMAP_DRAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Mode angkat-dan-jatuhkan lewat keyboard di dalam overview (`Space` angkat, panah geser
/// penanda, `Enter` jatuhkan lewat jalur `DROP` yang sama dengan mouse, `Esc` batal).
struct KbPick {
    src: u64,
    slots: Vec<DropSlot>,
    idx: usize,
}
static KB_PICK: Mutex<Option<KbPick>> = Mutex::new(None);

/// Hasil drop yang menunggu diambil thread utama: (window yang diseret, tujuannya).
type DropInfo = (u64, DropSlot);
static DROP: Mutex<Option<DropInfo>> = Mutex::new(None);

/// Gerak minimum (piksel) sebelum klik dianggap drag.
const DRAG_START: i32 = 6;

/// Ambil hasil drop terakhir (sekali).
pub fn take_drop() -> Option<DropInfo> {
    DROP.lock().ok().and_then(|mut g| g.take())
}

fn point_in(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn shift_y(r: RECT, dy: i32) -> RECT {
    RECT {
        left: r.left,
        top: r.top + dy,
        right: r.right,
        bottom: r.bottom + dy,
    }
}

/// Isi panel di titik (x, y): (indeks kartu dock, id window di stage selain `skip`).
fn hit_at(x: i32, y: i32, skip: u64) -> (Option<usize>, Option<u64>) {
    let Ok(g) = DRAW.lock() else {
        return (None, None);
    };
    let Some(d) = g.as_ref() else {
        return (None, None);
    };
    if let Some(di) = d.dock.iter().position(|c| point_in(&c.rect, x, y)) {
        return (Some(di), None);
    }
    if point_in(&d.stage, x, y) {
        let cy = y + d.scroll_y;
        let item = d
            .items
            .iter()
            .find(|it| it.id != skip && point_in(&it.rect, x, cy))
            .map(|it| it.id);
        return (None, item);
    }
    (None, None)
}

/// Di mana window yang dijatuhkan akan mendarat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Setengah lebar, di lajur seberang window ini (sejajar).
    Beside(u64),
    /// Penuh lebar di atas blok window ini.
    Before(u64),
    /// Penuh lebar di bawah blok window ini.
    After(u64),
    /// Membagi dua window ini; `bool` = window baru di kiri / atas.
    Split(u64, bool),
    /// Pindah ke workspace lain (dijatuhkan di kartu dock bawah).
    Workspace,
}

/// Titik tujuan drop: nomor workspace (mulai 1), jenisnya, dan area penandanya
/// (koordinat konten untuk stage, atau koordinat panel untuk `SlotKind::Workspace`).
#[derive(Debug, Clone, Copy)]
pub struct DropSlot {
    pub number: usize,
    pub kind: SlotKind,
    pub mark: RECT,
}

/// Tebal garis penanda "penuh lebar".
const BAR: i32 = 6;
/// Zona tepi atas / bawah strip (persen tinggi window di tepi itu, minimal piksel).
const EDGE_PCT: i32 = 22;
const EDGE_MIN: i32 = 10;
const EXPAND_TOP: i32 = 24;
const EXPAND_BOTTOM: i32 = 16;

fn same_slot(a: &Option<DropSlot>, b: &Option<DropSlot>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.number == b.number && a.kind == b.kind,
        _ => false,
    }
}

/// Area di lajur seberang `x` (sejajar dengannya) di dalam kolom `col`.
fn beside_rect(x: &DrawItem, col: &RECT) -> RECT {
    let (l, r) = if x.span == Span::Right {
        (col.left, x.rect.left - MARGIN)
    } else {
        (x.rect.right + MARGIN, col.right)
    };
    RECT {
        left: l,
        top: x.rect.top,
        right: r.max(l + 8),
        bottom: x.rect.bottom,
    }
}

/// Titik tujuan untuk window `src` yang dijatuhkan di titik panel `(x, y)`.
/// Cek kartu dock bawah dulu (pindah workspace); kalau tidak, cek kolom di stage atas.
pub fn slot_at(d: &Draw, x: i32, y: i32, src: u64) -> Option<DropSlot> {
    // 1. Dijatuhkan di kartu workspace di dock bawah -> pindah ke workspace itu
    if let Some(card) = d.dock.iter().find(|c| point_in(&c.rect, x, y)) {
        if !card.current {
            return Some(DropSlot {
                number: card.number,
                kind: SlotKind::Workspace,
                mark: card.rect,
            });
        }
        return None;
    }

    // 2. Dijatuhkan di stage atas (konversi y panel -> y konten)
    let cy = y + d.scroll_y;
    let col = d.columns.iter().find(|c| {
        x >= c.rect.left
            && x < c.rect.right
            && cy >= c.rect.top - EXPAND_TOP
            && cy < c.rect.bottom + EXPAND_BOTTOM
    })?;
    let in_col = |it: &&DrawItem| {
        point_in(
            &col.rect,
            (it.rect.left + it.rect.right) / 2,
            (it.rect.top + it.rect.bottom) / 2,
        )
    };
    let src_here = d.items.iter().any(|it| it.id == src && in_col(&it));
    let items: Vec<&DrawItem> = d
        .items
        .iter()
        .filter(|it| it.id != src)
        .filter(in_col)
        .collect();

    if src_here
        && d.items
            .iter()
            .any(|it| it.id == src && point_in(&it.rect, x, cy))
    {
        return None;
    }

    // Workspace kosong di stage: seluruh area kolom jadi target pindah workspace
    if items.is_empty() {
        return Some(DropSlot {
            number: col.number,
            kind: SlotKind::Workspace,
            mark: col.rect,
        });
    }

    let top_item = items
        .iter()
        .copied()
        .min_by_key(|it| (it.rect.top, it.rect.left))?;
    let bottom_item = items
        .iter()
        .copied()
        .max_by_key(|it| (it.rect.bottom, -it.rect.left))?;
    let edge = |it: &DrawItem| ((it.rect.bottom - it.rect.top) * EDGE_PCT / 100).max(EDGE_MIN);
    let bar = |yy: i32| RECT {
        left: col.rect.left,
        top: yy - BAR / 2,
        right: col.rect.right,
        bottom: yy + BAR / 2,
    };

    let (kind, mark) = if cy < top_item.rect.top + edge(top_item) {
        (
            SlotKind::Before(top_item.id),
            bar(top_item.rect.top - MARGIN),
        )
    } else if cy >= bottom_item.rect.bottom - edge(bottom_item) {
        (
            SlotKind::After(bottom_item.id),
            bar(bottom_item.rect.bottom + MARGIN),
        )
    } else {
        let over = items
            .iter()
            .copied()
            .filter(|it| cy >= it.rect.top && cy < it.rect.bottom)
            .min_by_key(|it| {
                if x < it.rect.left {
                    it.rect.left - x
                } else if x >= it.rect.right {
                    x - it.rect.right + 1
                } else {
                    0
                }
            });
        let near = over.or_else(|| {
            items.iter().copied().min_by_key(|it| {
                if cy < it.rect.top {
                    it.rect.top - cy
                } else {
                    cy - it.rect.bottom + 1
                }
            })
        })?;
        let r = near.rect;
        let inside = point_in(&r, x, cy);
        match near.span {
            Span::Full if inside && clean_full(&items, near) => {
                let mid = (r.left + r.right) / 2;
                let first = x < mid;
                let mark = RECT {
                    left: if first { r.left } else { mid },
                    right: if first { mid } else { r.right },
                    top: r.top,
                    bottom: r.bottom,
                };
                (SlotKind::Split(near.id, first), mark)
            }
            Span::Full => {
                if cy < (r.top + r.bottom) / 2 {
                    (SlotKind::Before(near.id), bar(r.top - MARGIN))
                } else {
                    (SlotKind::After(near.id), bar(r.bottom + MARGIN))
                }
            }
            _ if inside => {
                let mid = (r.top + r.bottom) / 2;
                let first = cy < mid;
                let mark = RECT {
                    left: r.left,
                    right: r.right,
                    top: if first { r.top } else { mid },
                    bottom: if first { mid } else { r.bottom },
                };
                (SlotKind::Split(near.id, first), mark)
            }
            _ => (SlotKind::Beside(near.id), beside_rect(near, &col.rect)),
        }
    };
    Some(DropSlot {
        number: col.number,
        kind,
        mark,
    })
}

fn clean_full(items: &[&DrawItem], x: &DrawItem) -> bool {
    const ADJ: i32 = 10;
    !items.iter().any(|o| {
        o.id != x.id
            && o.span != Span::Full
            && ((o.rect.top < x.rect.top && o.rect.bottom + ADJ >= x.rect.top)
                || (o.rect.bottom > x.rect.bottom && o.rect.top <= x.rect.bottom + ADJ))
    })
}

fn slot_for(x: i32, y: i32, src: u64) -> Option<DropSlot> {
    let g = DRAW.lock().ok()?;
    slot_at(g.as_ref()?, x, y, src)
}

/// Hasilkan semua kandidat `DropSlot` untuk mode pick-and-drop keyboard (`Space`):
/// semua slot di stage saat ini + kartu workspace lain di dock bawah.
pub fn drop_slots(d: &Draw, src: u64) -> Vec<DropSlot> {
    let mut out: Vec<DropSlot> = Vec::new();
    let push_unique = |out: &mut Vec<DropSlot>, sl: DropSlot| {
        if !out
            .iter()
            .any(|e| e.number == sl.number && e.kind == sl.kind)
        {
            out.push(sl);
        }
    };

    for col in &d.columns {
        let mut items: Vec<&DrawItem> = d
            .items
            .iter()
            .filter(|it| {
                it.id != src
                    && point_in(
                        &col.rect,
                        (it.rect.left + it.rect.right) / 2,
                        (it.rect.top + it.rect.bottom) / 2,
                    )
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        items.sort_by_key(|it| (it.rect.top, it.rect.left));

        let mut blocks: Vec<(i32, i32, Vec<&DrawItem>)> = Vec::new();
        for &it in &items {
            if let Some(last) = blocks.last_mut() {
                if it.rect.top < last.1 - 2 {
                    last.1 = last.1.max(it.rect.bottom);
                    last.2.push(it);
                    continue;
                }
            }
            blocks.push((it.rect.top, it.rect.bottom, vec![it]));
        }

        let bar = |yy: i32| RECT {
            left: col.rect.left,
            top: yy - BAR / 2,
            right: col.rect.right,
            bottom: yy + BAR / 2,
        };

        if let Some((top, _, b_items)) = blocks.first() {
            push_unique(
                &mut out,
                DropSlot {
                    number: col.number,
                    kind: SlotKind::Before(b_items[0].id),
                    mark: bar(*top - MARGIN),
                },
            );
        }

        for (bi, (_, b_bot, b_items)) in blocks.iter().enumerate() {
            for &it in b_items {
                let r = it.rect;
                if it.span == Span::Full {
                    if clean_full(&items, it) {
                        let mid = (r.left + r.right) / 2;
                        push_unique(
                            &mut out,
                            DropSlot {
                                number: col.number,
                                kind: SlotKind::Split(it.id, true),
                                mark: RECT {
                                    left: r.left,
                                    top: r.top,
                                    right: mid,
                                    bottom: r.bottom,
                                },
                            },
                        );
                        push_unique(
                            &mut out,
                            DropSlot {
                                number: col.number,
                                kind: SlotKind::Split(it.id, false),
                                mark: RECT {
                                    left: mid,
                                    top: r.top,
                                    right: r.right,
                                    bottom: r.bottom,
                                },
                            },
                        );
                    }
                } else {
                    let mid = (r.top + r.bottom) / 2;
                    push_unique(
                        &mut out,
                        DropSlot {
                            number: col.number,
                            kind: SlotKind::Split(it.id, true),
                            mark: RECT {
                                left: r.left,
                                top: r.top,
                                right: r.right,
                                bottom: mid,
                            },
                        },
                    );
                    push_unique(
                        &mut out,
                        DropSlot {
                            number: col.number,
                            kind: SlotKind::Split(it.id, false),
                            mark: RECT {
                                left: r.left,
                                top: mid,
                                right: r.right,
                                bottom: r.bottom,
                            },
                        },
                    );
                    let has_opposite = b_items.iter().any(|o| {
                        o.id != it.id
                            && o.span != Span::Full
                            && o.span != it.span
                            && o.rect.top < r.bottom
                            && o.rect.bottom > r.top
                    });
                    if !has_opposite {
                        push_unique(
                            &mut out,
                            DropSlot {
                                number: col.number,
                                kind: SlotKind::Beside(it.id),
                                mark: beside_rect(it, &col.rect),
                            },
                        );
                    }
                }
            }

            let bar_y = if let Some((next_top, _, _)) = blocks.get(bi + 1) {
                (*b_bot + *next_top) / 2
            } else {
                *b_bot + MARGIN
            };
            push_unique(
                &mut out,
                DropSlot {
                    number: col.number,
                    kind: SlotKind::After(b_items[0].id),
                    mark: bar(bar_y),
                },
            );
        }
    }

    // Kartu workspace lain di dock bawah
    for card in &d.dock {
        if !card.current {
            push_unique(
                &mut out,
                DropSlot {
                    number: card.number,
                    kind: SlotKind::Workspace,
                    mark: card.rect,
                },
            );
        }
    }
    out
}

/// Slot awal saat mengangkat `src`: yang paling dekat dengan posisi `src`.
pub fn initial_drop_slot(slots: &[DropSlot], d: &Draw, src: u64) -> usize {
    let Some(sr) = d.items.iter().find(|it| it.id == src).map(|it| it.rect) else {
        return 0;
    };
    let (sx, sy) = ((sr.left + sr.right) / 2, (sr.top + sr.bottom) / 2);
    slots
        .iter()
        .enumerate()
        .min_by_key(|(_, s)| {
            let is_ws = s.kind == SlotKind::Workspace;
            let cx = (s.mark.left + s.mark.right) / 2;
            let cy = (s.mark.top + s.mark.bottom) / 2;
            (is_ws, (cx - sx).abs() + (cy - sy).abs())
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Geser penanda `DropSlot` ke arah `dir`.
pub fn step_drop_slot(slots: &[DropSlot], cur: usize, dir: layout::Dir) -> Option<usize> {
    let f = slots.get(cur)?;
    let (fx0, fx1) = (f.mark.left, f.mark.right);
    let (fy0, fy1) = (f.mark.top, f.mark.bottom);
    let (fcx, fcy) = ((fx0 + fx1) / 2, (fy0 + fy1) / 2);
    let is_bar = |k: SlotKind| matches!(k, SlotKind::Before(_) | SlotKind::After(_));

    if f.kind == SlotKind::Workspace {
        return match dir {
            layout::Dir::Left => slots
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == SlotKind::Workspace && c.mark.left < f.mark.left)
                .max_by_key(|(_, c)| c.mark.left)
                .map(|(i, _)| i),
            layout::Dir::Right => slots
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind == SlotKind::Workspace && c.mark.left > f.mark.left)
                .min_by_key(|(_, c)| c.mark.left)
                .map(|(i, _)| i),
            layout::Dir::Up => slots
                .iter()
                .enumerate()
                .filter(|(_, c)| c.kind != SlotKind::Workspace)
                .max_by_key(|(_, c)| c.mark.bottom)
                .map(|(i, _)| i),
            layout::Dir::Down => None,
        };
    }

    match dir {
        layout::Dir::Up | layout::Dir::Down => {
            let mut best: Option<((bool, i32, i32, i32), usize)> = None;
            for (i, c) in slots.iter().enumerate() {
                if i == cur || c.kind == SlotKind::Workspace || c.number != f.number {
                    continue;
                }
                let (cx0, cx1) = (c.mark.left, c.mark.right);
                let (ccx, ccy) = ((cx0 + cx1) / 2, (c.mark.top + c.mark.bottom) / 2);
                let h_ov = (fx1.min(cx1) - fx0.max(cx0)).max(0);
                let key = match dir {
                    layout::Dir::Up if ccy < fcy - 1 => {
                        Some((h_ov == 0, fcy - ccy, (ccx - fcx).abs(), c.mark.left))
                    }
                    layout::Dir::Down if ccy > fcy + 1 => {
                        Some((h_ov == 0, ccy - fcy, (ccx - fcx).abs(), c.mark.left))
                    }
                    _ => None,
                };
                if let Some(k) = key {
                    if best.map_or(true, |(bk, _)| k < bk) {
                        best = Some((k, i));
                    }
                }
            }
            if let Some((_, i)) = best {
                return Some(i);
            }
            // Dari paling bawah stage -> turun ke kartu dock workspace
            if dir == layout::Dir::Down {
                return slots
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.kind == SlotKind::Workspace)
                    .min_by_key(|(_, c)| ((c.mark.left + c.mark.right) / 2 - fcx).abs())
                    .map(|(i, _)| i);
            }
            None
        }
        layout::Dir::Left | layout::Dir::Right => {
            let mut same: Option<((i32, i32, i32), usize)> = None;
            for (i, c) in slots.iter().enumerate() {
                if i == cur
                    || c.kind == SlotKind::Workspace
                    || c.number != f.number
                    || is_bar(c.kind)
                {
                    continue;
                }
                let (ccx, ccy) = (
                    (c.mark.left + c.mark.right) / 2,
                    (c.mark.top + c.mark.bottom) / 2,
                );
                let v_ov = (fy1.min(c.mark.bottom) - fy0.max(c.mark.top)).max(0);
                if v_ov <= 0 {
                    continue;
                }
                let key = match dir {
                    layout::Dir::Left if ccx < fcx - 1 => {
                        Some((fcx - ccx, (ccy - fcy).abs(), c.mark.top))
                    }
                    layout::Dir::Right if ccx > fcx + 1 => {
                        Some((ccx - fcx, (ccy - fcy).abs(), c.mark.top))
                    }
                    _ => None,
                };
                if let Some(k) = key {
                    if same.map_or(true, |(bk, _)| k < bk) {
                        same = Some((k, i));
                    }
                }
            }
            same.map(|(_, i)| i)
        }
    }
}

fn lparam_xy(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

const ANIM_TIMER_ID: usize = 1;
const DRAG_TIMER_ID: usize = 2;
const ANIM_TICK_MS: u32 = 16;
const ENTER_MS: f32 = 130.0;
const SLIDE_PX: i32 = 10;
/// Lama hover di kartu dock workspace saat drag sebelum stage otomatis pindah (ms).
const DOCK_HOVER_MS: f32 = 220.0;
/// Tinggi zona tepi atas / bawah stage untuk auto-scroll saat drag (piksel).
const EDGE_SCROLL_ZONE: i32 = 64;

struct EnterAnim {
    start: Instant,
    target: Rect,
}

struct ThumbEntry {
    handle: isize,
    item_idx: usize,
    src_size: SIZE,
}

static ENTER_ANIM: Mutex<Option<EnterAnim>> = Mutex::new(None);
static THUMBS: Mutex<Vec<ThumbEntry>> = Mutex::new(Vec::new());
static OV_HWND: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);
static FONT_BOLD: AtomicIsize = AtomicIsize::new(0);
static FONT_CAPTION: AtomicIsize = AtomicIsize::new(0);
static FONT_MINI: AtomicIsize = AtomicIsize::new(0);
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static VISIBLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Faktor zoom stage (f32 disimpan sebagai u32 bits; 1.0 = bawaan).
static STAGE_ZOOM: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0f32

const ZOOM_MIN: f32 = 0.45;
const ZOOM_MAX: f32 = 1.80;
const ZOOM_STEP: f32 = 0.12;

/// Faktor zoom stage saat ini (`ZOOM_MIN..=ZOOM_MAX`).
pub fn stage_zoom() -> f32 {
    f32::from_bits(STAGE_ZOOM.load(Ordering::Relaxed)).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Ubah zoom stage sebesar `delta` (+ untuk perbesar, - untuk perkecil).
/// Mengembalikan true kalau nilai berubah.
pub fn adjust_zoom(delta: f32) -> bool {
    let cur = stage_zoom();
    let next = ((cur + delta) * 100.0).round() / 100.0;
    let clamped = next.clamp(ZOOM_MIN, ZOOM_MAX);
    if (clamped - cur).abs() < 0.005 {
        return false;
    }
    STAGE_ZOOM.store(clamped.to_bits(), Ordering::Relaxed);
    true
}

/// Kembalikan zoom stage ke 1.0 (100%).
pub fn reset_zoom() -> bool {
    let cur = stage_zoom();
    if (cur - 1.0).abs() < 0.005 {
        return false;
    }
    STAGE_ZOOM.store(1.0f32.to_bits(), Ordering::Relaxed);
    true
}

/// Posisi tengah scroll stage saat ini sebagai fraksi (0.0..=1.0) dari tinggi strip,
/// agar zoom in/out tetap bertahan di titik yang sedang dilihat.
pub fn current_scroll_frac() -> Option<f32> {
    let g = DRAW.lock().ok()?;
    let d = g.as_ref()?;
    let col = d.columns.first()?;
    let strip_h = (col.rect.bottom - col.rect.top).max(1) as f32;
    let stage_h = (d.stage.bottom - d.stage.top).max(1) as f32;
    Some(((d.scroll_y as f32 + stage_h * 0.5) / strip_h).clamp(0.0, 1.0))
}

impl Draw {
    /// Terapkan kembali fraksi posisi scroll (0.0..=1.0) agar zoom tidak melompat ke
    /// posisi window aktif.
    pub fn apply_scroll_frac(&mut self, frac: f32) {
        if self.max_scroll <= 0 {
            self.scroll_y = 0;
            return;
        }
        let Some(col) = self.columns.first() else {
            return;
        };
        let strip_h = (col.rect.bottom - col.rect.top).max(1) as f32;
        let stage_h = (self.stage.bottom - self.stage.top).max(1) as f32;
        let target = (frac.clamp(0.0, 1.0) * strip_h - stage_h * 0.5).round() as i32;
        self.scroll_y = target.clamp(0, self.max_scroll);
    }
}

const MARGIN: i32 = 5;
const LABEL_BAND: i32 = 30;
const PAD: i32 = 22;
const HINT_H: i32 = 36;
const DOCK_H: i32 = 116;
const DOCK_CARD_W: i32 = 150;
const DOCK_CARD_H: i32 = 90;
const DOCK_LABEL_H: i32 = 22;
const DOCK_GAP: i32 = 14;
const SCROLLBAR_GUTTER: i32 = 14;
const MINIMAP_GAP: i32 = 16;
const MINIMAP_PAD: i32 = 10;
/// Lebar minimum panel supaya teks petunjuk tidak terpotong.
const MIN_PANEL_W_WITH_HINT: i32 = 460;
const MIN_PANEL_W_WITH_HINT_WS: i32 = 600;
/// Kartu workspace yang terlihat sekaligus di dock bawah.
pub const MAX_VISIBLE: usize = 5;

/// Data panel: peta tiap workspace yang berisi window dan bagian mana yang terlihat.
pub struct Model {
    /// (nomor workspace mulai 0, petanya), urut dari kiri.
    pub maps: Vec<(usize, Overview)>,
    /// Kartu dock terlihat: `maps[first .. first + visible]`.
    pub first: usize,
    pub visible: usize,
    /// Indeks di `maps` workspace yang sedang ditampilkan besar di stage atas.
    pub big: usize,
    /// Nomor workspace yang aktif di desktop.
    #[allow(dead_code)]
    pub active_ws: usize,
}

/// Jendela workspace yang terlihat di dock bawah untuk `columns` workspace dengan fokus `big`.
pub fn window_for(columns: usize, big: usize) -> (usize, usize) {
    let visible = columns.min(MAX_VISIBLE);
    let first = big.saturating_sub(visible / 2).min(columns - visible);
    (first, visible)
}

/// `first` baru supaya workspace `sel` terlihat di dock bawah.
pub fn scroll(first: usize, visible: usize, sel: usize) -> usize {
    if sel < first {
        sel
    } else if sel >= first + visible {
        sel + 1 - visible
    } else {
        first
    }
}

/// Hitung `scroll_y` agar kartu `target` terlihat nyaman di dalam `stage`.
fn ensure_visible_scroll(stage: RECT, current_scroll: i32, max_scroll: i32, target: RECT) -> i32 {
    if max_scroll <= 0 {
        return 0;
    }
    let pad = 12;
    let mut s = current_scroll.clamp(0, max_scroll);
    let vis_top = target.top - s;
    let vis_bot = target.bottom - s;
    if vis_top < stage.top + pad {
        s = target.top - (stage.top + pad);
    } else if vis_bot > stage.bottom - pad {
        s = target.bottom - (stage.bottom - pad);
    }
    s.clamp(0, max_scroll)
}

/// Bangun layout Task View Hybrid:
///  - Stage atas menampilkan workspace `model.big` dengan skala besar & scrollable vertikal.
///  - Dock bawah (saat `multi`) menampilkan bar kartu mini untuk semua workspace terlihat.
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
    let multi = model.maps.len() > 1;
    let hint_h = if opts.hint { HINT_H } else { 0 };
    let dock_h = if multi { DOCK_H } else { 0 };
    let min_w = if !opts.hint {
        0
    } else if multi {
        MIN_PANEL_W_WITH_HINT_WS
    } else {
        MIN_PANEL_W_WITH_HINT
    };

    let panel_w = (work.w * pct / 100).max(min_w).min(work.w);
    let panel_h = (work.h * pct / 100).min(work.h);
    let panel = Rect {
        x: work.x + (work.w - panel_w) / 2,
        y: work.y + (work.h - panel_h) / 2,
        w: panel_w,
        h: panel_h,
    };

    let content_w = (panel_w - 2 * PAD).max(80);
    let show_minimap = content_w >= 480;
    let minimap_w = if show_minimap {
        (content_w * 20 / 100).clamp(156, 220)
    } else {
        0
    };
    let stage_right = if show_minimap {
        (panel_w - PAD - minimap_w - MINIMAP_GAP).max(PAD + 80)
    } else {
        (panel_w - PAD).max(PAD + 80)
    };
    let stage = RECT {
        left: PAD,
        top: PAD,
        right: stage_right,
        bottom: (panel_h - PAD - hint_h - dock_h).max(PAD + 80),
    };
    let stage_w = stage.right - stage.left;
    let stage_h = stage.bottom - stage.top;

    let mut items = Vec::new();
    let mut columns = Vec::new();
    let mut minimap = None;
    let mut max_scroll = 0;
    let mut scroll_y = 0;

    if let Some((ws, ov)) = model.maps.get(model.big).or_else(|| model.maps.first()) {
        let sw = ov.width.max(1) as f32;
        let vh = ov.view_h.max(1) as f32;
        // Skala stage didasarkan pada ukuran 1 layar (viewport) dikalikan faktor zoom
        // pengguna (Ctrl+Wheel / +/-/0).
        let avail_w = (stage_w - SCROLLBAR_GUTTER).max(40) as f32;
        let base_scale = ((avail_w * 0.82) / sw).min(stage_h as f32 / (vh * 1.38));
        let zoom = stage_zoom();
        let scale = (base_scale * zoom).min(avail_w / sw);
        let sc = |v: i32| (v as f32 * scale).round() as i32;

        let col_w = sc(ov.width).max(1);
        let strip_h = sc(ov.height).max(1);
        let cx = stage.left + ((stage_w - SCROLLBAR_GUTTER) - col_w) / 2;
        let top = if strip_h <= stage_h {
            stage.top + (stage_h - strip_h) / 2
        } else {
            stage.top
        };
        max_scroll = (strip_h - stage_h).max(0);

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
                span: if it.rect.w * 100 >= ov.width * 90 {
                    Span::Full
                } else if it.rect.x * 100 < ov.width * 12 {
                    Span::Left
                } else {
                    Span::Right
                },
            });
        }

        let vt = top + sc(ov.view_y);
        columns.push(DrawColumn {
            number: ws + 1,
            rect: RECT {
                left: cx,
                top,
                right: cx + col_w,
                bottom: top + strip_h,
            },
            view: RECT {
                left: cx,
                top: vt,
                right: cx + col_w,
                bottom: vt + sc(ov.view_h),
            },
        });

        let init_scroll = sc(ov.view_y).clamp(0, max_scroll);
        scroll_y = if let Some(sel_it) = items.iter().find(|it| it.id == selected) {
            ensure_visible_scroll(stage, init_scroll, max_scroll, sel_it.rect)
        } else {
            init_scroll
        };

        // Bangun minimap penuh 100% strip di sisi kanan stage
        if show_minimap {
            let mrect = RECT {
                left: stage.right + MINIMAP_GAP,
                top: stage.top,
                right: panel_w - PAD,
                bottom: stage.bottom,
            };
            let mw_avail = (mrect.right - mrect.left - 2 * MINIMAP_PAD).max(1) as f32;
            let mh_avail = (mrect.bottom - mrect.top - 2 * MINIMAP_PAD).max(1) as f32;
            let ms = (mw_avail / sw).min(mh_avail / ov.height.max(1) as f32);
            let msc = |v: i32| (v as f32 * ms).round() as i32;
            let m_strip_w = msc(ov.width).max(1);
            let m_strip_h = msc(ov.height).max(1);
            let mx0 = mrect.left + ((mrect.right - mrect.left) - m_strip_w) / 2;
            let my0 = mrect.top + ((mrect.bottom - mrect.top) - m_strip_h) / 2;

            let m_items = ov
                .items
                .iter()
                .map(|it| {
                    let l = mx0 + msc(it.rect.x) + 2;
                    let t = my0 + msc(it.rect.y) + 2;
                    let r = mx0 + msc(it.rect.x + it.rect.w) - 2;
                    let b = my0 + msc(it.rect.y + it.rect.h) - 2;
                    MinimapItem {
                        id: it.id.0,
                        rect: RECT {
                            left: l,
                            top: t,
                            right: r.max(l + 3),
                            bottom: b.max(t + 3),
                        },
                        title: titles.get(&it.id.0).cloned().unwrap_or_default(),
                    }
                })
                .collect();

            let m_view = RECT {
                left: mx0,
                top: my0 + msc(ov.view_y),
                right: mx0 + m_strip_w,
                bottom: my0 + msc(ov.view_y + ov.view_h),
            };
            minimap = Some(DrawMinimap {
                rect: mrect,
                strip_rect: RECT {
                    left: mx0,
                    top: my0,
                    right: mx0 + m_strip_w,
                    bottom: my0 + m_strip_h,
                },
                view_rect: m_view,
                items: m_items,
            });
        }
    }

    // Dock workspace di bagian bawah (persis bar Desktop di Task View Windows 11)
    let mut dock = Vec::new();
    if multi {
        let end = (model.first + model.visible).min(model.maps.len());
        let start = model.first.min(end);
        let count = (end - start) as i32;
        if count > 0 {
            let avail_dock_w = (panel_w - 2 * PAD).max(1);
            let card_w = ((avail_dock_w - (count - 1) * DOCK_GAP) / count).clamp(72, DOCK_CARD_W);
            let total_w = count * card_w + (count - 1) * DOCK_GAP;
            let mut dx = (panel_w - total_w) / 2;
            let dy = stage.bottom + 16;

            for i in start..end {
                let (ws, ov) = &model.maps[i];
                let rect = RECT {
                    left: dx,
                    top: dy,
                    right: dx + card_w,
                    bottom: dy + DOCK_CARD_H,
                };
                let preview = RECT {
                    left: rect.left + 8,
                    top: rect.top + 8,
                    right: rect.right - 8,
                    bottom: rect.bottom - DOCK_LABEL_H - 4,
                };
                let pw = (preview.right - preview.left).max(1) as f32;
                let ph = (preview.bottom - preview.top).max(1) as f32;
                let ms = (pw / ov.width.max(1) as f32).min(ph / ov.height.max(1) as f32);
                let mw = (ov.width as f32 * ms).round() as i32;
                let mh = (ov.height as f32 * ms).round() as i32;
                let ox = preview.left + ((preview.right - preview.left) - mw) / 2;
                let oy = preview.top + ((preview.bottom - preview.top) - mh) / 2;

                let mini_items = ov
                    .items
                    .iter()
                    .map(|it| {
                        let l = ox + (it.rect.x as f32 * ms).round() as i32 + 1;
                        let t = oy + (it.rect.y as f32 * ms).round() as i32 + 1;
                        let r = ox + ((it.rect.x + it.rect.w) as f32 * ms).round() as i32 - 1;
                        let b = oy + ((it.rect.y + it.rect.h) as f32 * ms).round() as i32 - 1;
                        RECT {
                            left: l,
                            top: t,
                            right: r.max(l + 2),
                            bottom: b.max(t + 2),
                        }
                    })
                    .collect();

                dock.push(DrawDockCard {
                    number: ws + 1,
                    ws_index: *ws,
                    rect,
                    preview,
                    mini_items,
                    current: i == model.big,
                });
                dx += card_w + DOCK_GAP;
            }
        }
    }

    Draw {
        panel,
        stage,
        scroll_y,
        max_scroll,
        items,
        columns,
        minimap,
        dock,
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

/// Hitung posisi tujuan & sumber thumbnail DWM yang terpotong rapi di batas `stage`.
fn clipped_thumb_rects(
    item_rect: RECT,
    scroll_y: i32,
    stage: RECT,
    src_size: SIZE,
) -> Option<(RECT, RECT)> {
    let r = shift_y(item_rect, -scroll_y);
    let area = RECT {
        left: r.left + 8,
        top: r.top + 8,
        right: r.right - 8,
        bottom: r.bottom - 4 - LABEL_BAND,
    };
    if area.right - area.left < 40 || area.bottom - area.top < 30 {
        return None;
    }
    let dest = fit(area, src_size.cx, src_size.cy);
    let vis_top = dest.top.max(stage.top);
    let vis_bot = dest.bottom.min(stage.bottom);
    if vis_bot - vis_top < 4 {
        return None;
    }
    let vis_dest = RECT {
        left: dest.left,
        top: vis_top,
        right: dest.right,
        bottom: vis_bot,
    };
    let dh = (dest.bottom - dest.top).max(1) as f32;
    let src_h = src_size.cy.max(1) as f32;
    let sy0 = (((vis_top - dest.top) as f32 / dh) * src_h).round() as i32;
    let sy1 = (((vis_bot - dest.top) as f32 / dh) * src_h).round() as i32;
    let vis_src = RECT {
        left: 0,
        top: sy0.clamp(0, src_size.cy.max(1) - 1),
        right: src_size.cx.max(1),
        bottom: sy1.clamp(1, src_size.cy.max(1)),
    };
    Some((vis_dest, vis_src))
}

/// Pasang thumbnail DWM untuk tiap kotak yang cukup besar di stage.
unsafe fn register_thumbs(dest: HWND, draw: &mut Draw, init_opacity: u8) -> Vec<ThumbEntry> {
    let mut entries = Vec::new();
    for (idx, it) in draw.items.iter_mut().enumerate() {
        if it.source == 0 {
            continue;
        }
        let inner = RECT {
            left: it.rect.left + 4,
            top: it.rect.top + 4,
            right: it.rect.right - 4,
            bottom: it.rect.bottom - 4,
        };
        let area_w = (inner.right - 4) - (inner.left + 4);
        let area_h = (inner.bottom - LABEL_BAND) - (inner.top + 4);
        if area_w < 40 || area_h < 30 {
            continue;
        }
        let Ok(thumb) = DwmRegisterThumbnail(dest, HWND(it.source as *mut _)) else {
            continue;
        };
        let src_size = DwmQueryThumbnailSourceSize(thumb).unwrap_or(SIZE { cx: 0, cy: 0 });
        let props = match clipped_thumb_rects(it.rect, draw.scroll_y, draw.stage, src_size) {
            Some((rc_dest, rc_src)) => DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_RECTDESTINATION
                    | DWM_TNP_RECTSOURCE
                    | DWM_TNP_VISIBLE
                    | DWM_TNP_OPACITY,
                rcDestination: rc_dest,
                rcSource: rc_src,
                opacity: init_opacity,
                fVisible: BOOL(1),
                fSourceClientAreaOnly: BOOL(0),
            },
            None => DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                opacity: init_opacity,
                fVisible: BOOL(0),
                ..Default::default()
            },
        };
        if DwmUpdateThumbnailProperties(thumb, &props).is_err() {
            let _ = DwmUnregisterThumbnail(thumb);
            continue;
        }
        entries.push(ThumbEntry {
            handle: thumb,
            item_idx: idx,
            src_size,
        });
        it.label_band = Some(RECT {
            left: inner.left,
            top: inner.bottom - LABEL_BAND,
            right: inner.right,
            bottom: inner.bottom,
        });
    }
    entries
}

/// Perbarui posisi & pemotongan seluruh thumbnail DWM setelah `scroll_y` berubah.
unsafe fn sync_thumbs_scroll(d: &Draw) {
    if let Ok(thumbs) = THUMBS.lock() {
        for th in thumbs.iter() {
            let Some(it) = d.items.get(th.item_idx) else {
                continue;
            };
            let props = match clipped_thumb_rects(it.rect, d.scroll_y, d.stage, th.src_size) {
                Some((rc_dest, rc_src)) => DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE | DWM_TNP_VISIBLE,
                    rcDestination: rc_dest,
                    rcSource: rc_src,
                    fVisible: BOOL(1),
                    ..Default::default()
                },
                None => DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_VISIBLE,
                    fVisible: BOOL(0),
                    ..Default::default()
                },
            };
            let _ = DwmUpdateThumbnailProperties(th.handle, &props);
        }
    }
}

unsafe fn unregister_thumbs() {
    if let Ok(mut t) = THUMBS.lock() {
        for th in t.drain(..) {
            let _ = DwmUnregisterThumbnail(th.handle);
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

unsafe fn tick_enter_anim(hwnd: HWND) {
    let step = if let Ok(mut g) = ENTER_ANIM.lock() {
        if let Some(a) = g.as_ref() {
            let t = (a.start.elapsed().as_secs_f32() * 1000.0 / ENTER_MS).min(1.0);
            let e = theme::ease_fluent_out(t);
            let y = a.target.y + ((SLIDE_PX as f32) * (1.0 - e)).round() as i32;
            let opacity = (255.0 * e).round() as u8;
            let res = (a.target.x, y, opacity, t >= 1.0);
            if t >= 1.0 {
                *g = None;
            }
            Some(res)
        } else {
            None
        }
    } else {
        None
    };
    if let Some((x, y, opacity, done)) = step {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE,
        );
        if let Ok(thumbs) = THUMBS.lock() {
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY,
                opacity,
                ..Default::default()
            };
            for th in thumbs.iter() {
                let _ = DwmUpdateThumbnailProperties(th.handle, &props);
            }
        }
        if done {
            let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
        }
    } else {
        let _ = KillTimer(Some(hwnd), ANIM_TIMER_ID);
    }
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
        let pal = Palette::resolve(colors);
        let mut canvas = Canvas::new(hdc, client);
        canvas.fill_rect(client, pal.surface, 255);

        let kb_slot = KB_PICK
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|p| p.slots.get(p.idx).copied()));
        let drop_slot = DRAG
            .lock()
            .ok()
            .and_then(|g| g.as_ref().filter(|d| d.active).and_then(|d| d.slot))
            .or(kb_slot);

        let font = HFONT(FONT.load(Ordering::Relaxed) as *mut _);
        let font_bold = HFONT(FONT_BOLD.load(Ordering::Relaxed) as *mut _);
        let font_cap = HFONT(FONT_CAPTION.load(Ordering::Relaxed) as *mut _);
        let font_mini = HFONT(FONT_MINI.load(Ordering::Relaxed) as *mut _);

        if let Some(d) = guard.as_ref() {
            // 1. Stage atas (dipotong tepat di batas `d.stage` agar scroll rapi)
            canvas.set_clip(d.stage);

            for it in &d.items {
                let r = shift_y(it.rect, -d.scroll_y);
                if r.bottom < d.stage.top || r.top > d.stage.bottom {
                    continue;
                }
                let selected = it.id == d.selected;
                let card_fill = if selected {
                    theme::mix(pal.card_hover, pal.selected, 10)
                } else {
                    pal.card
                };
                canvas.fill_round_rect(r, 8.0, card_fill, 255);
                if selected {
                    canvas.stroke_round_rect(r, 8.0, pal.selected, 255, 2.5);
                } else {
                    canvas.stroke_round_rect(r, 8.0, pal.stroke, 220, 1.0);
                }

                if !it.title.is_empty() {
                    let inner = RECT {
                        left: r.left + 4,
                        top: r.top + 4,
                        right: (r.right - 4).max(r.left + 5),
                        bottom: (r.bottom - 4).max(r.top + 5),
                    };
                    match it.label_band {
                        Some(band) => {
                            let b = shift_y(band, -d.scroll_y);
                            let tr = RECT {
                                left: b.left + 8,
                                top: b.top,
                                right: (b.right - 8).max(b.left + 9),
                                bottom: b.bottom,
                            };
                            canvas.draw_text(
                                &it.title,
                                tr,
                                font,
                                pal.text,
                                DT_LEFT
                                    | DT_VCENTER
                                    | DT_SINGLELINE
                                    | DT_END_ELLIPSIS
                                    | DT_NOPREFIX,
                            );
                        }
                        None => {
                            let tr = RECT {
                                left: inner.left + 8,
                                top: inner.top,
                                right: (inner.right - 8).max(inner.left + 9),
                                bottom: inner.bottom,
                            };
                            canvas.draw_text(
                                &it.title,
                                tr,
                                font,
                                pal.text,
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

            // Bingkai bagian strip yang sedang tampil di layar (`view`)
            for c in &d.columns {
                let v = shift_y(c.view, -d.scroll_y);
                if d.items.is_empty() {
                    canvas.fill_round_rect(v, 10.0, pal.layer, 180);
                    let empty_drop = drop_slot.map_or(false, |sl| {
                        sl.kind == SlotKind::Workspace && sl.number == c.number
                    });
                    canvas.stroke_round_rect(
                        v,
                        10.0,
                        if empty_drop { pal.selected } else { pal.stroke },
                        230,
                        if empty_drop { 2.5 } else { 1.5 },
                    );
                    let msg = if empty_drop {
                        "Drop window here"
                    } else {
                        "Empty workspace"
                    };
                    canvas.draw_text(
                        msg,
                        v,
                        font,
                        pal.text_muted,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                } else {
                    canvas.stroke_round_rect(v, 8.0, pal.view, 220, 2.0);
                }
            }

            // Penanda drop di dalam stage
            if let Some(sl) = drop_slot {
                if sl.kind != SlotKind::Workspace {
                    let m = shift_y(sl.mark, -d.scroll_y);
                    match sl.kind {
                        SlotKind::Beside(_) | SlotKind::Split(..) => {
                            canvas.fill_round_rect(m, 8.0, pal.selected, 95);
                            canvas.stroke_round_rect(m, 8.0, pal.selected, 255, 2.5);
                        }
                        SlotKind::Before(_) | SlotKind::After(_) => {
                            canvas.fill_round_rect(m, 3.0, pal.selected, 255);
                        }
                        SlotKind::Workspace => {}
                    }
                }
            }

            // Scrollbar tipis khas Windows 11 kalau strip lebih tinggi dari stage
            if d.max_scroll > 0 {
                let stage_h = (d.stage.bottom - d.stage.top).max(1);
                let total_h = stage_h + d.max_scroll;
                let thumb_h = ((stage_h * stage_h) / total_h).clamp(28, stage_h);
                let track_h = (stage_h - thumb_h).max(1);
                let ty = d.stage.top + (d.scroll_y * track_h) / d.max_scroll;
                let bar = RECT {
                    left: d.stage.right - 5,
                    top: ty,
                    right: d.stage.right - 1,
                    bottom: ty + thumb_h,
                };
                canvas.fill_round_rect(bar, 2.0, pal.stroke_strong, 200);
            }

            // Lencana persentase zoom halus (hanya tampil kalau bukan 100%)
            let z = stage_zoom();
            if (z - 1.0).abs() >= 0.01 {
                let badge = RECT {
                    left: d.stage.left + 4,
                    top: d.stage.top + 4,
                    right: d.stage.left + 58,
                    bottom: d.stage.top + 28,
                };
                canvas.fill_round_rect(badge, 6.0, pal.control, 235);
                canvas.stroke_round_rect(badge, 6.0, pal.stroke_strong, 200, 1.0);
                canvas.draw_text(
                    &format!("{}%", (z * 100.0).round() as i32),
                    badge,
                    font_mini,
                    pal.text_secondary,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
            }

            canvas.reset_clip();

            // 1b. Minimap penuh 100% strip di sisi kanan stage (peta posisi semua window)
            if let Some(mm) = d.minimap.as_ref() {
                canvas.fill_round_rect(mm.rect, 10.0, pal.layer, 255);
                canvas.stroke_round_rect(mm.rect, 10.0, pal.stroke, 220, 1.0);

                // Slider transparan penanda area yang sedang terlihat di stage besar
                if d.max_scroll > 0 {
                    let stage_h = (d.stage.bottom - d.stage.top).max(1) as f32;
                    let total_h = stage_h + d.max_scroll as f32;
                    let ms_h = (mm.strip_rect.bottom - mm.strip_rect.top).max(1) as f32;
                    let sy0 = mm.strip_rect.top
                        + ((d.scroll_y as f32 / total_h) * ms_h).round() as i32;
                    let sh = ((stage_h / total_h) * ms_h).round().max(12.0) as i32;
                    let slider = RECT {
                        left: mm.strip_rect.left - 3,
                        top: sy0.max(mm.strip_rect.top - 2),
                        right: mm.strip_rect.right + 3,
                        bottom: (sy0 + sh).min(mm.strip_rect.bottom + 2),
                    };
                    canvas.fill_round_rect(slider, 5.0, pal.text, if pal.dark { 22 } else { 18 });
                }

                for it in &mm.items {
                    let sel = it.id == d.selected;
                    let fill = if sel {
                        theme::mix(pal.card_hover, pal.selected, 22)
                    } else {
                        pal.card
                    };
                    canvas.fill_round_rect(it.rect, 4.0, fill, 255);
                    if sel {
                        canvas.stroke_round_rect(it.rect, 4.0, pal.selected, 255, 1.8);
                    } else {
                        canvas.stroke_round_rect(it.rect, 4.0, pal.stroke, 200, 1.0);
                    }
                    let ih = it.rect.bottom - it.rect.top;
                    let iw = it.rect.right - it.rect.left;
                    if !it.title.is_empty() && ih >= 15 && iw >= 30 {
                        let tr = RECT {
                            left: it.rect.left + 4,
                            top: it.rect.top,
                            right: it.rect.right - 4,
                            bottom: it.rect.bottom,
                        };
                        canvas.draw_text(
                            &it.title,
                            tr,
                            font_mini,
                            if sel { pal.text } else { pal.text_secondary },
                            DT_CENTER
                                | DT_VCENTER
                                | DT_SINGLELINE
                                | DT_END_ELLIPSIS
                                | DT_NOPREFIX,
                        );
                    }
                }

                // Bingkai `view` (area layar aktif di desktop) di atas minimap
                canvas.stroke_round_rect(mm.view_rect, 4.0, pal.view, 220, 1.5);
            }

            // 2. Bilah kartu workspace di bagian bawah (gaya Task View Windows 11)
            if d.multi {
                let sep = RECT {
                    left: PAD,
                    top: d.stage.bottom + 7,
                    right: client.right - PAD,
                    bottom: d.stage.bottom + 8,
                };
                canvas.fill_rect(sep, pal.divider, 255);

                for card in &d.dock {
                    let drop_hi = drop_slot.map_or(false, |sl| {
                        sl.kind == SlotKind::Workspace && sl.number == card.number
                    });
                    let fill = if drop_hi {
                        theme::mix(pal.layer, pal.selected, 28)
                    } else if card.current {
                        pal.card_hover
                    } else {
                        pal.layer
                    };
                    canvas.fill_round_rect(card.rect, 8.0, fill, 255);
                    if drop_hi || card.current {
                        canvas.stroke_round_rect(card.rect, 8.0, pal.selected, 255, 2.0);
                    } else {
                        canvas.stroke_round_rect(card.rect, 8.0, pal.stroke, 220, 1.0);
                    }

                    // Miniatur wireframe window di dalam kartu workspace (kosong = dibiarkan polos)
                    canvas.fill_round_rect(card.preview, 4.0, pal.surface, 180);
                    for mr in &card.mini_items {
                        canvas.fill_round_rect(
                            *mr,
                            2.0,
                            if card.current {
                                theme::mix(pal.control, pal.accent, 22)
                            } else {
                                pal.control
                            },
                            255,
                        );
                    }

                    let label = format!("Workspace {}", card.number);
                    let lr = RECT {
                        left: card.rect.left + 6,
                        top: card.rect.bottom - DOCK_LABEL_H - 2,
                        right: card.rect.right - 6,
                        bottom: card.rect.bottom - 2,
                    };
                    canvas.draw_text(
                        &label,
                        lr,
                        if card.current { font_bold } else { font_cap },
                        if card.current {
                            pal.text
                        } else {
                            pal.text_secondary
                        },
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );
                }
            }

            // 3. Petunjuk tombol di paling bawah (kalau aktif)
            if d.hint {
                let hint_text = if kb_slot.is_some() {
                    "←↑↓→ move marker  ·  Enter drop  ·  Esc cancel"
                } else {
                    "←↑↓→ select  ·  Space pick up  ·  Enter jump  ·  Esc cancel"
                };
                let hr = RECT {
                    left: client.left,
                    top: client.bottom - HINT_H,
                    right: client.right,
                    bottom: client.bottom - 4,
                };
                canvas.draw_text(
                    hint_text,
                    hr,
                    font_cap,
                    pal.text_muted,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
            }
        }
        canvas.present();
    }
    let _ = EndPaint(hwnd, &ps);
}

/// Geser scroll stage berdasarkan posisi klik/seret di minimap (opsional ikut menyorot
/// window di titik tersebut).
unsafe fn scrub_minimap(hwnd: HWND, x: i32, y: i32, pick_selection: bool) {
    let changed = if let Ok(mut g) = DRAW.lock() {
        if let Some(d) = g.as_mut() {
            if let Some(mm) = d.minimap.as_ref() {
                let mut dirty = false;
                if pick_selection {
                    if let Some(it) = mm.items.iter().find(|it| point_in(&it.rect, x, y)) {
                        if d.selected != it.id {
                            d.selected = it.id;
                            dirty = true;
                        }
                    }
                }
                if d.max_scroll > 0 {
                    let ms_h = (mm.strip_rect.bottom - mm.strip_rect.top).max(1) as f32;
                    let stage_h = (d.stage.bottom - d.stage.top).max(1) as f32;
                    let total_h = stage_h + d.max_scroll as f32;
                    let frac = ((y - mm.strip_rect.top) as f32 / ms_h).clamp(0.0, 1.0);
                    let target_scroll = (frac * total_h - stage_h * 0.5).round() as i32;
                    let next = target_scroll.clamp(0, d.max_scroll);
                    if next != d.scroll_y {
                        d.scroll_y = next;
                        sync_thumbs_scroll(d);
                        dirty = true;
                    }
                }
                dirty
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };
    if changed {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Hitung langkah auto-scroll stage (piksel per tick) berdasarkan posisi y kursor saat drag.
fn drag_edge_scroll_delta(stage: RECT, y: i32, scroll_y: i32, max_scroll: i32) -> i32 {
    if max_scroll <= 0 {
        return 0;
    }
    let top_end = stage.top + EDGE_SCROLL_ZONE;
    let bot_start = stage.bottom - EDGE_SCROLL_ZONE;
    if y >= stage.top && y < top_end && scroll_y > 0 {
        let ratio = (top_end - y) as f32 / EDGE_SCROLL_ZONE as f32;
        -((8.0 + 18.0 * ratio).round() as i32)
    } else if y > bot_start && y <= stage.bottom && scroll_y < max_scroll {
        let ratio = (y - bot_start) as f32 / EDGE_SCROLL_ZONE as f32;
        (8.0 + 18.0 * ratio).round() as i32
    } else {
        0
    }
}

/// Tick timer saat drag window aktif:
///  1. Auto-scroll stage saat kursor mendekati tepi atas / bawah stage.
///  2. Hover di atas kartu workspace di dock bawah -> otomatis pindah stage ke workspace itu.
unsafe fn tick_drag(hwnd: HWND) {
    let info = DRAG.lock().ok().and_then(|g| {
        let d = g.as_ref().filter(|d| d.active)?;
        Some((d.id, d.pos, d.hover_dock))
    });
    let Some((src, (x, y), hover_dock)) = info else {
        let _ = KillTimer(Some(hwnd), DRAG_TIMER_ID);
        return;
    };

    let mut need_repaint = false;
    let mut switch_ws: Option<usize> = None;

    // 1. Cek hover dock -> pindah workspace setelah DOCK_HOVER_MS
    if let Some((ws_idx, since)) = hover_dock {
        let elapsed = since.elapsed().as_secs_f32() * 1000.0;
        if elapsed >= DOCK_HOVER_MS {
            switch_ws = Some(ws_idx);
            if let Ok(mut g) = DRAG.lock() {
                if let Some(d) = g.as_mut() {
                    d.hover_dock = None;
                }
            }
        }
    }

    // 2. Auto-scroll stage kalau kursor di zona tepi atas / bawah stage
    let mut scrolled = false;
    if let Ok(mut g) = DRAW.lock() {
        if let Some(d) = g.as_mut() {
            if x >= d.stage.left && x < d.stage.right {
                let delta = drag_edge_scroll_delta(d.stage, y, d.scroll_y, d.max_scroll);
                if delta != 0 {
                    let next = (d.scroll_y + delta).clamp(0, d.max_scroll);
                    if next != d.scroll_y {
                        d.scroll_y = next;
                        sync_thumbs_scroll(d);
                        scrolled = true;
                        need_repaint = true;
                    }
                }
            }
        }
    }

    if scrolled {
        let new_slot = slot_for(x, y, src);
        if let Ok(mut g) = DRAG.lock() {
            if let Some(d) = g.as_mut() {
                d.slot = new_slot;
            }
        }
    }

    if let Some(ws) = switch_ws {
        post(WM_OV_WS, 2 + ws);
    }
    if need_repaint {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn ov_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER if wparam.0 == ANIM_TIMER_ID => {
            tick_enter_anim(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DRAG_TIMER_ID => {
            tick_drag(hwnd);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as u16) as i16 as i32;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            if ctrl {
                let step = if delta > 0 { ZOOM_STEP } else { -ZOOM_STEP };
                if adjust_zoom(step) {
                    post(WM_OV_ZOOM, 0);
                }
                return LRESULT(0);
            }
            let changed = if let Ok(mut g) = DRAW.lock() {
                if let Some(d) = g.as_mut() {
                    if d.max_scroll > 0 {
                        let next = (d.scroll_y - delta / 2).clamp(0, d.max_scroll);
                        if next != d.scroll_y {
                            d.scroll_y = next;
                            sync_thumbs_scroll(d);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if changed {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;

            let picking = KB_PICK.lock().ok().map_or(false, |g| g.is_some());
            if picking {
                if vk == VK_ESCAPE.0 {
                    if let Ok(mut g) = KB_PICK.lock() {
                        *g = None;
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if vk == VK_RETURN.0 || (vk == VK_SPACE.0 && !ctrl && !alt && !shift) {
                    let chosen = KB_PICK.lock().ok().and_then(|mut g| {
                        let p = g.take()?;
                        p.slots.get(p.idx).copied().map(|sl| (p.src, sl))
                    });
                    if let Some(info) = chosen {
                        if let Ok(mut d) = DROP.lock() {
                            *d = Some(info);
                        }
                        post(WM_OV_DROP, 0);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if let Some(dir) = match vk {
                    x if x == VK_LEFT.0 && !ctrl => Some(layout::Dir::Left),
                    x if x == VK_RIGHT.0 && !ctrl => Some(layout::Dir::Right),
                    x if x == VK_UP.0 && !ctrl => Some(layout::Dir::Up),
                    x if x == VK_DOWN.0 && !ctrl => Some(layout::Dir::Down),
                    _ => None,
                } {
                    let mut target_mark = None;
                    if let Ok(mut g) = KB_PICK.lock() {
                        if let Some(p) = g.as_mut() {
                            if let Some(next) = step_drop_slot(&p.slots, p.idx, dir) {
                                p.idx = next;
                                if let Some(sl) = p.slots.get(next) {
                                    if sl.kind != SlotKind::Workspace {
                                        target_mark = Some(sl.mark);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(mark) = target_mark {
                        if let Ok(mut g) = DRAW.lock() {
                            if let Some(d) = g.as_mut() {
                                let ns = ensure_visible_scroll(d.stage, d.scroll_y, d.max_scroll, mark);
                                if ns != d.scroll_y {
                                    d.scroll_y = ns;
                                    sync_thumbs_scroll(d);
                                }
                            }
                        }
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if (0x31..=0x39).contains(&vk) {
                    let target_num = (vk - 0x31) as usize + 1;
                    if let Ok(mut g) = KB_PICK.lock() {
                        if let Some(p) = g.as_mut() {
                            if let Some(i) = p.slots.iter().position(|s| s.number == target_num) {
                                p.idx = i;
                            }
                        }
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                return LRESULT(0);
            }

            if vk == VK_SPACE.0 && !ctrl && !alt && !shift {
                let started = DRAW.lock().ok().and_then(|g| {
                    let d = g.as_ref()?;
                    let src = d.selected;
                    let slots = drop_slots(d, src);
                    if slots.is_empty() {
                        return None;
                    }
                    let idx = initial_drop_slot(&slots, d, src);
                    Some(KbPick { src, slots, idx })
                });
                if let Some(p) = started {
                    if let Ok(mut g) = KB_PICK.lock() {
                        *g = Some(p);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else if ctrl && vk == VK_LEFT.0 {
                post(WM_OV_WS, 0);
            } else if ctrl && vk == VK_RIGHT.0 {
                post(WM_OV_WS, 1);
            } else if (0x31..=0x39).contains(&vk) {
                post(WM_OV_WS, 2 + (vk - 0x31) as usize);
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
                let dragging = DRAG.lock().ok().map_or(false, |g| g.is_some());
                if dragging {
                    let _ = KillTimer(Some(hwnd), DRAG_TIMER_ID);
                    if let Ok(mut g) = DRAG.lock() {
                        *g = None;
                    }
                    let _ = ReleaseCapture();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else {
                    post(WM_OV_CANCEL, 0);
                }
            } else if vk == VK_TAB.0 {
                let back = shift;
                post(WM_OV_CYCLE, if back { 0 } else { 1 });
            } else if vk == 0xBB || vk == 0x6B {
                // '+' / '=' atau Numpad '+' -> zoom in
                if adjust_zoom(ZOOM_STEP) {
                    post(WM_OV_ZOOM, 0);
                }
            } else if vk == 0xBD || vk == 0x6D {
                // '-' atau Numpad '-' -> zoom out
                if adjust_zoom(-ZOOM_STEP) {
                    post(WM_OV_ZOOM, 0);
                }
            } else if vk == 0x30 || vk == 0x60 {
                // '0' atau Numpad '0' -> reset zoom ke 100%
                if reset_zoom() {
                    post(WM_OV_ZOOM, 0);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_xy(lparam);
            let in_mm = DRAW
                .lock()
                .ok()
                .and_then(|g| g.as_ref()?.minimap.as_ref().map(|m| point_in(&m.rect, x, y)))
                .unwrap_or(false);
            if in_mm {
                MINIMAP_DRAG.store(true, Ordering::Relaxed);
                let _ = SetCapture(hwnd);
                scrub_minimap(hwnd, x, y, true);
                return LRESULT(0);
            }
            if let (_, Some(id)) = hit_at(x, y, 0) {
                if let Ok(mut g) = DRAG.lock() {
                    *g = Some(Drag {
                        id,
                        start: (x, y),
                        pos: (x, y),
                        active: false,
                        slot: None,
                        hover_dock: None,
                    });
                }
                let _ = SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_xy(lparam);
            if MINIMAP_DRAG.load(Ordering::Relaxed) {
                scrub_minimap(hwnd, x, y, false);
                return LRESULT(0);
            }
            let src = DRAG.lock().ok().and_then(|g| g.as_ref().map(|d| d.id));
            if let Some(src) = src {
                let slot = slot_for(x, y, src);
                let hovered_ws = DRAW.lock().ok().and_then(|g| {
                    g.as_ref()?
                        .dock
                        .iter()
                        .find(|c| !c.current && point_in(&c.rect, x, y))
                        .map(|c| c.ws_index)
                });
                let mut repaint = false;
                let mut start_timer = false;
                if let Ok(mut g) = DRAG.lock() {
                    if let Some(d) = g.as_mut() {
                        d.pos = (x, y);
                        if !d.active
                            && ((x - d.start.0).abs() > DRAG_START
                                || (y - d.start.1).abs() > DRAG_START)
                        {
                            d.active = true;
                            start_timer = true;
                            repaint = true;
                        }
                        if d.active {
                            if !same_slot(&d.slot, &slot) {
                                d.slot = slot;
                                repaint = true;
                            }
                            match (d.hover_dock, hovered_ws) {
                                (Some((cur, _)), Some(ws)) if cur == ws => {}
                                (_, Some(ws)) => {
                                    d.hover_dock = Some((ws, Instant::now()));
                                    repaint = true;
                                }
                                (Some(_), None) => {
                                    d.hover_dock = None;
                                    repaint = true;
                                }
                                (None, None) => {}
                            }
                        }
                    }
                }
                if start_timer {
                    let _ = SetTimer(Some(hwnd), DRAG_TIMER_ID, ANIM_TICK_MS, None);
                }
                if repaint {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let active = DRAG
                .lock()
                .ok()
                .map_or(false, |g| g.as_ref().map_or(false, |d| d.active));
            if active {
                if let Ok(c) = LoadCursorW(None, IDC_SIZEALL) {
                    let _ = SetCursor(Some(c));
                }
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CAPTURECHANGED => {
            let _ = KillTimer(Some(hwnd), DRAG_TIMER_ID);
            MINIMAP_DRAG.store(false, Ordering::Relaxed);
            if let Ok(mut g) = DRAG.lock() {
                *g = None;
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_xy(lparam);
            if MINIMAP_DRAG.swap(false, Ordering::Relaxed) {
                let _ = ReleaseCapture();
                return LRESULT(0);
            }
            let _ = KillTimer(Some(hwnd), DRAG_TIMER_ID);
            let drag = DRAG.lock().ok().and_then(|mut g| g.take());
            let _ = ReleaseCapture();
            if let Some(d) = drag.filter(|d| d.active) {
                if let Some(slot) = slot_for(x, y, d.id) {
                    if let Ok(mut g) = DROP.lock() {
                        *g = Some((d.id, slot));
                    }
                    post(WM_OV_DROP, 0);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            // Klik kartu dock bawah -> pindah workspace di overview
            let (dock_idx, item_id) = hit_at(x, y, 0);
            if let Some(di) = dock_idx {
                let ws_idx = DRAW
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref()?.dock.get(di).map(|c| c.ws_index));
                if let Some(ws) = ws_idx {
                    post(WM_OV_WS, 2 + ws);
                }
                return LRESULT(0);
            }
            match item_id {
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

    let pal = Palette::resolve(OverviewColors::default());
    theme::apply_window_chrome(hwnd, &pal, false);

    FONT.store(theme::create_font(-17, 400).0 as isize, Ordering::Relaxed);
    FONT_BOLD.store(theme::create_font(-14, 600).0 as isize, Ordering::Relaxed);
    FONT_CAPTION.store(theme::create_font(-14, 400).0 as isize, Ordering::Relaxed);
    FONT_MINI.store(theme::create_font(-11, 500).0 as isize, Ordering::Relaxed);
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
    let was_visible = VISIBLE.load(Ordering::Relaxed);
    let animate = !was_visible && animations_enabled();
    let pal = Palette::resolve(draw.colors);
    theme::apply_window_chrome(h, &pal, false);

    unregister_thumbs();
    let handles = if thumbnails {
        register_thumbs(h, &mut draw, if animate { 0 } else { 255 })
    } else {
        Vec::new()
    };
    if let Ok(mut t) = THUMBS.lock() {
        *t = handles;
    }
    let p = draw.panel;
    if let Ok(mut g) = KB_PICK.lock() {
        *g = None;
    }
    if let Ok(mut g) = DRAW.lock() {
        *g = Some(draw);
    }
    VISIBLE.store(true, Ordering::Relaxed);
    let init_y = if animate { p.y + SLIDE_PX } else { p.y };
    if animate {
        if let Ok(mut a) = ENTER_ANIM.lock() {
            *a = Some(EnterAnim {
                start: Instant::now(),
                target: p,
            });
        }
    } else {
        let _ = KillTimer(Some(h), ANIM_TIMER_ID);
        if let Ok(mut a) = ENTER_ANIM.lock() {
            *a = None;
        }
    }
    let _ = SetWindowPos(h, Some(HWND_TOPMOST), p.x, init_y, p.w, p.h, SWP_SHOWWINDOW);
    let _ = SetForegroundWindow(h);
    let _ = InvalidateRect(Some(h), None, false);
    if animate {
        let _ = SetTimer(Some(h), ANIM_TIMER_ID, ANIM_TICK_MS, None);
    }
}

/// Ganti window yang sedang dipilih (kursor overview) dan scroll stage agar terlihat.
pub unsafe fn set_selected(id: u64) {
    if let Ok(mut g) = DRAW.lock() {
        if let Some(d) = g.as_mut() {
            d.selected = id;
            if let Some(it) = d.items.iter().find(|it| it.id == id) {
                let ns = ensure_visible_scroll(d.stage, d.scroll_y, d.max_scroll, it.rect);
                if ns != d.scroll_y {
                    d.scroll_y = ns;
                    sync_thumbs_scroll(d);
                }
            }
        }
    }
    let _ = InvalidateRect(Some(hwnd()), None, false);
}

pub unsafe fn hide() {
    VISIBLE.store(false, Ordering::Relaxed);
    unregister_thumbs();
    if OV_HWND.load(Ordering::Relaxed) != 0 {
        let h = hwnd();
        let _ = KillTimer(Some(h), ANIM_TIMER_ID);
        let _ = KillTimer(Some(h), DRAG_TIMER_ID);
        let _ = ShowWindow(h, SW_HIDE);
    }
    if let Ok(mut a) = ENTER_ANIM.lock() {
        *a = None;
    }
    if let Ok(mut g) = KB_PICK.lock() {
        *g = None;
    }
    if let Ok(mut g) = DRAW.lock() {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::rgb;
    use crate::theme::{luminance, mix};
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
    fn boxes_fit_inside_the_stage_and_do_not_overlap() {
        let d = build_with(&opts(true, 70));
        assert_eq!(d.items.len(), 3);
        for it in &d.items {
            assert!(it.rect.left >= d.stage.left && it.rect.top >= d.stage.top);
            assert!(it.rect.right <= d.stage.right && it.rect.bottom <= d.stage.bottom);
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
    fn tall_strip_stays_readable_and_shows_full_minimap_alongside_stage() {
        let mut o = ov();
        o.height = 5400; // 5 layar tinggi
        let d = build(
            &single(&o),
            &HashMap::new(),
            &HashMap::new(),
            1,
            1,
            WORK,
            &opts(true, 70),
        );
        assert!(d.max_scroll > 0);
        // Lebar kolom stage tetap besar (tidak menciut gara-gara tinggi total 5400px)
        let col_w = d.columns[0].rect.right - d.columns[0].rect.left;
        assert!(col_w > 500, "col_w = {col_w}");
        // Minimap samping menampilkan 100% strip di dalam kotaknya
        let mm = d.minimap.as_ref().expect("minimap must exist");
        assert!(mm.rect.left > d.stage.right);
        assert!(mm.strip_rect.top >= mm.rect.top && mm.strip_rect.bottom <= mm.rect.bottom);
        assert_eq!(mm.items.len(), 3);
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
        o.height = 2160;
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
        assert!(view.top > d.items[0].rect.top);
    }

    #[test]
    fn fit_keeps_aspect_ratio_and_stays_inside() {
        let area = RECT {
            left: 100,
            top: 100,
            right: 500,
            bottom: 300,
        };
        let r = fit(area, 1920, 1080);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        assert!(r.left >= area.left && r.right <= area.right);
        assert!(r.top >= area.top && r.bottom <= area.bottom);
        assert!((w as f32 / h as f32 - 1920.0 / 1080.0).abs() < 0.05);
        assert_eq!(h, 200);
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
        assert!(luminance(rgb(20, 22, 28)) < 150);
        let lighter = mix(rgb(40, 40, 40), 0xFF_FFFF, 20);
        assert!(luminance(lighter) > luminance(rgb(40, 40, 40)));
        assert_eq!(mix(rgb(10, 20, 30), rgb(200, 200, 200), 0), rgb(10, 20, 30));
    }

    fn build_model(m: &Model) -> Draw {
        build(m, &HashMap::new(), &HashMap::new(), 1, 1, WORK, &opts(true, 70))
    }

    #[test]
    fn multi_workspace_shows_bottom_dock_cards_and_single_stage_column() {
        let d = build_model(&model(3, 1));
        assert!(d.multi);
        assert_eq!(d.columns.len(), 1);
        assert_eq!(d.dock.len(), 3);
        assert!(!d.dock[0].current && d.dock[1].current && !d.dock[2].current);
        for c in &d.dock {
            assert!(c.rect.top >= d.stage.bottom);
            assert!(c.rect.bottom <= d.panel.h);
        }
    }

    #[test]
    fn panel_size_does_not_depend_on_which_workspace_is_selected() {
        let a = build_model(&model(4, 0));
        let b = build_model(&model(4, 3));
        assert_eq!((a.panel.w, a.panel.h), (b.panel.w, b.panel.h));
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
    fn single_workspace_has_no_bottom_dock() {
        let d = build_with(&opts(true, 70));
        assert!(!d.multi);
        assert!(d.dock.is_empty());
        assert_eq!(d.columns.len(), 1);
    }

    // ---- titik drop ----

    fn item(id: u64, span: Span, l: i32, t: i32, r: i32, b: i32) -> DrawItem {
        DrawItem {
            id,
            rect: RECT { left: l, top: t, right: r, bottom: b },
            title: String::new(),
            source: 0,
            label_band: None,
            span,
        }
    }

    fn column(number: usize, l: i32, t: i32, r: i32, b: i32) -> DrawColumn {
        let rect = RECT { left: l, top: t, right: r, bottom: b };
        DrawColumn { number, rect, view: rect }
    }

    fn dock_card(number: usize, current: bool, l: i32, t: i32, r: i32, b: i32) -> DrawDockCard {
        let rect = RECT { left: l, top: t, right: r, bottom: b };
        DrawDockCard {
            number,
            ws_index: number - 1,
            rect,
            preview: rect,
            mini_items: Vec::new(),
            current,
        }
    }

    /// Stage workspace 1: A (kiri, atas), B (penuh, tengah), C (kiri, bawah).
    /// Dock bawah: Workspace 1 (current) & Workspace 2.
    fn drop_draw() -> Draw {
        Draw {
            panel: Rect { x: 0, y: 0, w: 600, h: 600 },
            stage: RECT { left: 0, top: 40, right: 600, bottom: 460 },
            scroll_y: 0,
            max_scroll: 0,
            items: vec![
                item(1, Span::Left, 3, 53, 97, 203),
                item(2, Span::Full, 3, 203, 197, 303),
                item(3, Span::Left, 3, 303, 97, 447),
            ],
            columns: vec![column(1, 0, 50, 200, 450)],
            minimap: None,
            dock: vec![
                dock_card(1, true, 50, 480, 190, 570),
                dock_card(2, false, 210, 480, 350, 570),
            ],
            multi: true,
            selected: 3,
            focused: 3,
            hint: false,
            colors: OverviewColors::default(),
        }
    }

    fn kind(d: &Draw, x: i32, y: i32) -> Option<(usize, SlotKind)> {
        slot_at(d, x, y, 3).map(|s| (s.number, s.kind))
    }

    #[test]
    fn dropping_on_a_half_width_window_stacks_in_its_lane() {
        let d = drop_draw();
        assert_eq!(kind(&d, 50, 100), Some((1, SlotKind::Split(1, true))));
        assert_eq!(kind(&d, 50, 180), Some((1, SlotKind::Split(1, false))));
    }

    #[test]
    fn the_empty_half_next_to_a_window_is_filled_beside_it() {
        let d = drop_draw();
        assert_eq!(kind(&d, 150, 128), Some((1, SlotKind::Beside(1))));
    }

    #[test]
    fn dropping_on_another_workspace_dock_card_targets_that_workspace() {
        let d = drop_draw();
        assert_eq!(kind(&d, 280, 520), Some((2, SlotKind::Workspace)));
        // kartu workspace saat ini tidak jadi target drop
        assert_eq!(kind(&d, 120, 520), None);
    }

    #[test]
    fn drop_slots_include_stage_slots_and_other_workspace_dock_cards() {
        let d = drop_draw();
        let slots = drop_slots(&d, 3);
        assert!(slots.iter().any(|s| s.kind == SlotKind::Before(1)));
        assert!(slots.iter().any(|s| s.kind == SlotKind::Split(1, true)));
        assert!(slots.iter().any(|s| s.kind == SlotKind::Beside(1)));
        assert!(slots
            .iter()
            .any(|s| s.number == 2 && s.kind == SlotKind::Workspace));
    }

    #[test]
    fn drag_edge_scroll_scrolls_up_near_top_and_down_near_bottom() {
        let stage = RECT {
            left: 20,
            top: 20,
            right: 800,
            bottom: 620,
        };
        assert!(drag_edge_scroll_delta(stage, 30, 100, 400) < 0);
        assert!(drag_edge_scroll_delta(stage, 610, 100, 400) > 0);
        assert_eq!(drag_edge_scroll_delta(stage, 320, 100, 400), 0);
        // sudah mentok atas / bawah -> 0
        assert_eq!(drag_edge_scroll_delta(stage, 30, 0, 400), 0);
        assert_eq!(drag_edge_scroll_delta(stage, 610, 400, 400), 0);
    }
}
