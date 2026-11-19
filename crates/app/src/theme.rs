//! Native Windows 11-style theme and drawing primitives (Fluent Design / WinUI 3):
//!  - Read dark/light mode (`AppsUseLightTheme`) and accent color (`AccentColor`)
//!    directly from the Windows registry.
//!  - Enable rounded corners (`DWMWCP_ROUND`), DWM dark mode, a subtle 1px border,
//!    and native dark right-click menus (`uxtheme.dll`).
//!  - Double-buffered + anti-aliased GDI+ canvas for rounded rectangles, pills, and
//!    smooth lines without flicker, plus `Segoe UI Variable` (fallback to `Segoe UI`).
//!  - Fluent Decelerate animation curve (`cubic-bezier(0.1, 0.9, 0.2, 1.0)`).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, DeleteDC,
    DeleteObject, DrawTextW, EnumFontFamiliesExW, GetDC, IntersectClipRect, ReleaseDC,
    SelectClipRgn, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
    DEFAULT_PITCH, DRAW_TEXT_FORMAT, ENUMLOGFONTEXW, FF_SWISS, HBITMAP, HDC, HFONT, HGDIOBJ,
    LOGFONTW, SRCCOPY, TEXTMETRICW, TRANSPARENT,
};
use windows::Win32::Graphics::GdiPlus::{
    GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen,
    GdipDrawPath, GdipFillEllipse, GdipFillPath, GdipFillRectangle, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GpBrush,
    GpGraphics, GpPath, GpPen, GpSolidFill, PixelOffsetModeHalf, SmoothingModeAntiAlias,
    UnitPixel, FillModeAlternate,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

use crate::config::{rgb, OverviewColors};

static GDIPLUS_TOKEN: AtomicUsize = AtomicUsize::new(0);
static HAS_SEGOE_VAR: AtomicBool = AtomicBool::new(false);

/// Initialize GDI+, check for the `Segoe UI Variable` font, and enable automatic
/// dark/light theming for Win32 popup menus (`TrackPopupMenu`). Call once from `main`.
pub unsafe fn init() {
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        ..Default::default()
    };
    let mut token: usize = 0;
    if GdiplusStartup(&mut token, &input, std::ptr::null_mut()).0 == 0 {
        GDIPLUS_TOKEN.store(token, Ordering::Relaxed);
    }
    HAS_SEGOE_VAR.store(font_exists("Segoe UI Variable Text"), Ordering::Relaxed);
    enable_native_dark_menus();
}

/// Clean up GDI+ when the application exits.
pub unsafe fn shutdown() {
    let token = GDIPLUS_TOKEN.swap(0, Ordering::Relaxed);
    if token != 0 {
        GdiplusShutdown(token);
    }
}

/// Enable automatic Windows 11 dark/light mode for context menus (`uxtheme.dll`
/// ordinal 135 = `SetPreferredAppMode(AllowDark = 1)`, ordinal 136 = `FlushMenuThemes`).
unsafe fn enable_native_dark_menus() {
    let Ok(ux) = LoadLibraryW(w!("uxtheme.dll")) else {
        return;
    };
    type SetPreferredAppModeFn = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemesFn = unsafe extern "system" fn();
    if let Some(f) = GetProcAddress(ux, windows::core::PCSTR(135usize as *const u8)) {
        let set_mode: SetPreferredAppModeFn = std::mem::transmute(f);
        let _ = set_mode(1); // AllowDark: follow the Windows system theme.
    }
    if let Some(f) = GetProcAddress(ux, windows::core::PCSTR(136usize as *const u8)) {
        let flush: FlushMenuThemesFn = std::mem::transmute(f);
        flush();
    }
}

unsafe extern "system" fn enum_font_cb(
    _elf: *const LOGFONTW,
    _ntm: *const TEXTMETRICW,
    _font_type: u32,
    lparam: windows::Win32::Foundation::LPARAM,
) -> i32 {
    *(lparam.0 as *mut bool) = true;
    0 // Only need to know that at least one exists.
}

