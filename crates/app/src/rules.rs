//! Aturan window: pengecualian (`vscroll-wm.ignore`) dan aturan kaya (`rule = ...` di
//! `vscroll-wm.conf`). Keduanya dimuat ulang tanpa restart lewat "Reload configuration".
//!
//! Format `vscroll-wm.ignore` (satu aturan per baris, selalu float):
//!   class:ClassName     cocok persis, tidak peka huruf besar/kecil
//!   exe:name.exe        nama berkas eksekusi
//!   title:substring     judul window memuat teks ini
//!
//! Format `rule = ...` di `vscroll-wm.conf` (dipisah koma, semua pencocok harus cocok):
//!   Pencocok: class:<nama>, exe:<nama.exe>, title:<teks>, max_width=<px>, max_height=<px>
//!   Aksi:     float [width=<px>] [height=<px>]
//!             atau workspace=<1-9>, span=left|right|full, height=1/3|1/2|full|40%

use std::sync::RwLock;

use layout::{Height, Span};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
};

/// Shell / system window classes that are not apps to be tiled.
const BUILTIN_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "XamlExplorerHostIslandWindow",
    "MultitaskingViewFrame",
    "ForegroundStaging",
    "Windows.UI.Core.CoreWindow",
    "Windows.UI.Composition.DesktopWindowContentBridge",
    "Ghost",                 // Placeholder for an unresponsive app.
    "#32770",                // Standard dialog (installer, "Run", etc.).
    "OperationStatusWindow", // File Explorer copy / move / delete / replace dialog.
];

// ------------------------------------------------------------ vscroll-wm.ignore

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Rules {
    classes: Vec<String>,
    exes: Vec<String>,
    titles: Vec<String>,
}

impl Rules {
    pub fn parse(text: &str) -> Rules {
        let mut r = Rules::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((kind, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().to_lowercase();
            if value.is_empty() {
                continue;
            }
            match kind.trim().to_lowercase().as_str() {
                "class" => r.classes.push(value),
                "exe" => r.exes.push(value),
                "title" => r.titles.push(value),
                _ => {}
            }
        }
        r
    }
}

/// VSCROLL_IGNORE environment variable, or a file next to the executable, or in the working directory.
fn ignore_path() -> String {
    if let Ok(p) = std::env::var("VSCROLL_IGNORE") {
        return p;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("vscroll-wm.ignore");
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    "vscroll-wm.ignore".into()
}

static IGNORE_RULES: RwLock<Option<Rules>> = RwLock::new(None);

fn load_ignore_from_disk() -> Rules {
    let path = ignore_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let r = Rules::parse(&text);
            println!(
                "aturan abaikan dimuat dari {path}: {} class, {} exe, {} title",
                r.classes.len(),
                r.exes.len(),
                r.titles.len()
            );
            r
        }
        Err(_) => Rules::default(),
    }
}

/// Muat ulang `vscroll-wm.ignore` dari disk (dipanggil saat "Reload configuration").
pub fn reload() {
    let loaded = load_ignore_from_disk();
    if let Ok(mut g) = IGNORE_RULES.write() {
        *g = Some(loaded);
    }
}

fn current_ignore() -> Rules {
    if let Ok(g) = IGNORE_RULES.read() {
        if let Some(ref r) = *g {
            return r.clone();
        }
    }
    let loaded = load_ignore_from_disk();
    if let Ok(mut g) = IGNORE_RULES.write() {
        *g = Some(loaded.clone());
    }
    loaded
}

