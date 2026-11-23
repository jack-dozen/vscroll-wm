//! Configuration: the `vscroll-wm.conf` text file contains `key = value` lines.
//! Location: VSCROLL_CONFIG environment variable, next to the executable, or in the working directory.
//! VSCROLL_GAP / VSCROLL_ANIM_MS environment variables (if set) override the file values.
//!
//! Recognized keys:
//!   gap      = 16     gap between windows and from the screen edge (pixels, 0-200)
//!   anim_ms  = 160    animation duration (ms, 0 = off). Empty = default; automatically
//!                     off when Windows "Animation effects" is disabled.
//!   focus_border = #4FA8F5   border color for the focused window (Windows 11).
//!                     Empty / off = unchanged.
//!   overview_hint = on        key hints at the bottom of the overview
//!   overview_minimap = right  overview strip minimap position: right, left, or off
//!   overview_thumbnails = on  thumbnail of each window in the overview
//!   overview_label = title    box label: title (full title), app (app name only),
//!                     reverse ("title - Vivaldi" becomes "Vivaldi - title")
//!   alias.code = VS Code      display name for `app` mode (executable name without .exe)
//!   cheatsheet_key = slash    cheatsheet key with Win: slash (/), f1-f12, tab,
//!                     space, grave, a letter/number, or off. Differs from overview_key.
//!   resize_step = 80          Win+[ and Win+] step size (pixels, 10-400)
//!   height_mode = preset      Win+↑/↓: preset (1/3, 1/2, full) or step (incremental)
//!   height_step = 80          Win+↑/↓ step size in step mode (pixels, 10-400)
//!   overview_size = 70        overview panel size, percent of work area (30-100)
//!   overview_key = tab        overview key with Win: tab, space, grave, a
//!                     letter/number (not q/m), or off. Win+Alt+Space is always active.
//!   overview_color_selected / _active / _view / _background / _box = #RRGGBB
//!   mouse_drag = on           drag a window onto another to swap positions; drag a window edge
//!                     next to the column divider to move it; in overview, drag a window
//!                     to another workspace column to switch workspaces
//!   remember_layout = on      save workspace layout; restore it when vscroll-wm is restarted
//!                     while its windows are still open
//!   workspace_osd = on        brief panel at the bottom of the screen on each workspace switch
//!   workspace_osd_dots = off  show workspace-count dots/pill below the OSD text
//!   overview_wrap = on        overview arrows wrap from one end to the other
//!   snap_layouts = on         show preset layouts when a window is dragged to the top edge;
//!                     release over a zone to place the window there
//!   snap_layouts_defaults = on  include built-in presets (halves, quarters, ...)
//!   snap_trigger = 24         trigger band width (pixels) at the top edge of the screen
//!   snap_assist = on          after a window enters a zone, other zones show a list of
//!                     open windows to choose from
//!   snap_layout.<name> = split=67, L, R:1/2, R:1/2   custom preset (or `off` to
//!                     remove the built-in preset with that name); see vscroll-wm.conf.example
//!   bind.<action> = win+q     change a shortcut (action list: bindings.rs / vscroll-wm.conf.example).
//!                     Separate multiple keys with commas; off = disabled. overview_key and
//!                     cheatsheet_key remain the defaults for bind.overview /
//!                     bind.cheatsheet.
//!
//! Comments apply only to lines beginning with #.

use std::path::PathBuf;

use crate::bindings::{self, Bindings, Chord, WIN};
use crate::rules::WindowRule;

pub const DEFAULT_GAP: i32 = 16;

/// COLORREF (0x00BBGGRR) from RGB components.
pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

/// Overview colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewColors {
    /// Overview cursor: the window to focus.
    pub selected: u32,
    /// The currently active window.
    pub active: u32,
    /// Frame around the strip section (row) currently visible on screen.
    pub view: u32,
    pub background: u32,
    pub box_fill: u32,
}

impl Default for OverviewColors {
    fn default() -> Self {
        Self {
            selected: rgb(255, 190, 60),
            active: rgb(79, 168, 245),
            view: rgb(120, 200, 130),
            background: rgb(20, 22, 28),
            box_fill: rgb(45, 50, 64),
        }
    }
}

/// Position of the overview strip minimap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimapSide {
    Right,
    Left,
    Off,
}

/// Content of an overview box label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelMode {
    /// Window title as-is.
    Title,
    /// App name only (from the executable name).
    App,
    /// Move the part after " - " to the front: "title - Vivaldi" becomes "Vivaldi - title".
    Reverse,
}