unsafe fn font_exists(name: &str) -> bool {
    let hdc = GetDC(None);
    if hdc.is_invalid() {
        return false;
    }
    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfCharSet = DEFAULT_CHARSET;
    for (i, ch) in name.encode_utf16().take(31).enumerate() {
        lf.lfFaceName[i] = ch;
    }
    let mut found = false;
    let _ = EnumFontFamiliesExW(
        hdc,
        &lf,
        Some(enum_font_cb),
        windows::Win32::Foundation::LPARAM(&mut found as *mut bool as isize),
        0,
    );
    let _ = ReleaseDC(None, hdc);
    let _ = std::mem::size_of::<ENUMLOGFONTEXW>();
    found
}

/// Create native Windows 11 fonts (`Segoe UI Variable Display` for large headings,
/// `Segoe UI Variable Text` for regular text; automatically falls back to `Segoe UI` on Windows 10).
pub unsafe fn create_font(height: i32, weight: i32) -> HFONT {
    let mut lf: LOGFONTW = std::mem::zeroed();
    lf.lfHeight = height;
    lf.lfWeight = weight;
    lf.lfQuality = CLEARTYPE_QUALITY;
    lf.lfPitchAndFamily = (DEFAULT_PITCH.0 | FF_SWISS.0) as u8;
    let face = if HAS_SEGOE_VAR.load(Ordering::Relaxed) {
        if height.abs() >= 24 {
            "Segoe UI Variable Display"
        } else {
            "Segoe UI Variable Text"
        }
    } else {
        "Segoe UI"
    };
    for (i, ch) in face.encode_utf16().take(31).enumerate() {
        lf.lfFaceName[i] = ch;
    }
    CreateFontIndirectW(&lf)
}

// ---------------------------------------------------------------- colors & theme

fn channels(c: u32) -> (u32, u32, u32) {
    (c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF)
}

/// Blend color `c` toward `toward` by `pct` percent (0..=100).
pub fn mix(c: u32, toward: u32, pct: u32) -> u32 {
    let p = pct.min(100);
    let (r, g, b) = channels(c);
    let (tr, tg, tb) = channels(toward);
    let f = |a: u32, t: u32| (a * (100 - p) + t * p) / 100;
    rgb(f(r, tr), f(g, tg), f(b, tb))
}

/// Color brightness for a COLORREF (0-255).
pub fn luminance(c: u32) -> u32 {
    let (r, g, b) = channels(c);
    (r * 299 + g * 587 + b * 114) / 1000
}

/// Convert `COLORREF` (0x00BBGGRR) + alpha to GDI+ `ARGB` format (0xAARRGGBB).
pub fn argb(c: u32, alpha: u8) -> u32 {
    let (r, g, b) = channels(c);
    ((alpha as u32) << 24) | (r << 16) | (g << 8) | b
}

unsafe fn read_reg_dword(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    let status = RegGetValueW(
        HKEY_CURRENT_USER,
        subkey,
        value,
        RRF_RT_REG_DWORD,
        None,
        Some(&mut data as *mut u32 as *mut c_void),
        Some(&mut size),
    );
    if status.is_ok() {
        Some(data)
    } else {
        None
    }
}

/// True if Windows is currently using Dark Mode for the application.
pub fn is_dark_mode() -> bool {
    unsafe {
        read_reg_dword(
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
        )
        .map_or(true, |v| v == 0)
    }
}

/// Windows accent color (COLORREF 0x00BBGGRR) from Personalization -> Colors settings.
pub fn system_accent() -> Option<u32> {
    unsafe {
        read_reg_dword(w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor"))
            .map(|abgr| abgr & 0x00FF_FFFF)
    }
}

/// Complete Windows 11 Fluent Design color palette.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Main flyout / panel surface.
    pub surface: u32,
    /// Background layer over the surface (e.g. workspace columns).
    pub layer: u32,
    /// Window card / box fill.
    pub card: u32,
    /// Card fill when hovered.
    pub card_hover: u32,
    /// Small control fill (keycaps, inactive dots, idle snap zones).
    pub control: u32,
    /// Subtle 1px Fluent Design border.
    pub stroke: u32,
    /// Stronger border for active controls / keycaps.
    pub stroke_strong: u32,
    /// Thin divider.
    pub divider: u32,
    /// Primary text.
    pub text: u32,
    /// Secondary text.
    pub text_secondary: u32,
    /// Hint / caption text.
    pub text_muted: u32,
    /// Windows accent color (contrast-adjusted for dark/light mode).
    pub accent: u32,
    /// Selection highlight color (defaults to the Windows accent).
    pub selected: u32,
    /// Viewport frame color (defaults to a subtle accent).
    pub view: u32,
}