// -------------------------------------------------------- rich window rules (B2)

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    Float {
        width: Option<i32>,
        height: Option<i32>,
    },
    Tile {
        /// Indeks workspace 0..=8 (ditulis 1..=9 di config).
        workspace: Option<usize>,
        span: Option<Span>,
        height: Option<Height>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRule {
    pub class: Option<String>,
    pub exe: Option<String>,
    pub title: Option<String>,
    pub max_width: Option<i32>,
    pub max_height: Option<i32>,
    pub action: RuleAction,
}

fn parse_tile_height(s: &str) -> Result<Height, String> {
    let lower = s.trim().to_lowercase();
    let frac: f64 = if lower == "full" {
        1.0
    } else if let Some(p) = lower.strip_suffix('%') {
        p.trim()
            .parse::<f64>()
            .map_err(|_| format!("bad height '{s}'"))?
            / 100.0
    } else if let Some((a, b)) = lower.split_once('/') {
        let a = a.trim().parse::<f64>().map_err(|_| format!("bad height '{s}'"))?;
        let b = b.trim().parse::<f64>().map_err(|_| format!("bad height '{s}'"))?;
        if b <= 0.0 {
            return Err(format!("bad height '{s}'"));
        }
        a / b
    } else {
        return Err(format!(
            "bad height '{s}' (use 1/3, 1/2, full or 40%; for float use pixel height)"
        ));
    };
    if !frac.is_finite() {
        return Err(format!("bad height '{s}'"));
    }
    let permil = (frac * 1000.0).round() as i32;
    if !(150..=1000).contains(&permil) {
        return Err(format!("height '{s}' must be between 15% and 100%"));
    }
    Ok(match permil {
        1000 => Height::Full,
        500 => Height::Half,
        333 => Height::Third,
        p => Height::Custom(p),
    })
}

fn parse_px(name: &str, v: &str) -> Result<i32, String> {
    let px: i32 = v
        .trim()
        .parse()
        .map_err(|_| format!("{name} must be a whole number of pixels"))?;
    if !(100..=10000).contains(&px) {
        return Err(format!("{name} must be between 100 and 10000"));
    }
    Ok(px)
}

impl WindowRule {
    /// Baca satu baris `rule = ...`.
    /// Contoh:
    ///   `exe:vlc.exe, float, width=900, height=540`
    ///   `exe:code.exe, workspace=1, span=left, height=full`
    ///   `class:CabinetWClass, max_width=500, max_height=400, float`
    pub fn parse(spec: &str) -> Result<WindowRule, String> {
        let mut class = None;
        let mut exe = None;
        let mut title = None;
        let mut max_width = None;
        let mut max_height = None;

        let mut is_float = false;
        let mut float_w = None;
        let mut raw_height: Option<String> = None;
        let mut workspace = None;
        let mut span = None;

        for tok in spec.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let lower = tok.to_lowercase();
            if lower == "float" {
                is_float = true;
                continue;
            }
            // dukung pemisah `:` (gaya ignore) maupun `=` (gaya config)
            let sep = tok
                .find([':', '='])
                .ok_or_else(|| format!("unknown rule token '{tok}'"))?;
            let k = tok[..sep].trim().to_lowercase();
            let v = tok[sep + 1..].trim();
            if v.is_empty() {
                return Err(format!("missing value for '{k}'"));
            }
            match k.as_str() {
                "class" => class = Some(v.to_lowercase()),
                "exe" => exe = Some(v.to_lowercase()),
                "title" => title = Some(v.to_lowercase()),
                "max_width" => max_width = Some(parse_px("max_width", v)?),
                "max_height" => max_height = Some(parse_px("max_height", v)?),
                "width" => float_w = Some(parse_px("width", v)?),
                "height" => raw_height = Some(v.to_string()),
                "workspace" => {
                    let n: usize = v
                        .parse()
                        .map_err(|_| format!("workspace must be 1..9, got '{v}'"))?;
                    if !(1..=9).contains(&n) {
                        return Err(format!("workspace must be 1..9, got '{v}'"));
                    }
                    workspace = Some(n - 1);
                }
                "span" => {
                    span = Some(match v.to_lowercase().as_str() {
                        "left" | "l" => Span::Left,
                        "right" | "r" => Span::Right,
                        "full" | "f" => Span::Full,
                        _ => return Err(format!("span must be left, right or full, got '{v}'")),
                    });
                }
                other => return Err(format!("unknown rule key '{other}'")),
            }
        }

        if class.is_none()
            && exe.is_none()
            && title.is_none()
            && max_width.is_none()
            && max_height.is_none()
        {
            return Err("rule needs at least one matcher (class, exe, title, max_width, max_height)".into());
        }

        let action = if is_float {
            if workspace.is_some() || span.is_some() {
                return Err("float cannot be combined with workspace or span".into());
            }
            let height = match raw_height {
                Some(ref h) => Some(parse_px("height", h)?),
                None => None,
            };
            RuleAction::Float {
                width: float_w,
                height,
            }
        } else {
            if float_w.is_some() {
                return Err("width is only for float rules".into());
            }
            let height = match raw_height {
                Some(ref h) => Some(parse_tile_height(h)?),
                None => None,
            };
            if workspace.is_none() && span.is_none() && height.is_none() {
                return Err("rule needs an action (float, workspace, span, or height)".into());
            }
            RuleAction::Tile {
                workspace,
                span,
                height,
            }
        };

        Ok(WindowRule {
            class,
            exe,
            title,
            max_width,
            max_height,
            action,
        })
    }

    /// True kalau semua pencocok yang diatur pada aturan ini terpenuhi.
    pub fn matches(&self, class: &str, exe: Option<&str>, title: &str, size: (i32, i32)) -> bool {
        if let Some(ref c) = self.class {
            if !class.eq_ignore_ascii_case(c) {
                return false;
            }
        }
        if let Some(ref e) = self.exe {
            if !exe.is_some_and(|x| x.eq_ignore_ascii_case(e)) {
                return false;
            }
        }
        if let Some(ref t) = self.title {
            if !title.to_lowercase().contains(t.as_str()) {
                return false;
            }
        }
        if let Some(mw) = self.max_width {
            if size.0 > mw {
                return false;
            }
        }
        if let Some(mh) = self.max_height {
            if size.1 > mh {
                return false;
            }
        }
        true
    }
}