/// How Win+↑/↓ changes the window height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightMode {
    /// Jump among the 1/3, 1/2, and full presets.
    Preset,
    /// Move by `height_step` pixels per press.
    Step,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub gap: i32,
    /// None = default (160 ms, or off when system animations are disabled).
    pub anim_ms: Option<f32>,
    /// Focus border color as COLORREF (0x00BBGGRR). None = unchanged.
    pub focus_border: Option<u32>,
    /// Target border color while aiming in `join` mode (COLORREF). None = use `focus_border`.
    pub join_border: Option<u32>,
    pub overview_hint: bool,
    pub overview_minimap: MinimapSide,
    pub overview_thumbnails: bool,
    pub overview_label: LabelMode,
    /// Display name for `app` mode: (lowercase executable name without .exe) -> name.
    pub aliases: Vec<(String, String)>,
    /// Column divider movement per Win+[ / Win+] press (pixels).
    pub resize_step: i32,
    pub height_mode: HeightMode,
    pub height_step: i32,
    /// Virtual-key code for the cheatsheet key (with Win). 0 = disabled.
    pub cheatsheet_key: u16,
    /// Overview panel size as a percentage of the work area.
    pub overview_size: i32,
    /// Virtual-key code for the overview key (with Win). 0 = disabled.
    pub overview_key: u16,
    pub overview_colors: OverviewColors,
    /// Overview arrows wrap (jump from one end to the other).
    pub overview_wrap: bool,
    /// Brief panel shown on each workspace switch.
    pub workspace_osd: bool,
    /// Show workspace indicator dots/pill in the OSD (default: off).
    pub workspace_osd_dots: bool,
    /// Save / restore the workspace layout (layout.state).
    pub remember_layout: bool,
    /// Mouse drag: swap positions, move the divider (on the desktop), and switch workspaces (in overview).
    pub mouse_drag: bool,
    /// Snap layouts menu when a window is dragged to the top edge of the screen.
    pub snap_layouts_enabled: bool,
    /// Height of the menu trigger band at the top of the work area (pixels).
    pub snap_trigger: i32,
    /// Window list for filling zones left empty after snapping.
    pub snap_assist: bool,
    /// Presets offered by the menu, in display order (built-in, then custom).
    pub snap_layouts: Vec<layout::SnapPreset>,
    /// Keys for each action (defaults + `bind.*` + overview_key / cheatsheet_key).
    pub bindings: Bindings,
    /// Window rules (`rule = ...`), in file order.
    pub rules: Vec<WindowRule>,
    /// Auto-minimize non-sticky floating windows when focus moves to another window.
    pub float_auto_minimize: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gap: DEFAULT_GAP,
            anim_ms: None,
            focus_border: None,
            join_border: None,
            overview_hint: true,
            overview_minimap: MinimapSide::Right,
            overview_thumbnails: true,
            overview_label: LabelMode::Title,
            aliases: Vec::new(),
            resize_step: 80,
            height_mode: HeightMode::Preset,
            height_step: 80,
            cheatsheet_key: 0xBF, // /
            overview_size: 70,
            overview_key: 0x09, // Tab
            overview_colors: OverviewColors::default(),
            overview_wrap: true,
            workspace_osd: true,
            workspace_osd_dots: false,
            remember_layout: true,
            mouse_drag: true,
            snap_layouts_enabled: true,
            snap_trigger: 24,
            snap_assist: true,
            snap_layouts: layout::builtin_presets(),
            bindings: Bindings::defaults(),
            rules: Vec::new(),
            float_auto_minimize: true,
        }
    }
}