impl Palette {
    /// Build a Fluent Design palette from the active Windows theme; preserve any
    /// custom colors the user sets in `vscroll-wm.conf`.
    pub fn resolve(overrides: OverviewColors) -> Self {
        let dark = is_dark_mode();
        let raw_accent = system_accent();
        Self::build(dark, raw_accent, overrides)
    }

    /// Pure version (does not read the registry) for tests.
    pub fn build(dark: bool, raw_accent: Option<u32>, overrides: OverviewColors) -> Self {
        let def = OverviewColors::default();
        let sys_accent = match raw_accent {
            Some(c) if dark => {
                // WinUI 3 dark mode: lighten the accent if it is too dark to read.
                if luminance(c) < 125 {
                    mix(c, rgb(255, 255, 255), 32)
                } else {
                    mix(c, rgb(255, 255, 255), 14)
                }
            }
            Some(c) => {
                // WinUI 3 light mode: darken the accent if it is too bright for contrast.
                if luminance(c) > 165 {
                    mix(c, rgb(0, 0, 0), 28)
                } else {
                    mix(c, rgb(0, 0, 0), 10)
                }
            }
            None if dark => rgb(96, 205, 255), // #60CDFF (Fluent Dark Blue)
            None => rgb(0, 95, 184),           // #005FB8 (Fluent Light Blue)
        };

        let mut surface = if dark {
            rgb(32, 32, 32) // #202020 (SolidBackgroundFillColorBase / Acrylic Dark)
        } else {
            rgb(243, 243, 243) // #F3F3F3 (SolidBackgroundFillColorBase Light)
        };
        let mut card = if dark {
            rgb(46, 46, 46) // #2E2E2E (CardBackgroundFillColorDefault Dark)
        } else {
            rgb(252, 252, 252) // #FCFCFC (CardBackgroundFillColorDefault Light)
        };
        let mut accent = sys_accent;
        let mut selected = sys_accent;
        let mut view = if dark {
            mix(sys_accent, rgb(255, 255, 255), 25)
        } else {
            mix(sys_accent, rgb(0, 0, 0), 10)
        };

        // Respect explicit overrides from vscroll-wm.conf.
        if overrides.background != def.background {
            surface = overrides.background;
        }
        if overrides.box_fill != def.box_fill {
            card = overrides.box_fill;
        }
        if overrides.active != def.active {
            accent = overrides.active;
        }
        if overrides.selected != def.selected {
            selected = overrides.selected;
        }
        if overrides.view != def.view {
            view = overrides.view;
        }

        let eff_dark = luminance(surface) <= 145;
        let layer = if eff_dark {
            mix(surface, rgb(255, 255, 255), 5)
        } else {
            mix(surface, rgb(0, 0, 0), 4)
        };
        let card_hover = if eff_dark {
            mix(card, rgb(255, 255, 255), 8)
        } else {
            mix(card, rgb(0, 0, 0), 5)
        };
        let control = if eff_dark {
            mix(surface, rgb(255, 255, 255), 11)
        } else {
            mix(surface, rgb(0, 0, 0), 7)
        };
        let stroke = if eff_dark {
            mix(surface, rgb(255, 255, 255), 16)
        } else {
            mix(surface, rgb(0, 0, 0), 14)
        };
        let stroke_strong = if eff_dark {
            mix(surface, rgb(255, 255, 255), 26)
        } else {
            mix(surface, rgb(0, 0, 0), 22)
        };
        let divider = if eff_dark {
            mix(surface, rgb(255, 255, 255), 10)
        } else {
            mix(surface, rgb(0, 0, 0), 10)
        };
        let text = if eff_dark {
            rgb(255, 255, 255)
        } else {
            rgb(26, 26, 26)
        };
        let text_secondary = if eff_dark {
            rgb(207, 207, 207)
        } else {
            rgb(92, 92, 92)
        };
        let text_muted = if eff_dark {
            rgb(156, 156, 156)
        } else {
            rgb(122, 122, 122)
        };

        Self {
            dark: eff_dark,
            surface,
            layer,
            card,
            card_hover,
            control,
            stroke,
            stroke_strong,
            divider,
            text,
            text_secondary,
            text_muted,
            accent,
            selected,
            view,
        }
    }
}