/// Cari aturan pertama di `rules` yang cocok dengan window `h`.
pub unsafe fn match_rule<'a>(rules: &'a [WindowRule], h: HWND) -> Option<&'a WindowRule> {
    if rules.is_empty() {
        return None;
    }
    let need_class = rules.iter().any(|r| r.class.is_some());
    let need_exe = rules.iter().any(|r| r.exe.is_some());
    let need_title = rules.iter().any(|r| r.title.is_some());
    let need_size = rules
        .iter()
        .any(|r| r.max_width.is_some() || r.max_height.is_some());

    let class = if need_class { class_name(h) } else { String::new() };
    let exe = if need_exe { exe_name(h) } else { None };
    let title = if need_title {
        window_title(h)
    } else {
        String::new()
    };
    let size = if need_size {
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        (r.right - r.left, r.bottom - r.top)
    } else {
        (0, 0)
    };

    rules
        .iter()
        .find(|r| r.matches(&class, exe.as_deref(), &title, size))
}

// ---------------------------------------------------------------- Win32 info

pub unsafe fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetClassNameW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub unsafe fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(h, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub unsafe fn exe_name(h: HWND) -> Option<String> {
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = [0u16; 520];
    let mut len = buf.len() as u32;
    let ok =
        QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
    let _ = CloseHandle(proc);
    ok.ok()?;
    let full = String::from_utf16_lossy(&buf[..len as usize]);
    full.rsplit('\\').next().map(|s| s.to_lowercase())
}

/// Why this window must remain floating, or None if it can be tiled.
pub unsafe fn ignored(h: HWND) -> Option<&'static str> {
    let rules = current_ignore();
    let class = class_name(h);
    if BUILTIN_CLASSES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&class))
    {
        return Some("kelas bawaan (shell / sistem / dialog standar)");
    }
    let class = class.to_lowercase();
    if rules.classes.iter().any(|c| *c == class) {
        return Some("cocok aturan class di file ignore");
    }
    if !rules.titles.is_empty() {
        let title = window_title(h).to_lowercase();
        if rules.titles.iter().any(|t| title.contains(t.as_str())) {
            return Some("cocok aturan title di file ignore");
        }
    }
    if !rules.exes.is_empty() {
        if let Some(exe) = exe_name(h) {
            if rules.exes.contains(&exe) {
                return Some("cocok aturan exe di file ignore");
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_ignoring_comments_blanks_and_unknown_kinds() {
        let r = Rules::parse(
            "# komentar\n\nclass: MyClass \nEXE:Calc.exe\ntitle:Picture in Picture\nfoo:bar\nnocolon\nclass:\n",
        );
        assert_eq!(r.classes, vec!["myclass"]);
        assert_eq!(r.exes, vec!["calc.exe"]);
        assert_eq!(r.titles, vec!["picture in picture"]);
    }

    #[test]
    fn empty_text_gives_empty_rules() {
        assert_eq!(Rules::parse(""), Rules::default());
    }

    #[test]
    fn parses_float_and_tile_window_rules() {
        let r = WindowRule::parse("exe:vlc.exe, float, width=900, height=540").unwrap();
        assert_eq!(r.exe.as_deref(), Some("vlc.exe"));
        assert_eq!(
            r.action,
            RuleAction::Float {
                width: Some(900),
                height: Some(540),
            }
        );

        let r = WindowRule::parse("exe:code.exe, workspace=2, span=left, height=1/2").unwrap();
        assert_eq!(
            r.action,
            RuleAction::Tile {
                workspace: Some(1),
                span: Some(Span::Left),
                height: Some(Height::Half),
            }
        );

        let r = WindowRule::parse("class:CabinetWClass, max_width=500, max_height=400, float")
            .unwrap();
        assert_eq!(r.max_width, Some(500));
        assert_eq!(r.max_height, Some(400));
        assert!(r.matches("CabinetWClass", Some("explorer.exe"), "Copy", (450, 300)));
        assert!(!r.matches("CabinetWClass", Some("explorer.exe"), "Files", (900, 600)));
    }

    #[test]
    fn rejects_invalid_window_rules() {
        assert!(WindowRule::parse("float").is_err()); // tanpa pencocok
        assert!(WindowRule::parse("exe:foo.exe").is_err()); // tanpa aksi
        assert!(WindowRule::parse("exe:foo.exe, float, workspace=2").is_err());
        assert!(WindowRule::parse("exe:foo.exe, width=500, span=left").is_err());
        assert!(WindowRule::parse("exe:foo.exe, workspace=0").is_err());
        assert!(WindowRule::parse("exe:foo.exe, workspace=10").is_err());
        assert!(WindowRule::parse("exe:foo.exe, span=center").is_err());
    }
}