impl Config {
    /// Read the file contents. Returns the config and warnings for problematic lines.
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut cfg = Config::default();
        let mut warnings = Vec::new();
        let mut binds = Bindings::defaults();
        // Actions configured via bind.* (file order = conflict priority).
        let mut explicit: Vec<usize> = Vec::new();
        // Custom snap layout presets (None = remove the built-in preset with that name).
        let mut snap_defaults = true;
        let mut snap_user: Vec<(String, Option<layout::SnapPreset>)> = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let n = n + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("line {n}: not in 'key = value' form"));
                continue;
            };
            let key = key.trim().to_lowercase();
            let value = value.trim();
            match key.as_str() {
                "gap" => match value.parse::<i32>() {
                    Ok(v) => cfg.gap = v.clamp(0, 200),
                    Err(_) => warnings.push(format!("line {n}: gap must be a whole number")),
                },
                "anim_ms" => match value.parse::<f32>() {
                    Ok(v) if v.is_finite() => cfg.anim_ms = Some(v.clamp(0.0, 1000.0)),
                    _ => warnings.push(format!("line {n}: anim_ms must be a number")),
                },
                "focus_border" => match parse_color(value) {
                    Some(color) => cfg.focus_border = color,
                    None => warnings.push(format!("line {n}: focus_border must be #RRGGBB or off")),
                },
                "join_border" => match parse_color(value) {
                    Some(color) => cfg.join_border = color,
                    None => warnings.push(format!("line {n}: join_border must be #RRGGBB or off")),
                },
                "rule" => match WindowRule::parse(value) {
                    Ok(r) => cfg.rules.push(r),
                    Err(e) => warnings.push(format!("line {n}: rule: {e}")),
                },
                "float_auto_minimize" => match parse_bool(value) {
                    Some(b) => cfg.float_auto_minimize = b,
                    None => warnings.push(format!(
                        "line {n}: float_auto_minimize must be on/off"
                    )),
                },
                "overview_hint" => match parse_bool(value) {
                    Some(b) => cfg.overview_hint = b,
                    None => warnings.push(format!("line {n}: overview_hint must be on/off")),
                },
                "overview_minimap" => match value.to_lowercase().as_str() {
                    "right" => cfg.overview_minimap = MinimapSide::Right,
                    "left" => cfg.overview_minimap = MinimapSide::Left,
                    "off" | "none" => cfg.overview_minimap = MinimapSide::Off,
                    _ => warnings.push(format!(
                        "line {n}: overview_minimap must be right, left or off"
                    )),
                },
                "overview_thumbnails" => match parse_bool(value) {
                    Some(b) => cfg.overview_thumbnails = b,
                    None => warnings.push(format!("line {n}: overview_thumbnails must be on/off")),
                },
                "overview_label" => match value.to_lowercase().as_str() {
                    "title" => cfg.overview_label = LabelMode::Title,
                    "app" => cfg.overview_label = LabelMode::App,
                    "reverse" => cfg.overview_label = LabelMode::Reverse,
                    _ => warnings.push(format!(
                        "line {n}: overview_label must be title, app or reverse"
                    )),
                },
                "resize_step" => match value.parse::<i32>() {
                    Ok(v) => cfg.resize_step = v.clamp(10, 400),
                    Err(_) => warnings.push(format!("line {n}: resize_step must be a whole number")),
                },
                "height_mode" => match value.to_lowercase().as_str() {
                    "preset" => cfg.height_mode = HeightMode::Preset,
                    "step" => cfg.height_mode = HeightMode::Step,
                    _ => warnings.push(format!("line {n}: height_mode must be preset or step")),
                },
                "height_step" => match value.parse::<i32>() {
                    Ok(v) => cfg.height_step = v.clamp(10, 400),
                    Err(_) => warnings.push(format!("line {n}: height_step must be a whole number")),
                },
                "overview_size" => match value.parse::<i32>() {
                    Ok(v) => cfg.overview_size = v.clamp(30, 100),
                    Err(_) => warnings.push(format!("line {n}: overview_size must be a whole number")),
                },
                "cheatsheet_key" => match parse_key(value) {
                    Some(k) => cfg.cheatsheet_key = k,
                    None => warnings.push(format!(
                        "line {n}: cheatsheet_key must be slash, f1-f12, tab, space, grave, off, or a single letter/digit (not q/m)"
                    )),
                },
                "overview_key" => match parse_key(value) {
                    Some(k) => cfg.overview_key = k,
                    None => warnings.push(format!(
                        "line {n}: overview_key must be tab, space, grave, off, or a single letter/digit (not q/m)"
                    )),
                },
                "overview_color_selected" => {
                    color_setting(value, &mut cfg.overview_colors.selected, &key, n, &mut warnings)
                }
                "overview_color_active" => {
                    color_setting(value, &mut cfg.overview_colors.active, &key, n, &mut warnings)
                }
                "overview_color_view" => {
                    color_setting(value, &mut cfg.overview_colors.view, &key, n, &mut warnings)
                }
                "overview_color_background" => {
                    color_setting(value, &mut cfg.overview_colors.background, &key, n, &mut warnings)
                }
                "overview_color_box" => {
                    color_setting(value, &mut cfg.overview_colors.box_fill, &key, n, &mut warnings)
                }
                "mouse_drag" => match parse_bool(value) {
                    Some(b) => cfg.mouse_drag = b,
                    None => warnings.push(format!("line {n}: mouse_drag must be on/off")),
                },
                "remember_layout" => match parse_bool(value) {
                    Some(b) => cfg.remember_layout = b,
                    None => warnings.push(format!("line {n}: remember_layout must be on/off")),
                },
                "workspace_osd" => match parse_bool(value) {
                    Some(b) => cfg.workspace_osd = b,
                    None => warnings.push(format!("line {n}: workspace_osd must be on/off")),
                },
                "workspace_osd_dots" => match parse_bool(value) {
                    Some(b) => cfg.workspace_osd_dots = b,
                    None => warnings.push(format!("line {n}: workspace_osd_dots must be on/off")),
                },
                "overview_wrap" => match parse_bool(value) {
                    Some(b) => cfg.overview_wrap = b,
                    None => warnings.push(format!("line {n}: overview_wrap must be on/off")),
                },
                "snap_layouts" => match parse_bool(value) {
                    Some(b) => cfg.snap_layouts_enabled = b,
                    None => warnings.push(format!("line {n}: snap_layouts must be on/off")),
                },
                "snap_layouts_defaults" => match parse_bool(value) {
                    Some(b) => snap_defaults = b,
                    None => warnings.push(format!("line {n}: snap_layouts_defaults must be on/off")),
                },
                "snap_assist" => match parse_bool(value) {
                    Some(b) => cfg.snap_assist = b,
                    None => warnings.push(format!("line {n}: snap_assist must be on/off")),
                },
                "snap_trigger" => match value.parse::<i32>() {
                    Ok(v) => cfg.snap_trigger = v.clamp(2, 300),
                    Err(_) => warnings.push(format!("line {n}: snap_trigger must be a whole number")),
                },
                k if k.starts_with("snap_layout.") => {
                    let name = k["snap_layout.".len()..].trim().to_string();
                    if name.is_empty() {
                        warnings.push(format!("line {n}: snap_layout needs a name (snap_layout.<name>)"));
                    } else if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("none") {
                        snap_user.retain(|(other, _)| *other != name);
                        snap_user.push((name, None));
                    } else {
                        match layout::SnapPreset::parse(&name, value) {
                            Ok(p) => {
                                snap_user.retain(|(other, _)| *other != name);
                                snap_user.push((name, Some(p)));
                            }
                            Err(e) => warnings.push(format!("line {n}: snap_layout.{name}: {e}")),
                        }
                    }
                }
                k if k.starts_with("bind.") => {
                    let name = k["bind.".len()..].trim();
                    match bindings::find_action(name) {
                        None => warnings.push(format!("line {n}: unknown action '{name}' in bind.{name}")),
                        Some(i) => match bindings::parse_list(i, value) {
                            Ok(list) => {
                                binds.set(i, list);
                                explicit.retain(|&e| e != i);
                                explicit.push(i);
                            }
                            Err(e) => warnings.push(format!("line {n}: {e}")),
                        },
                    }
                }
                k if k.starts_with("alias.") => {
                    let stem = k["alias.".len()..].trim().trim_end_matches(".exe").to_string();
                    if stem.is_empty() || value.is_empty() {
                        warnings.push(format!("line {n}: alias needs an exe name and a display name"));
                    } else {
                        cfg.aliases.retain(|(s, _)| *s != stem);
                        cfg.aliases.push((stem, value.to_string()));
                    }
                }
                _ => warnings.push(format!("line {n}: unknown key '{key}'")),
            }
        }
        if cfg.cheatsheet_key != 0 && cfg.cheatsheet_key == cfg.overview_key {
            warnings
                .push("cheatsheet_key equals overview_key: cheatsheet key disabled".to_string());
            cfg.cheatsheet_key = 0;
        }
        // overview_key / cheatsheet_key (legacy settings) = the default keys for those actions,
        // unless configured via bind.*
        if let Some(i) = bindings::find_action("overview").filter(|i| !explicit.contains(i)) {
            let list = if cfg.overview_key != 0 {
                vec![Chord::new(WIN, cfg.overview_key)]
            } else {
                Vec::new()
            };
            binds.set(i, list);
        }
        if let Some(i) = bindings::find_action("cheatsheet").filter(|i| !explicit.contains(i)) {
            let list = if cfg.cheatsheet_key != 0 {
                vec![Chord::new(WIN, cfg.cheatsheet_key)]
            } else {
                Vec::new()
            };
            binds.set(i, list);
        }
        warnings.extend(binds.resolve_conflicts(&explicit));
        cfg.bindings = binds;
        // Presets: built-in first, then custom (matching names override in place).
        let mut presets = if snap_defaults {
            layout::builtin_presets()
        } else {
            Vec::new()
        };
        for (name, preset) in snap_user {
            match preset {
                Some(p) => match presets.iter().position(|x| x.name == name) {
                    Some(i) => presets[i] = p,
                    None => presets.push(p),
                },
                None => presets.retain(|x| x.name != name),
            }
        }
        cfg.snap_layouts = presets;
        (cfg, warnings)
    }
}