/// Apply native Windows 11 DWM chrome: rounded corners (`DWMWCP_ROUND`), DWM dark
/// mode, and a subtle 1px border.
pub unsafe fn apply_window_chrome(hwnd: HWND, pal: &Palette, small_corners: bool) {
    let corner: u32 = if small_corners { 3 } else { 2 }; // DWMWCP_ROUNDSMALL / DWMWCP_ROUND
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        &corner as *const u32 as *const c_void,
        4,
    );
    let dark: u32 = if pal.dark { 1 } else { 0 };
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
        &dark as *const u32 as *const c_void,
        4,
    );
    let border: u32 = pal.stroke;
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_BORDER_COLOR,
        &border as *const u32 as *const c_void,
        4,
    );
}

// ---------------------------------------------------------------- Fluent animation

/// Fluent Decelerate easing curve (`cubic-bezier(0.1, 0.9, 0.2, 1.0`):
/// responsive and fast at first, then settles smoothly without a jolt (typical of WinUI 3).
pub fn ease_fluent_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 {
        return t;
    }
    // Solve x(u) = t for a cubic Bezier with P1=(0.1, 0.9), P2=(0.2, 1.0).
    const X1: f32 = 0.1;
    const Y1: f32 = 0.9;
    const X2: f32 = 0.2;
    const Y2: f32 = 1.0;
    let mut u = t;
    for _ in 0..5 {
        let inv = 1.0 - u;
        let x = 3.0 * inv * inv * u * X1 + 3.0 * inv * u * u * X2 + u * u * u;
        let dx = 3.0 * inv * inv * X1
            + 6.0 * inv * u * (X2 - X1)
            + 3.0 * u * u * (1.0 - X2);
        if dx.abs() < 1e-5 {
            break;
        }
        u = (u - (x - t) / dx).clamp(0.0, 1.0);
    }
    let inv = 1.0 - u;
    (3.0 * inv * inv * u * Y1 + 3.0 * inv * u * u * Y2 + u * u * u).clamp(0.0, 1.0)
}

/// Fluent Accelerate easing curve for exit animations (fade/slide out).
pub fn ease_fluent_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

// ---------------------------------------------------------------- GDI+ canvas

/// Double-buffered in-memory canvas + anti-aliased GDI+ context.
/// When `present()` is called (or on `Drop`), copy the result to the screen in one flicker-free operation.
pub struct Canvas {
    dst: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old_bmp: HGDIOBJ,
    w: i32,
    h: i32,
    presented: bool,
}

impl Canvas {
    pub unsafe fn new(dst: HDC, client: RECT) -> Self {
        let w = (client.right - client.left).max(1);
        let h = (client.bottom - client.top).max(1);
        let mem = CreateCompatibleDC(Some(dst));
        let bmp = CreateCompatibleBitmap(dst, w, h);
        let old_bmp = SelectObject(mem, HGDIOBJ(bmp.0));
        SetBkMode(mem, TRANSPARENT);
        Self {
            dst,
            mem,
            bmp,
            old_bmp,
            w,
            h,
            presented: false,
        }
    }

    pub fn hdc(&self) -> HDC {
        self.mem
    }

    pub unsafe fn set_clip(&self, r: RECT) {
        let _ = SelectClipRgn(self.mem, None);
        let _ = IntersectClipRect(self.mem, r.left, r.top, r.right, r.bottom);
    }

    pub unsafe fn reset_clip(&self) {
        let _ = SelectClipRgn(self.mem, None);
    }

    unsafe fn with_graphics<F: FnOnce(*mut GpGraphics)>(&self, f: F) {
        let mut g: *mut GpGraphics = std::ptr::null_mut();
        if GdipCreateFromHDC(self.mem, &mut g).0 == 0 && !g.is_null() {
            let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            let _ = GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
            f(g);
            let _ = GdipDeleteGraphics(g);
        }
    }

