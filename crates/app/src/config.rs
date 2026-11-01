//! Konfigurasi: file teks `vscroll-wm.conf` berisi baris `kunci = nilai`.
//! Lokasi: env VSCROLL_CONFIG, atau di samping exe, atau di folder kerja.
//! Env VSCROLL_GAP / VSCROLL_ANIM_MS (kalau ada) menimpa isi file.
//!
//! Kunci yang dikenal:
//!   gap      = 16     jarak antar window dan ke tepi layar (piksel, 0-200)
//!   anim_ms  = 160    durasi animasi (ms, 0 = mati). Kosong = bawaan; otomatis
//!                     mati kalau "Animation effects" Windows dimatikan.
//!   focus_border = #4FA8F5   warna border window yang fokus (Windows 11).
//!                     Kosong / off = tidak diubah.
//!
//! Komentar hanya berlaku untuk baris yang diawali #.

use std::path::PathBuf;

pub const DEFAULT_GAP: i32 = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub gap: i32,
    /// None = bawaan (160 ms, atau mati bila animasi sistem dimatikan).
    pub anim_ms: Option<f32>,
    /// Warna border fokus sebagai COLORREF (0x00BBGGRR). None = tidak diubah.
    pub focus_border: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self { gap: DEFAULT_GAP, anim_ms: None, focus_border: None }
    }
}

impl Config {
    /// Baca isi file. Mengembalikan konfigurasi dan peringatan untuk baris bermasalah.
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut cfg = Config::default();
        let mut warnings = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let n = n + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("baris {n}: bukan 'kunci = nilai'"));
                continue;
            };
            let key = key.trim().to_lowercase();
            let value = value.trim();
            match key.as_str() {
                "gap" => match value.parse::<i32>() {
                    Ok(v) => cfg.gap = v.clamp(0, 200),
                    Err(_) => warnings.push(format!("baris {n}: gap harus bilangan bulat")),
                },
                "anim_ms" => match value.parse::<f32>() {
                    Ok(v) if v.is_finite() => cfg.anim_ms = Some(v.clamp(0.0, 1000.0)),
                    _ => warnings.push(format!("baris {n}: anim_ms harus angka")),
                },
                "focus_border" => match parse_color(value) {
                    Some(color) => cfg.focus_border = color,
                    None => warnings.push(format!("baris {n}: focus_border harus #RRGGBB atau off")),
                },
                _ => warnings.push(format!("baris {n}: kunci tidak dikenal '{key}'")),
            }
        }
        (cfg, warnings)
    }
}

/// "#RRGGBB" / "RRGGBB" -> Some(Some(COLORREF)); "off"/"none"/kosong -> Some(None);
/// selain itu None (tidak valid).
fn parse_color(v: &str) -> Option<Option<u32>> {
    let v = v.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let hex = v.strip_prefix('#').unwrap_or(v);
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    let (r, g, b) = ((n >> 16) & 0xFF, (n >> 8) & 0xFF, n & 0xFF);
    Some(Some(r | (g << 8) | (b << 16)))
}

fn find_file() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("VSCROLL_CONFIG") {
        return Some(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("vscroll-wm.conf");
            if p.exists() {
                return Some(p);
            }
        }
    }
    let p = PathBuf::from("vscroll-wm.conf");
    p.exists().then_some(p)
}

/// Muat konfigurasi: bawaan, lalu file, lalu environment variable.
pub fn load() -> Config {
    let mut cfg = Config::default();
    if let Some(path) = find_file() {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let (c, warnings) = Config::parse(&text);
                cfg = c;
                println!("konfigurasi dimuat dari {}", path.display());
                for w in warnings {
                    eprintln!("konfigurasi: {w}");
                }
            }
            Err(e) => eprintln!("gagal membaca {}: {e}", path.display()),
        }
    }
    if let Some(v) = std::env::var("VSCROLL_GAP").ok().and_then(|v| v.parse::<i32>().ok()) {
        cfg.gap = v.clamp(0, 200);
    }
    if let Some(v) = std::env::var("VSCROLL_ANIM_MS").ok().and_then(|v| v.parse::<f32>().ok()) {
        if v.is_finite() {
            cfg.anim_ms = Some(v.clamp(0.0, 1000.0));
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_gives_defaults() {
        let (cfg, warnings) = Config::parse("");
        assert_eq!(cfg, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn parses_values_comments_and_case() {
        let (cfg, w) = Config::parse("# komentar\nGap = 24\nanim_ms=200\nfocus_border = #4FA8F5\n");
        assert_eq!(cfg.gap, 24);
        assert_eq!(cfg.anim_ms, Some(200.0));
        assert_eq!(cfg.focus_border, Some(0x00F5_A84F)); // BGR untuk RGB #4FA8F5
        assert!(w.is_empty());
    }

    #[test]
    fn focus_border_accepts_plain_hex_and_off() {
        let (a, _) = Config::parse("focus_border = FF0000");
        assert_eq!(a.focus_border, Some(0x0000_00FF));
        let (b, _) = Config::parse("focus_border = #FF0000\nfocus_border = off");
        assert_eq!(b.focus_border, None);
    }

    #[test]
    fn focus_border_rejects_bad_colors() {
        let (cfg, w) = Config::parse("focus_border = merah\nfocus_border = #12345");
        assert_eq!(cfg.focus_border, None);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn clamps_out_of_range_values() {
        let (cfg, _) = Config::parse("gap = 9999\nanim_ms = -5\n");
        assert_eq!(cfg.gap, 200);
        assert_eq!(cfg.anim_ms, Some(0.0));
    }

    #[test]
    fn bad_lines_warn_and_keep_defaults() {
        let (cfg, w) = Config::parse("gap = banyak\nanim_ms = nan\nwarna = biru\nbukan kunci\n");
        assert_eq!(cfg, Config::default());
        assert_eq!(w.len(), 4);
    }
}