/// "#RRGGBB" / "RRGGBB" -> Some(Some(COLORREF)); "off"/"none"/empty -> Some(None);
/// otherwise None (invalid).
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

fn color_setting(value: &str, target: &mut u32, name: &str, n: usize, warnings: &mut Vec<String>) {
    match parse_color(value) {
        Some(Some(c)) => *target = c,
        _ => warnings.push(format!("line {n}: {name} must be #RRGGBB")),
    }
}

/// Key name -> virtual-key code. "off" = 0 (disabled). None = invalid
/// or conflicts with another used Win+... key (Q close, M minimize).
fn parse_key(v: &str) -> Option<u16> {
    let v = v.trim().to_lowercase();
    let code: u16 = match v.as_str() {
        "off" | "none" => return Some(0),
        "tab" => 0x09,
        "slash" => 0xBF,
        "space" => 0x20,
        s if s.len() >= 2
            && s.starts_with('f')
            && s[1..]
                .parse::<u16>()
                .map_or(false, |n| (1..=12).contains(&n)) =>
        {
            0x6F + s[1..].parse::<u16>().unwrap_or(1)
        }
        "grave" | "backtick" | "`" => 0xC0,
        s if s.len() == 1 && s.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
            s.to_ascii_uppercase().as_bytes()[0] as u16
        }
        _ => return None,
    };
    if code == 0x51 || code == 0x4D {
        return None;
    }
    Some(code)
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// Default display name for executables with awkward names.
const BUILTIN_ALIASES: &[(&str, &str)] = &[
    ("code", "VS Code"),
    ("windowsterminal", "Windows Terminal"),
    ("explorer", "File Explorer"),
    ("msedge", "Edge"),
    ("chrome", "Chrome"),
    ("firefox", "Firefox"),
    ("vivaldi", "Vivaldi"),
    ("notepad", "Notepad"),
    ("claude", "Claude"),
    ("discord", "Discord"),
    ("spotify", "Spotify"),
    ("slack", "Slack"),
    ("ms-teams", "Teams"),
    ("wezterm-gui", "WezTerm"),
];