    unsafe fn make_round_path(x: f32, y: f32, w: f32, h: f32, radius: f32) -> *mut GpPath {
        let mut path: *mut GpPath = std::ptr::null_mut();
        if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
            return std::ptr::null_mut();
        }
        let r = radius.min(w * 0.5).min(h * 0.5).max(0.5);
        let d = r * 2.0;
        let _ = GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(path);
        path
    }

    pub unsafe fn fill_rect(&self, r: RECT, color: u32, alpha: u8) {
        let w = (r.right - r.left) as f32;
        let h = (r.bottom - r.top) as f32;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.with_graphics(|g| {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(argb(color, alpha), &mut brush).0 == 0 && !brush.is_null() {
                let _ = GdipFillRectangle(g, brush as *mut GpBrush, r.left as f32, r.top as f32, w, h);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
        });
    }

    pub unsafe fn fill_round_rect(&self, r: RECT, radius: f32, color: u32, alpha: u8) {
        let w = (r.right - r.left) as f32;
        let h = (r.bottom - r.top) as f32;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.with_graphics(|g| {
            let path = Self::make_round_path(r.left as f32, r.top as f32, w, h, radius);
            if path.is_null() {
                return;
            }
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(argb(color, alpha), &mut brush).0 == 0 && !brush.is_null() {
                let _ = GdipFillPath(g, brush as *mut GpBrush, path);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
            let _ = GdipDeletePath(path);
        });
    }

    pub unsafe fn stroke_round_rect(
        &self,
        r: RECT,
        radius: f32,
        color: u32,
        alpha: u8,
        width: f32,
    ) {
        let inset = width * 0.5;
        let w = (r.right - r.left) as f32 - width;
        let h = (r.bottom - r.top) as f32 - width;
        if w <= 0.5 || h <= 0.5 {
            return;
        }
        self.with_graphics(|g| {
            let path = Self::make_round_path(
                r.left as f32 + inset,
                r.top as f32 + inset,
                w,
                h,
                (radius - inset).max(0.5),
            );
            if path.is_null() {
                return;
            }
            let mut pen: *mut GpPen = std::ptr::null_mut();
            if GdipCreatePen1(argb(color, alpha), width, UnitPixel, &mut pen).0 == 0
                && !pen.is_null()
            {
                let _ = GdipDrawPath(g, pen, path);
                let _ = GdipDeletePen(pen);
            }
            let _ = GdipDeletePath(path);
        });
    }

    pub unsafe fn fill_ellipse(&self, r: RECT, color: u32, alpha: u8) {
        let w = (r.right - r.left) as f32;
        let h = (r.bottom - r.top) as f32;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.with_graphics(|g| {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(argb(color, alpha), &mut brush).0 == 0 && !brush.is_null() {
                let _ = GdipFillEllipse(g, brush as *mut GpBrush, r.left as f32, r.top as f32, w, h);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
        });
    }

    pub unsafe fn draw_text(
        &self,
        text: &str,
        r: RECT,
        font: HFONT,
        color: u32,
        flags: DRAW_TEXT_FORMAT,
    ) {
        if !font.0.is_null() {
            SelectObject(self.mem, HGDIOBJ(font.0));
        }
        SetTextColor(self.mem, COLORREF(color));
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rr = r;
        DrawTextW(self.mem, &mut wide, &mut rr, flags);
    }

    pub unsafe fn present(&mut self) {
        if !self.presented {
            let _ = BitBlt(self.dst, 0, 0, self.w, self.h, Some(self.mem), 0, 0, SRCCOPY);
            self.presented = true;
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            self.present();
            SelectObject(self.mem, self.old_bmp);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.mem);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_easing_is_monotonic_and_snappy() {
        assert_eq!(ease_fluent_out(0.0), 0.0);
        assert_eq!(ease_fluent_out(1.0), 1.0);
        // Fluent decelerate starts quickly: more than halfway through by t=0.25.
        let q = ease_fluent_out(0.25);
        let h = ease_fluent_out(0.5);
        assert!(q > 0.55 && q < 1.0, "q = {q}");
        assert!(h > q && h < 1.0, "h = {h}");
    }

    #[test]
    fn palette_respects_dark_light_mode_and_config_overrides() {
        let dark = Palette::build(true, Some(rgb(0, 120, 212)), OverviewColors::default());
        let light = Palette::build(false, Some(rgb(0, 120, 212)), OverviewColors::default());
        assert!(dark.dark);
        assert!(!light.dark);
        assert!(luminance(dark.surface) < 60);
        assert!(luminance(light.surface) > 220);

        let mut custom = OverviewColors::default();
        custom.background = rgb(10, 20, 30);
        custom.selected = rgb(255, 0, 0);
        let p = Palette::build(false, Some(rgb(0, 120, 212)), custom);
        assert_eq!(p.surface, rgb(10, 20, 30));
        assert_eq!(p.selected, rgb(255, 0, 0));
    }
}
