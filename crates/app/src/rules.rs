//! Rules for windows that must NOT be tiled (always floating).
//!
//! In addition to the built-in list, users can add rules in the
//! `vscroll-wm.ignore` text file (in the app's working directory, or at the path
//! specified by the VSCROLL_IGNORE environment variable). One rule per line:
//!
//!   class:ClassName     exact match, case-insensitive
//!   exe:name.exe        executable file name
//!   title:substring     window title contains this text
//!
//! Blank lines and lines beginning with # are ignored.

use std::sync::OnceLock;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowTextW, GetWindowThreadProcessId,
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
    "Ghost",  // Placeholder for an unresponsive app.
    "#32770", // Standard dialog (installer, "Run", etc.).
];

#[derive(Debug, Default, PartialEq)]
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
/// (When launched automatically at login, the working directory is not the project directory.)
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

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
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
    })
}

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
    let rules = rules();
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
}