/// App name from the executable name (e.g. "vivaldi.exe" -> "Vivaldi"). None if
/// uninformative (UWP host), so the caller uses the window title.
pub fn app_name(exe: &str, aliases: &[(String, String)]) -> Option<String> {
    let lower = exe.trim().to_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower).to_string();
    if let Some((_, name)) = aliases.iter().find(|(s, _)| *s == stem) {
        return Some(name.clone());
    }
    if let Some((_, name)) = BUILTIN_ALIASES.iter().find(|(s, _)| *s == stem) {
        return Some((*name).to_string());
    }
    if stem.is_empty() || stem == "applicationframehost" {
        return None;
    }
    let mut chars = stem.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().collect::<String>() + chars.as_str())
}

/// "title - section - Vivaldi" -> "Vivaldi - title - section" (split at the last separator).
fn swap_around_separator(title: &str) -> String {
    let mut best: Option<(usize, &str)> = None;
    for sep in [" - ", " — ", " – "] {
        if let Some(i) = title.rfind(sep) {
            if best.map_or(true, |(b, _)| i > b) {
                best = Some((i, sep));
            }
        }
    }
    match best {
        Some((i, sep)) => {
            let (left, right) = (&title[..i], &title[i + sep.len()..]);
            if left.is_empty() || right.is_empty() {
                title.to_string()
            } else {
                format!("{right}{sep}{left}")
            }
        }
        None => title.to_string(),
    }
}

/// Overview box label for the selected mode.
pub fn label(
    mode: LabelMode,
    title: &str,
    exe: Option<&str>,
    aliases: &[(String, String)],
) -> String {
    match mode {
        LabelMode::Title => title.to_string(),
        LabelMode::Reverse => swap_around_separator(title),
        LabelMode::App => exe
            .and_then(|e| app_name(e, aliases))
            .unwrap_or_else(|| title.to_string()),
    }
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
    if p.exists() {
        return Some(p);
    }
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(dir).join("vscroll-wm").join("vscroll-wm.conf");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Result of loading configuration: its contents, any issues found (to display
/// to the user), and the file that was read.
pub struct LoadReport {
    pub cfg: Config,
    pub problems: Vec<String>,
    pub path: Option<PathBuf>,
}

/// Load configuration: defaults, then file, then environment variables.
pub fn load() -> LoadReport {
    let mut cfg = Config::default();
    let mut problems = Vec::new();
    let path = find_file();
    if let Some(path) = &path {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let (c, warnings) = Config::parse(&text);
                cfg = c;
                crate::log_info!("loaded config from {}", path.display());
                for w in &warnings {
                    crate::log_warn!("config: {w}");
                }
                problems = warnings;
            }
            Err(e) => {
                crate::log_warn!("failed to read {}: {e}", path.display());
                problems.push(format!("could not read the file: {e}"));
            }
        }
    }
    if let Some(v) = std::env::var("VSCROLL_GAP")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
    {
        cfg.gap = v.clamp(0, 200);
    }
    if let Some(v) = std::env::var("VSCROLL_ANIM_MS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    {
        if v.is_finite() {
            cfg.anim_ms = Some(v.clamp(0.0, 1000.0));
        }
    }
    LoadReport {
        cfg,
        problems,
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::ALT;

    #[test]
    fn empty_text_gives_defaults() {
        let (cfg, warnings) = Config::parse("");
        assert_eq!(cfg, Config::default());
        assert!(!cfg.workspace_osd_dots);
        assert!(warnings.is_empty());
        let (cfg, w) = Config::parse("workspace_osd_dots = on\n");
        assert!(cfg.workspace_osd_dots);
        assert!(w.is_empty());
    }

    #[test]
    fn parses_values_comments_and_case() {
        let (cfg, w) = Config::parse("# komentar\nGap = 24\nanim_ms=200\nfocus_border = #4FA8F5\n");
        assert_eq!(cfg.gap, 24);
        assert_eq!(cfg.anim_ms, Some(200.0));
        assert_eq!(cfg.focus_border, Some(0x00F5_A84F)); // BGR for RGB #4FA8F5.
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
    fn join_border_accepts_hex_and_off() {
        let (a, w) = Config::parse("join_border = #FFBE3C");
        assert!(w.is_empty());
        assert_eq!(a.join_border, Some(0x003C_BEFF));
        let (b, w) = Config::parse("join_border = off");
        assert!(w.is_empty());
        assert_eq!(b.join_border, None);
    }

    #[test]
    fn parses_window_rule_lines_and_warns_on_bad_ones() {
        let (cfg, w) = Config::parse(
            "rule = exe:vlc.exe, float, width=800, height=500\n\
             rule = exe:discord.exe, workspace=2, span=right, height=1/2\n\
             rule = bad_token\n",
        );
        assert_eq!(cfg.rules.len(), 2);
        assert_eq!(w.len(), 1);
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

    #[test]
    fn parses_overview_options_and_aliases() {
        let (cfg, w) = Config::parse(
            "overview_hint = off\noverview_minimap = left\noverview_thumbnails = no\noverview_label = Reverse\nalias.code = Kode\nalias.Foo.exe = Fu\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert!(!cfg.overview_hint);
        assert_eq!(cfg.overview_minimap, MinimapSide::Left);
        assert!(!cfg.overview_thumbnails);
        assert_eq!(cfg.overview_label, LabelMode::Reverse);
        assert_eq!(
            cfg.aliases,
            vec![
                ("code".to_string(), "Kode".to_string()),
                ("foo".to_string(), "Fu".to_string())
            ]
        );
        assert_eq!(
            Config::parse("overview_minimap = off").0.overview_minimap,
            MinimapSide::Off
        );
    }

    #[test]
    fn bad_overview_values_warn() {
        let (cfg, w) =
            Config::parse("overview_hint = mungkin\noverview_label = hias\nalias. = x\n");
        assert_eq!(w.len(), 3);
        assert!(cfg.overview_hint);
        assert_eq!(cfg.overview_label, LabelMode::Title);
    }

    #[test]
    fn reverse_label_moves_the_last_segment_to_front() {
        let al: Vec<(String, String)> = vec![];
        assert_eq!(
            label(LabelMode::Reverse, "Percakapan baru - Vivaldi", None, &al),
            "Vivaldi - Percakapan baru"
        );
        assert_eq!(
            label(LabelMode::Reverse, "a - b - Vivaldi", None, &al),
            "Vivaldi - a - b"
        );
        assert_eq!(
            label(LabelMode::Reverse, "Tanpa pemisah", None, &al),
            "Tanpa pemisah"
        );
        assert_eq!(
            label(LabelMode::Reverse, "Judul — Edge", None, &al),
            "Edge — Judul"
        );
    }

    #[test]
    fn app_label_uses_exe_name_and_falls_back_to_title() {
        let al = vec![("vivaldi".to_string(), "Viv".to_string())];
        assert_eq!(
            label(LabelMode::App, "x - Vivaldi", Some("vivaldi.exe"), &al),
            "Viv"
        );
        assert_eq!(label(LabelMode::App, "x", Some("code.exe"), &[]), "VS Code");
        assert_eq!(label(LabelMode::App, "x", Some("foo.exe"), &[]), "Foo");
        assert_eq!(
            label(
                LabelMode::App,
                "Kalkulator",
                Some("applicationframehost.exe"),
                &[]
            ),
            "Kalkulator"
        );
        assert_eq!(label(LabelMode::App, "judul", None, &[]), "judul");
    }

    #[test]
    fn parses_overview_size_key_and_colors() {
        let (cfg, w) = Config::parse(
            "overview_size = 55\noverview_key = z\noverview_color_selected = #FF0000\noverview_color_view = 00FF00\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.overview_size, 55);
        assert_eq!(cfg.overview_key, 0x5A);
        assert_eq!(cfg.overview_colors.selected, 0x0000_00FF);
        assert_eq!(cfg.overview_colors.view, 0x0000_FF00);
        assert_eq!(cfg.overview_colors.active, OverviewColors::default().active);
    }

    #[test]
    fn overview_key_names_and_rejections() {
        assert_eq!(parse_key("tab"), Some(0x09));
        assert_eq!(parse_key("Space"), Some(0x20));
        assert_eq!(parse_key("grave"), Some(0xC0));
        assert_eq!(parse_key("Z"), Some(0x5A));
        assert_eq!(parse_key("3"), Some(0x33));
        assert_eq!(parse_key("off"), Some(0));
        assert_eq!(parse_key("q"), None); // Reserved: close window.
        assert_eq!(parse_key("m"), None); // Reserved: minimize.
        assert_eq!(parse_key("enter"), None);
    }

    #[test]
    fn overview_size_is_clamped_and_bad_colors_warn() {
        let (cfg, w) = Config::parse(
            "overview_size = 5\noverview_color_box = biru\noverview_color_active = off\n",
        );
        assert_eq!(cfg.overview_size, 30);
        assert_eq!(w.len(), 2);
        assert_eq!(cfg.overview_colors, OverviewColors::default());
    }

    #[test]
    fn resize_step_is_parsed_and_clamped() {
        let (cfg, w) = Config::parse("resize_step = 120");
        assert!(w.is_empty());
        assert_eq!(cfg.resize_step, 120);
        assert_eq!(Config::parse("resize_step = 1").0.resize_step, 10);
        assert_eq!(Config::parse("resize_step = 9999").0.resize_step, 400);
        assert_eq!(Config::default().resize_step, 80);
        let (cfg, w) = Config::parse("resize_step = lebar");
        assert_eq!((cfg.resize_step, w.len()), (80, 1));
    }

    #[test]
    fn cheatsheet_key_parsing_and_conflict_with_overview_key() {
        assert_eq!(Config::default().cheatsheet_key, 0xBF);
        assert_eq!(Config::parse("cheatsheet_key = f1").0.cheatsheet_key, 0x70);
        assert_eq!(Config::parse("cheatsheet_key = F12").0.cheatsheet_key, 0x7B);
        assert_eq!(Config::parse("cheatsheet_key = h").0.cheatsheet_key, 0x48);
        assert_eq!(Config::parse("cheatsheet_key = off").0.cheatsheet_key, 0);
        assert_eq!(parse_key("f13"), None);
        assert_eq!(parse_key("f"), Some(0x46)); // Letter F, not the F-key.
        let (cfg, w) = Config::parse("overview_key = z\ncheatsheet_key = z\n");
        assert_eq!(cfg.cheatsheet_key, 0); // bentrok: cheatsheet mengalah
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn height_mode_and_step_are_parsed() {
        let d = Config::default();
        assert_eq!((d.height_mode, d.height_step), (HeightMode::Preset, 80));
        let (cfg, w) = Config::parse("height_mode = Step\nheight_step = 120");
        assert!(w.is_empty());
        assert_eq!((cfg.height_mode, cfg.height_step), (HeightMode::Step, 120));
        assert_eq!(Config::parse("height_step = 1").0.height_step, 10);
        assert_eq!(Config::parse("height_step = 9999").0.height_step, 400);
        let (cfg, w) = Config::parse("height_mode = pixel");
        assert_eq!((cfg.height_mode, w.len()), (HeightMode::Preset, 1));
    }

    #[test]
    fn mouse_drag_defaults_to_on() {
        assert!(Config::default().mouse_drag);
        let (cfg, w) = Config::parse("mouse_drag = off");
        assert!(!cfg.mouse_drag && w.is_empty());
        let (cfg, w) = Config::parse("mouse_drag = sering");
        assert!(cfg.mouse_drag);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn remember_layout_defaults_to_on() {
        assert!(Config::default().remember_layout);
        let (cfg, w) = Config::parse("remember_layout = off");
        assert!(!cfg.remember_layout && w.is_empty());
        let (cfg, w) = Config::parse("remember_layout = mungkin");
        assert!(cfg.remember_layout);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn workspace_osd_defaults_to_on() {
        assert!(Config::default().workspace_osd);
        let (cfg, w) = Config::parse("workspace_osd = off");
        assert!(!cfg.workspace_osd && w.is_empty());
        let (cfg, w) = Config::parse("workspace_osd = ya");
        assert!(cfg.workspace_osd);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn overview_wrap_defaults_to_on() {
        assert!(Config::default().overview_wrap);
        let (cfg, w) = Config::parse("overview_wrap = off");
        assert!(!cfg.overview_wrap && w.is_empty());
        let (cfg, w) = Config::parse("overview_wrap = kadang");
        assert!(cfg.overview_wrap);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn bind_lines_change_shortcuts() {
        let (cfg, w) = Config::parse("bind.close = win+x\nbind.minimize = off\nbind.overview = ctrl+alt+o\n");
        assert!(w.is_empty(), "{w:?}");
        let b = &cfg.bindings;
        assert_eq!(b.lookup(WIN, 0x58), Some(crate::consts::HOTKEY_CLOSE));
        assert_eq!(b.lookup(WIN, 0x51), None); // Win+Q no longer closes the window.
        assert_eq!(b.lookup(WIN, 0x4D), None); // Minimize is disabled.
        assert_eq!(
            b.lookup(bindings::CTRL | ALT, 0x4F),
            Some(crate::consts::HOTKEY_OVERVIEW)
        );
        assert_eq!(b.lookup(WIN, 0x09), None); // Win+Tab is unbound (bind.overview replaces it).
    }

    #[test]
    fn bad_bind_lines_warn_and_keep_defaults() {
        let (cfg, w) = Config::parse("bind.nope = win+x\nbind.close = win+\nbind.float = hyper+f\n");
        assert_eq!(w.len(), 3, "{w:?}");
        assert_eq!(cfg.bindings, Bindings::defaults());
    }

    #[test]
    fn bind_conflicts_are_resolved_in_favour_of_the_explicit_line() {
        let (cfg, w) = Config::parse("bind.close = win+m\n");
        assert_eq!(w.len(), 1, "{w:?}"); // minimize kehilangan Win+M
        assert_eq!(cfg.bindings.lookup(WIN, 0x4D), Some(crate::consts::HOTKEY_CLOSE));
    }

    #[test]
    fn legacy_keys_still_set_the_default_chords() {
        let (cfg, w) = Config::parse("overview_key = z\ncheatsheet_key = f1\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.bindings.lookup(WIN, 0x5A), Some(crate::consts::HOTKEY_OVERVIEW));
        assert_eq!(cfg.bindings.lookup(WIN, 0x70), Some(crate::consts::HOTKEY_CHEATSHEET));
        assert_eq!(cfg.bindings.lookup(WIN | ALT, 0x20), Some(crate::consts::HOTKEY_PICK));
        assert_eq!(cfg.bindings.lookup(WIN, 0x09), None);
        // bind.* takes precedence over overview_key
        let (cfg, _) = Config::parse("overview_key = z\nbind.overview = win+o\n");
        assert_eq!(cfg.bindings.lookup(WIN, 0x4F), Some(crate::consts::HOTKEY_OVERVIEW));
        assert_eq!(cfg.bindings.lookup(WIN, 0x5A), None);
        // overview_key = off: overview has no key
        let (cfg, _) = Config::parse("overview_key = off\n");
        assert_eq!(cfg.bindings.lookup(WIN, 0x09), None);
        assert_eq!(cfg.bindings.overview_cycle_vk(), 0);
    }

    fn names(cfg: &Config) -> Vec<&str> {
        cfg.snap_layouts.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn snap_layouts_default_to_on_with_the_builtin_presets() {
        let cfg = Config::default();
        assert!(cfg.snap_layouts_enabled && cfg.snap_assist);
        assert_eq!(cfg.snap_trigger, 24);
        assert_eq!(cfg.snap_layouts, layout::builtin_presets());
        assert!(names(&cfg).contains(&"halves") && names(&cfg).contains(&"quarters"));
        let (cfg, w) = Config::parse("snap_layouts = off\nsnap_assist = off\nsnap_trigger = 40\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!cfg.snap_layouts_enabled && !cfg.snap_assist);
        assert_eq!(cfg.snap_trigger, 40);
    }

    #[test]
    fn custom_snap_layouts_are_appended_after_the_builtin_ones() {
        let (cfg, w) = Config::parse("snap_layout.Focus = split=70, L, R:1/2, R:1/2\n");
        assert!(w.is_empty(), "{w:?}");
        let n = layout::builtin_presets().len();
        assert_eq!(cfg.snap_layouts.len(), n + 1);
        let p = cfg.snap_layouts.last().unwrap();
        assert_eq!(p.name, "focus"); // Config keys are case-insensitive.
        assert_eq!(p.split, 700);
        assert_eq!(p.zones.len(), 3);
    }

    #[test]
    fn a_custom_layout_with_a_builtin_name_replaces_it_in_place() {
        let before = Config::default();
        let at = names(&before).iter().position(|x| *x == "halves").unwrap();
        let (cfg, w) = Config::parse("snap_layout.halves = split=60, L, R\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.snap_layouts.len(), before.snap_layouts.len());
        assert_eq!(cfg.snap_layouts[at].name, "halves");
        assert_eq!(cfg.snap_layouts[at].split, 600);
    }

    #[test]
    fn builtin_layouts_can_be_removed_or_turned_off_entirely() {
        let (cfg, w) = Config::parse("snap_layout.quarters = off\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!names(&cfg).contains(&"quarters") && names(&cfg).contains(&"halves"));

        // Line order does not matter for snap_layouts_defaults.
        let (cfg, w) = Config::parse("snap_layout.mine = L, R\nsnap_layouts_defaults = off\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(names(&cfg), vec!["mine"]);
    }

    #[test]
    fn bad_snap_layout_lines_warn_and_are_skipped() {
        let (cfg, w) = Config::parse(
            "snap_layout.a = L, Q\nsnap_layout. = L\nsnap_layout.b =\nsnap_trigger = x\nsnap_assist = maybe\n",
        );
        assert_eq!(w.len(), 5, "{w:?}");
        assert_eq!(cfg.snap_layouts, layout::builtin_presets());
    }

    #[test]
    fn the_last_definition_of_a_layout_wins() {
        let (cfg, _) = Config::parse("snap_layout.x = L\nsnap_layout.x = L, R\n");
        let x: Vec<_> = cfg.snap_layouts.iter().filter(|p| p.name == "x").collect();
        assert_eq!(x.len(), 1);
        assert_eq!(x[0].zones.len(), 2);
    }
}
