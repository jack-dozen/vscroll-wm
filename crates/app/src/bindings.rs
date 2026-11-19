//! Keyboard shortcuts: one table (`ACTIONS`) for all bindable actions.
//!
//! Each action has a config name (`bind.<name>`), a hotkey ID (sent to the main
//! thread), a group + description for the cheatsheet, default keys, and whether
//! holding the key repeats the action. The keyboard hook, config parser, and
//! cheatsheet all read this table, so new actions only need to be added here:
//! they automatically become configurable and appear in the cheatsheet (the
//! tests below enforce this).
//!
//! Format in vscroll-wm.conf:
//!   bind.close = win+q                  one key
//!   bind.overview = win+tab, win+alt+space   multiple keys (comma-separated)
//!   bind.minimize = off                 disable
//!   bind.workspace_goto = win+ctrl+1-9  numbered action: "1-9" keys = workspaces 1..9
//! Modifiers: win, ctrl, alt, shift (at least one). Keys: letters/numbers, left/right/up/down,
//! enter, esc, space, tab, backspace, delete, insert, home, end, pageup, pagedown, f1-f12,
//! slash, backslash, lbracket, rbracket, grave, comma, period, minus, equals, semicolon, quote.

use std::sync::RwLock;

use crate::consts::{
    HOTKEY_CHEATSHEET, HOTKEY_CLOSE, HOTKEY_FLOAT, HOTKEY_INSPECT, HOTKEY_JOIN_DOWN,
    HOTKEY_JOIN_LEFT, HOTKEY_JOIN_RIGHT, HOTKEY_JOIN_UP, HOTKEY_MAXIMIZE, HOTKEY_MINIMIZE,
    HOTKEY_OVERVIEW, HOTKEY_PICK, HOTKEY_QUIT, HOTKEY_SPLIT_LEFT, HOTKEY_SPLIT_RESET,
    HOTKEY_SPLIT_RIGHT, HOTKEY_WS_GO, HOTKEY_WS_NEXT, HOTKEY_WS_NOOP, HOTKEY_WS_PREV,
    HOTKEY_WS_SEND, HOTKEY_WS_SEND_NEXT, HOTKEY_WS_SEND_PREV,
};

// ---------------------------------------------------------------- modifiers

pub const WIN: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const SHIFT: u8 = 8;

/// Virtual-key codes used outside the key-name table.
const VK_TAB: u16 = 0x09;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_D: u16 = 0x44;
const VK_F4: u16 = 0x73;

// ------------------------------------------------------------------ groups

pub const G_LAYOUT: &str = "Window layout";
pub const G_MOVE: &str = "Moving around";
pub const G_WORKSPACE: &str = "Workspaces";
pub const G_WINDOW: &str = "Windows";
pub const G_APP: &str = "App";
pub const G_OVERVIEW: &str = "Inside the overview";

// -------------------------------------------------------------------- table

pub struct ActionDef {
    /// Name in the config: `bind.<name>`.
    pub name: &'static str,
    /// Hotkey ID sent to the main thread (numbered action: base ID plus 0..8).
    pub id: i32,
    /// Group title in the cheatsheet.
    pub group: &'static str,
    /// Cheatsheet description. Actions in the same group with the same description
    /// are combined into one row (e.g. ↑ and ↓). Placeholders: {resize_step}, {height_note}.
    pub desc: &'static str,
    /// Default keys (same format as the config).
    pub defaults: &'static [&'static str],
    /// Holding the key repeats the action (move / resize). Others trigger once per press.
    pub repeat: bool,
    /// Numbered action: keys are "1-9" and the base ID is incremented by 0..8.
    pub numbered: bool,
}

const fn act(
    name: &'static str,
    id: i32,
    group: &'static str,
    desc: &'static str,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        name,
        id,
        group,
        desc,
        defaults,
        repeat: false,
        numbered: false,
    }
}

const fn rep(
    name: &'static str,
    id: i32,
    group: &'static str,
    desc: &'static str,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        name,
        id,
        group,
        desc,
        defaults,
        repeat: true,
        numbered: false,
    }
}

const fn num(
    name: &'static str,
    id: i32,
    group: &'static str,
    desc: &'static str,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        name,
        id,
        group,
        desc,
        defaults,
        repeat: false,
        numbered: true,
    }
}

const D_HEIGHT: &str = "shorter / taller{height_note}";
const D_COLUMN: &str = "move into the left / right column";
const D_JOIN: &str = "join with the neighbouring window (split / stack)";
const D_DIVIDER: &str = "move the column divider (±{resize_step} px)";
const D_FOCUS: &str = "focus the neighbouring window";
const D_SWAP: &str = "swap places with the neighbouring window";
const D_WS_STEP: &str = "previous / next workspace";
const D_WS_SEND_STEP: &str = "send window to previous / next workspace";

/// Table order = row order in the cheatsheet.
pub const ACTIONS: &[ActionDef] = &[
    // Window layout (IDs 1..4 = snap left, right, up, down)
    rep("snap_up", 3, G_LAYOUT, D_HEIGHT, &["win+up"]),
    rep("snap_down", 4, G_LAYOUT, D_HEIGHT, &["win+down"]),
    rep("snap_left", 1, G_LAYOUT, D_COLUMN, &["win+left"]),
    rep("snap_right", 2, G_LAYOUT, D_COLUMN, &["win+right"]),
    act("join_left", HOTKEY_JOIN_LEFT, G_LAYOUT, D_JOIN, &["win+alt+shift+left"]),
    act("join_right", HOTKEY_JOIN_RIGHT, G_LAYOUT, D_JOIN, &["win+alt+shift+right"]),
    act("join_up", HOTKEY_JOIN_UP, G_LAYOUT, D_JOIN, &["win+alt+shift+up"]),
    act("join_down", HOTKEY_JOIN_DOWN, G_LAYOUT, D_JOIN, &["win+alt+shift+down"]),
    act(
        "pick",
        HOTKEY_PICK,
        G_LAYOUT,
        "pick up window and choose drop spot",
        &["win+alt+space"],
    ),
    rep("split_right", HOTKEY_SPLIT_RIGHT, G_LAYOUT, D_DIVIDER, &["win+]"]),
    rep("split_left", HOTKEY_SPLIT_LEFT, G_LAYOUT, D_DIVIDER, &["win+["]),
    act(
        "split_reset",
        HOTKEY_SPLIT_RESET,
        G_LAYOUT,
        "reset the column divider to 50/50",
        &["win+\\"],
    ),
    act(
        "maximize",
        HOTKEY_MAXIMIZE,
        G_LAYOUT,
        "back to full width and height",
        &["win+enter"],
    ),
    // Moving around (IDs 5..8 = focus left, right, up, down; 10..13 = swap positions)
    rep("focus_left", 5, G_MOVE, D_FOCUS, &["win+shift+left"]),
    rep("focus_right", 6, G_MOVE, D_FOCUS, &["win+shift+right"]),
    rep("focus_up", 7, G_MOVE, D_FOCUS, &["win+shift+up"]),
    rep("focus_down", 8, G_MOVE, D_FOCUS, &["win+shift+down"]),
    rep("swap_left", 10, G_MOVE, D_SWAP, &["win+alt+left"]),
    rep("swap_right", 11, G_MOVE, D_SWAP, &["win+alt+right"]),
    rep("swap_up", 12, G_MOVE, D_SWAP, &["win+alt+up"]),
    rep("swap_down", 13, G_MOVE, D_SWAP, &["win+alt+down"]),
    act(
        "overview",
        HOTKEY_OVERVIEW,
        G_MOVE,
        "overview: map of the whole strip",
        &["win+tab"],
    ),
    // Workspaces
    act("workspace_prev", HOTKEY_WS_PREV, G_WORKSPACE, D_WS_STEP, &["win+ctrl+left"]),
    act("workspace_next", HOTKEY_WS_NEXT, G_WORKSPACE, D_WS_STEP, &["win+ctrl+right"]),
    num(
        "workspace_goto",
        HOTKEY_WS_GO,
        G_WORKSPACE,
        "go to workspace n",
        &["win+ctrl+1-9"],
    ),
    act(
        "workspace_send_prev",
        HOTKEY_WS_SEND_PREV,
        G_WORKSPACE,
        D_WS_SEND_STEP,
        &["win+ctrl+shift+left"],
    ),
    act(
        "workspace_send_next",
        HOTKEY_WS_SEND_NEXT,
        G_WORKSPACE,
        D_WS_SEND_STEP,
        &["win+ctrl+shift+right"],
    ),
    num(
        "workspace_send",
        HOTKEY_WS_SEND,
        G_WORKSPACE,
        "send window to workspace n",
        &["win+ctrl+shift+1-9"],
    ),
    // Windows
    act("close", HOTKEY_CLOSE, G_WINDOW, "close the window", &["win+q"]),
    act("minimize", HOTKEY_MINIMIZE, G_WINDOW, "minimize the window", &["win+m"]),
    act("float", HOTKEY_FLOAT, G_WINDOW, "float / tile the window", &["win+alt+f"]),
    act(
        "inspect",
        HOTKEY_INSPECT,
        G_WINDOW,
        "window info (class, exe, title)",
        &["win+alt+i"],
    ),
    // App
    act("cheatsheet", HOTKEY_CHEATSHEET, G_APP, "show this cheatsheet", &["win+/"]),
];

/// Keys that cannot be changed (checked before user bindings): emergency exit
/// and swallowed Windows virtual desktop shortcuts (replaced by our workspaces).
/// (modifier, vk, id hotkey)
pub const RESERVED: &[(u8, u16, i32)] = &[
    (WIN | CTRL, VK_ESCAPE, HOTKEY_QUIT),
    (WIN | CTRL, VK_D, HOTKEY_WS_NOOP),
    (WIN | CTRL, VK_F4, HOTKEY_WS_NOOP),
];

pub fn find_action(name: &str) -> Option<usize> {
    ACTIONS.iter().position(|a| a.name == name)
}

/// May holding the action key with ID `id` repeat it?
pub fn repeat_allowed(id: i32) -> bool {
    ACTIONS.iter().any(|a| a.repeat && a.id == id)
}

/// Workspace hotkey IDs (switch / send / swallowed virtual desktop shortcuts).
pub fn is_workspace_id(id: i32) -> bool {
    (HOTKEY_WS_PREV..=HOTKEY_WS_NOOP).contains(&id)
        || (HOTKEY_WS_GO..HOTKEY_WS_GO + 9).contains(&id)
        || (HOTKEY_WS_SEND..HOTKEY_WS_SEND + 9).contains(&id)
}

// -------------------------------------------------------------------- chords

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Vk(u16),
    /// Number keys 1..9 (only for numbered actions).
    Digits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub mods: u8,
    pub key: Key,
}

fn key_overlap(a: Key, b: Key) -> bool {
    match (a, b) {
        (Key::Vk(x), Key::Vk(y)) => x == y,
        (Key::Digits, Key::Digits) => true,
        (Key::Digits, Key::Vk(v)) | (Key::Vk(v), Key::Digits) => (0x31..=0x39).contains(&v),
    }
}

impl Chord {
    pub const fn new(mods: u8, vk: u16) -> Self {
        Chord {
            mods,
            key: Key::Vk(vk),
        }
    }

    /// Two chords that can be pressed together by one key combination.
    pub fn conflicts(&self, other: &Chord) -> bool {
        self.mods == other.mods && key_overlap(self.key, other.key)
    }

    /// Matches the currently pressed (modifier, vk) combination.
    fn hits(&self, mods: u8, vk: u16) -> Option<i32> {
        if self.mods != mods {
            return None;
        }
        match self.key {
            Key::Vk(k) if k == vk => Some(0),
            Key::Digits if (0x31..=0x39).contains(&vk) => Some((vk - 0x31) as i32),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        format!("{} + {}", mods_label(self.mods), key_label(self.key))
    }
}

/// "tab" / "left" / "f5" / "z" / "/" -> virtual-key code. None = unknown.
fn parse_key(tok: &str) -> Option<u16> {
    let t = tok.trim().to_lowercase();
    let code = match t.as_str() {
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => VK_ESCAPE,
        "space" => VK_SPACE,
        "tab" => VK_TAB,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "slash" | "/" => 0xBF,
        "backslash" | "\\" => 0xDC,
        "lbracket" | "[" => 0xDB,
        "rbracket" | "]" => 0xDD,
        "grave" | "backtick" | "`" => 0xC0,
        "comma" => 0xBC,
        "period" | "dot" => 0xBE,
        "minus" => 0xBD,
        "equals" | "plus" => 0xBB,
        "semicolon" => 0xBA,
        "quote" | "apostrophe" => 0xDE,
        s if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphanumeric()) => {
            s.to_ascii_uppercase().as_bytes()[0] as u16
        }
        s if s.len() >= 2
            && s.starts_with('f')
            && s[1..].parse::<u16>().map_or(false, |n| (1..=12).contains(&n)) =>
        {
            0x6F + s[1..].parse::<u16>().unwrap_or(1)
        }
        _ => return None,
    };
    Some(code)
}

/// "win+shift+left" -> Chord. `numbered`: numbered action; keys must be "1-9".
pub fn parse_chord(text: &str, numbered: bool) -> Result<Chord, String> {
    let lower = text.trim().to_lowercase();
    let mut parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    let key_tok = parts.pop().unwrap_or("");
    if key_tok.is_empty() {
        return Err(format!("'{}': missing key", text.trim()));
    }
    let mut mods = 0u8;
    for m in parts {
        let bit = match m {
            "win" | "windows" | "super" | "meta" => WIN,
            "ctrl" | "control" => CTRL,
            "alt" => ALT,
            "shift" => SHIFT,
            other => {
                return Err(format!(
                    "'{}': unknown modifier '{other}' (win, ctrl, alt, shift)",
                    text.trim()
                ))
            }
        };
        if mods & bit != 0 {
            return Err(format!("'{}': modifier given twice", text.trim()));
        }
        mods |= bit;
    }
    if mods == 0 {
        return Err(format!(
            "'{}': needs at least one modifier (win, ctrl, alt, shift)",
            text.trim()
        ));
    }
    let key = if matches!(key_tok, "1-9" | "1..9") {
        if !numbered {
            return Err(format!(
                "'{}': 1-9 is only for workspace_goto / workspace_send",
                text.trim()
            ));
        }
        Key::Digits
    } else if numbered {
        return Err(format!(
            "'{}': this action takes 1-9 as its key (e.g. win+ctrl+1-9)",
            text.trim()
        ));
    } else {
        match parse_key(key_tok) {
            Some(vk) => Key::Vk(vk),
            None => return Err(format!("'{}': unknown key '{key_tok}'", text.trim())),
        }
    };
    Ok(Chord { mods, key })
}

/// Parse `bind.<action>`: comma-separated chord list; empty / off / none = no keys.
pub fn parse_list(idx: usize, value: &str) -> Result<Vec<Chord>, String> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    let numbered = ACTIONS.get(idx).map_or(false, |a| a.numbered);
    v.split(',').map(|p| parse_chord(p, numbered)).collect()
}

// -------------------------------------------------------------------- labels

pub fn mods_label(mods: u8) -> String {
    let mut out: Vec<&str> = Vec::new();
    if mods & WIN != 0 {
        out.push("Win");
    }
    if mods & CTRL != 0 {
        out.push("Ctrl");
    }
    if mods & ALT != 0 {
        out.push("Alt");
    }
    if mods & SHIFT != 0 {
        out.push("Shift");
    }
    out.join(" + ")
}

/// Display name for a key, from its virtual-key code.
pub fn vk_label(vk: u16) -> String {
    match vk {
        0x25 => "←".to_string(),
        0x26 => "↑".to_string(),
        0x27 => "→".to_string(),
        0x28 => "↓".to_string(),
        0x0D => "Enter".to_string(),
        0x1B => "Esc".to_string(),
        0x20 => "Space".to_string(),
        0x09 => "Tab".to_string(),
        0x08 => "Backspace".to_string(),
        0x2E => "Delete".to_string(),
        0x2D => "Insert".to_string(),
        0x24 => "Home".to_string(),
        0x23 => "End".to_string(),
        0x21 => "PgUp".to_string(),
        0x22 => "PgDn".to_string(),
        0xBF => "/".to_string(),
        0xDC => "\\".to_string(),
        0xDB => "[".to_string(),
        0xDD => "]".to_string(),
        0xC0 => "`".to_string(),
        0xBC => ",".to_string(),
        0xBE => ".".to_string(),
        0xBD => "-".to_string(),
        0xBB => "=".to_string(),
        0xBA => ";".to_string(),
        0xDE => "'".to_string(),
        0x70..=0x7B => format!("F{}", vk - 0x6F),
        0x30..=0x39 | 0x41..=0x5A => ((vk as u8) as char).to_string(),
        _ => format!("0x{vk:02X}"),
    }
}

pub fn key_label(key: Key) -> String {
    match key {
        Key::Vk(v) => vk_label(v),
        Key::Digits => "1 … 9".to_string(),
    }
}

/// Key text for a cheatsheet row. Identical modifiers are combined:
/// "Win + ← / →"; all four arrows become "Win + Shift + arrows".
pub fn join_chords(chords: &[Chord]) -> String {
    let Some(first) = chords.first() else {
        return String::new();
    };
    if chords.iter().all(|c| c.mods == first.mods) {
        let vks: Vec<u16> = chords
            .iter()
            .filter_map(|c| match c.key {
                Key::Vk(v) => Some(v),
                Key::Digits => None,
            })
            .collect();
        let all_arrows = chords.len() == 4
            && vks.len() == 4
            && [0x25u16, 0x26, 0x27, 0x28].iter().all(|a| vks.contains(a));
        let keys = if all_arrows {
            "arrows".to_string()
        } else {
            chords
                .iter()
                .map(|c| key_label(c.key))
                .collect::<Vec<_>>()
                .join(" / ")
        };
        return format!("{} + {keys}", mods_label(first.mods));
    }
    chords
        .iter()
        .map(Chord::label)
        .collect::<Vec<_>>()
        .join(" / ")
}

// ----------------------------------------------------------------- Bindings

/// Keys for each action, in the same order as `ACTIONS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    chords: Vec<Vec<Chord>>,
}

impl Bindings {
    pub const fn empty() -> Self {
        Bindings { chords: Vec::new() }
    }

    pub fn defaults() -> Self {
        Bindings {
            chords: ACTIONS
                .iter()
                .map(|a| {
                    a.defaults
                        .iter()
                        .filter_map(|s| parse_chord(s, a.numbered).ok())
                        .collect::<Vec<Chord>>()
                })
                .collect(),
        }
    }

    pub fn chords(&self, idx: usize) -> &[Chord] {
        match self.chords.get(idx) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    pub fn set(&mut self, idx: usize, list: Vec<Chord>) {
        if let Some(slot) = self.chords.get_mut(idx) {
            *slot = list;
        }
    }

    /// Hotkey ID for a (modifier, vk) combination, if an action is bound to it.
    /// Numbered action: base ID + key number.
    pub fn lookup(&self, mods: u8, vk: u16) -> Option<i32> {
        for (i, list) in self.chords.iter().enumerate() {
            for c in list {
                if let Some(off) = c.hits(mods, vk) {
                    return ACTIONS.get(i).map(|a| a.id + off);
                }
            }
        }
        None
    }

    /// Key for cycling through windows while overview is open and Win is held: the
    /// overview key when it uses Win alone. 0 = none.
    pub fn overview_cycle_vk(&self) -> u16 {
        let Some(idx) = find_action("overview") else {
            return 0;
        };
        self.chords(idx)
            .iter()
            .find_map(|c| match c.key {
                Key::Vk(v) if c.mods == WIN => Some(v),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Remove conflicting chords: with reserved keys (RESERVED) or higher-priority
    /// actions. Priority order: `first` (explicitly configured actions, in file order),
    /// then the remaining actions in table order. Returns warnings.
    pub fn resolve_conflicts(&mut self, first: &[usize]) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.chords.len() != ACTIONS.len() {
            return warnings;
        }
        let mut order: Vec<usize> = first.to_vec();
        for i in 0..ACTIONS.len() {
            if !order.contains(&i) {
                order.push(i);
            }
        }
        let mut taken: Vec<(Chord, usize)> = Vec::new();
        for i in order {
            let list = std::mem::take(&mut self.chords[i]);
            let mut kept: Vec<Chord> = Vec::new();
            for c in list {
                if RESERVED
                    .iter()
                    .any(|r| c.conflicts(&Chord::new(r.0, r.1)))
                {
                    warnings.push(format!(
                        "{} (bind.{}) is reserved by the app and was ignored",
                        c.label(),
                        ACTIONS[i].name
                    ));
                    continue;
                }
                if let Some(other) = taken.iter().find(|t| t.0.conflicts(&c)) {
                    warnings.push(format!(
                        "{} is used by bind.{} and bind.{}: bind.{} was ignored",
                        c.label(),
                        ACTIONS[other.1].name,
                        ACTIONS[i].name,
                        ACTIONS[i].name
                    ));
                    continue;
                }
                taken.push((c, i));
                kept.push(c);
            }
            self.chords[i] = kept;
        }
        warnings
    }
}

// ------------------------------------------------------ active bindings

/// Bindings used by the keyboard hook. Populated from the main thread (apply_config);
/// the hook runs on the same thread, so there is no contention, but try_read is still
/// used to ensure the hook never waits.
static ACTIVE: RwLock<Bindings> = RwLock::new(Bindings::empty());

pub fn install(b: &Bindings) {
    if let Ok(mut g) = ACTIVE.write() {
        *g = b.clone();
    }
}

/// Hotkey ID for the currently pressed combination: reserved keys first, then user bindings.
pub fn lookup_active(mods: u8, vk: u16) -> Option<i32> {
    if let Some(r) = RESERVED.iter().find(|r| r.0 == mods && r.1 == vk) {
        return Some(r.2);
    }
    ACTIVE.try_read().ok().and_then(|g| g.lookup(mods, vk))
}

/// First currently active key for action `name`, for hint text. "unbound" if empty.
pub fn active_label(name: &str) -> String {
    let Some(i) = find_action(name) else {
        return String::new();
    };
    ACTIVE
        .read()
        .ok()
        .and_then(|g| g.chords(i).first().map(Chord::label))
        .unwrap_or_else(|| "unbound".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Bindings {
        Bindings::defaults()
    }

    #[test]
    fn every_default_binding_parses() {
        let b = defaults();
        for (i, a) in ACTIONS.iter().enumerate() {
            assert_eq!(b.chords(i).len(), a.defaults.len(), "{}", a.name);
        }
    }

    #[test]
    fn default_bindings_do_not_conflict() {
        let mut b = defaults();
        let w = b.resolve_conflicts(&[]);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(b, defaults());
    }

    #[test]
    fn action_names_and_ids_are_unique_and_described() {
        for (i, a) in ACTIONS.iter().enumerate() {
            assert!(!a.desc.is_empty() && !a.group.is_empty(), "{}", a.name);
            for b in &ACTIONS[i + 1..] {
                assert_ne!(a.name, b.name);
                let span = |x: &ActionDef| if x.numbered { 9 } else { 1 };
                let overlap = a.id < b.id + span(b) && b.id < a.id + span(a);
                assert!(!overlap, "{} / {}", a.name, b.name);
            }
        }
    }

    #[test]
    fn parses_chords_and_keys() {
        assert_eq!(
            parse_chord("Win+Shift+Left", false),
            Ok(Chord::new(WIN | SHIFT, 0x25))
        );
        assert_eq!(parse_chord(" super + ] ", false), Ok(Chord::new(WIN, 0xDD)));
        assert_eq!(parse_chord("alt+f5", false), Ok(Chord::new(ALT, 0x74)));
        assert_eq!(parse_chord("ctrl+f", false), Ok(Chord::new(CTRL, 0x46))); // Letter F.
        assert_eq!(parse_chord("win+slash", false), Ok(Chord::new(WIN, 0xBF)));
        assert_eq!(
            parse_chord("win+ctrl+1-9", true),
            Ok(Chord {
                mods: WIN | CTRL,
                key: Key::Digits
            })
        );
    }

    #[test]
    fn rejects_bad_chords() {
        assert!(parse_chord("q", false).is_err()); // No modifier.
        assert!(parse_chord("win+", false).is_err()); // No key.
        assert!(parse_chord("win+win+q", false).is_err()); // Duplicate modifier.
        assert!(parse_chord("hyper+q", false).is_err());
        assert!(parse_chord("win+f13", false).is_err());
        assert!(parse_chord("win+1-9", false).is_err()); // Not a numbered action.
        assert!(parse_chord("win+3", true).is_err()); // Numbered actions require 1-9.
    }

    #[test]
    fn list_values() {
        let close = find_action("close").unwrap();
        assert_eq!(parse_list(close, "off"), Ok(vec![]));
        assert_eq!(parse_list(close, ""), Ok(vec![]));
        assert_eq!(
            parse_list(close, "win+x, ctrl+alt+x"),
            Ok(vec![Chord::new(WIN, 0x58), Chord::new(CTRL | ALT, 0x58)])
        );
        assert!(parse_list(close, "win+x, nope").is_err());
    }

    #[test]
    fn default_lookup_matches_the_documented_shortcuts() {
        let b = defaults();
        assert_eq!(b.lookup(WIN, 0x25), Some(1)); // Win+Left = snap left.
        assert_eq!(b.lookup(WIN, 0x26), Some(3)); // Win+Up = shorter.
        assert_eq!(b.lookup(WIN | SHIFT, 0x27), Some(6)); // Focus right.
        assert_eq!(b.lookup(WIN | ALT, 0x28), Some(13)); // Swap down.
        assert_eq!(b.lookup(WIN | ALT | SHIFT, 0x25), Some(HOTKEY_JOIN_LEFT));
        assert_eq!(b.lookup(WIN | ALT | SHIFT, 0x27), Some(HOTKEY_JOIN_RIGHT));
        assert_eq!(b.lookup(WIN | ALT | SHIFT, 0x26), Some(HOTKEY_JOIN_UP));
        assert_eq!(b.lookup(WIN | ALT | SHIFT, 0x28), Some(HOTKEY_JOIN_DOWN));
        assert!(!repeat_allowed(HOTKEY_JOIN_UP));
        assert_eq!(b.lookup(WIN, 0x0D), Some(HOTKEY_MAXIMIZE));
        assert_eq!(b.lookup(WIN, 0x51), Some(HOTKEY_CLOSE));
        assert_eq!(b.lookup(WIN, 0x4D), Some(HOTKEY_MINIMIZE));
        assert_eq!(b.lookup(WIN | ALT, 0x46), Some(HOTKEY_FLOAT));
        assert_eq!(b.lookup(WIN | ALT, 0x49), Some(HOTKEY_INSPECT));
        assert_eq!(b.lookup(WIN, 0xDD), Some(HOTKEY_SPLIT_RIGHT));
        assert_eq!(b.lookup(WIN, 0xDB), Some(HOTKEY_SPLIT_LEFT));
        assert_eq!(b.lookup(WIN, 0xDC), Some(HOTKEY_SPLIT_RESET));
        assert_eq!(b.lookup(WIN, VK_TAB), Some(HOTKEY_OVERVIEW));
        assert_eq!(b.lookup(WIN | ALT, VK_SPACE), Some(HOTKEY_PICK));
        assert_eq!(b.lookup(WIN, 0xBF), Some(HOTKEY_CHEATSHEET));
        assert_eq!(b.lookup(WIN | CTRL, 0x25), Some(HOTKEY_WS_PREV));
        assert_eq!(b.lookup(WIN | CTRL, 0x27), Some(HOTKEY_WS_NEXT));
        assert_eq!(b.lookup(WIN | CTRL | SHIFT, 0x25), Some(HOTKEY_WS_SEND_PREV));
        assert_eq!(b.lookup(WIN | CTRL | SHIFT, 0x27), Some(HOTKEY_WS_SEND_NEXT));
        assert_eq!(b.lookup(WIN | CTRL, 0x31), Some(HOTKEY_WS_GO));
        assert_eq!(b.lookup(WIN | CTRL, 0x39), Some(HOTKEY_WS_GO + 8));
        assert_eq!(b.lookup(WIN | CTRL | SHIFT, 0x31), Some(HOTKEY_WS_SEND));
        assert_eq!(b.lookup(WIN | CTRL | SHIFT, 0x39), Some(HOTKEY_WS_SEND + 8));
    }

    #[test]
    fn lookup_needs_the_exact_modifiers() {
        let b = defaults();
        assert_eq!(b.lookup(WIN | SHIFT, 0x51), None); // Win+Shift+Q
        assert_eq!(b.lookup(WIN | CTRL, 0x26), None); // Win+Ctrl+Up
        assert_eq!(b.lookup(WIN | CTRL, 0x30), None); // '0'
        assert_eq!(b.lookup(0, 0x51), None);
    }

    #[test]
    fn reserved_keys_win_over_user_bindings() {
        assert_eq!(lookup_active(WIN | CTRL, VK_ESCAPE), Some(HOTKEY_QUIT));
        assert_eq!(lookup_active(WIN | CTRL, VK_D), Some(HOTKEY_WS_NOOP));
        assert_eq!(lookup_active(WIN | CTRL, VK_F4), Some(HOTKEY_WS_NOOP));
        let mut b = defaults();
        let close = find_action("close").unwrap();
        b.set(close, vec![Chord::new(WIN | CTRL, VK_ESCAPE)]);
        let w = b.resolve_conflicts(&[close]);
        assert_eq!(w.len(), 1);
        assert!(b.chords(close).is_empty());
    }

    #[test]
    fn explicit_binding_takes_the_key_from_a_default() {
        let mut b = defaults();
        let close = find_action("close").unwrap();
        let minimize = find_action("minimize").unwrap();
        // close moved to Win+M, previously assigned to minimize
        b.set(close, vec![Chord::new(WIN, 0x4D)]);
        let w = b.resolve_conflicts(&[close]);
        assert_eq!(w.len(), 1);
        assert_eq!(b.lookup(WIN, 0x4D), Some(HOTKEY_CLOSE));
        assert!(b.chords(minimize).is_empty());
    }

    #[test]
    fn digit_binding_conflicts_with_a_single_digit_chord() {
        let mut b = defaults();
        let close = find_action("close").unwrap();
        let goto = find_action("workspace_goto").unwrap();
        b.set(close, vec![Chord::new(WIN | CTRL, 0x33)]); // Win+Ctrl+3 is within 1-9.
        let w = b.resolve_conflicts(&[close]);
        assert_eq!(w.len(), 1);
        assert_eq!(b.lookup(WIN | CTRL, 0x33), Some(HOTKEY_CLOSE));
        assert!(b.chords(goto).is_empty()); // The 1-9 chord is dropped entirely (with a warning).
    }

    #[test]
    fn only_movement_and_resize_actions_repeat() {
        for id in [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13] {
            assert!(repeat_allowed(id), "{id}");
        }
        assert!(repeat_allowed(HOTKEY_SPLIT_RIGHT) && repeat_allowed(HOTKEY_SPLIT_LEFT));
        for id in [
            HOTKEY_CLOSE,
            HOTKEY_MINIMIZE,
            HOTKEY_FLOAT,
            HOTKEY_OVERVIEW,
            HOTKEY_CHEATSHEET,
            HOTKEY_QUIT,
            HOTKEY_MAXIMIZE,
            HOTKEY_INSPECT,
            HOTKEY_SPLIT_RESET,
            HOTKEY_WS_PREV,
            HOTKEY_WS_NEXT,
            HOTKEY_WS_SEND_PREV,
            HOTKEY_WS_SEND_NEXT,
            HOTKEY_WS_NOOP,
            HOTKEY_WS_GO,
            HOTKEY_WS_GO + 8,
            HOTKEY_WS_SEND,
            HOTKEY_WS_SEND + 8,
        ] {
            assert!(!repeat_allowed(id), "{id}");
        }
    }

    #[test]
    fn workspace_ids() {
        for id in [
            HOTKEY_WS_PREV,
            HOTKEY_WS_NEXT,
            HOTKEY_WS_SEND_PREV,
            HOTKEY_WS_SEND_NEXT,
            HOTKEY_WS_NOOP,
            HOTKEY_WS_GO,
            HOTKEY_WS_GO + 8,
            HOTKEY_WS_SEND,
            HOTKEY_WS_SEND + 8,
        ] {
            assert!(is_workspace_id(id), "{id}");
        }
        assert!(!is_workspace_id(HOTKEY_CLOSE));
        assert!(!is_workspace_id(HOTKEY_OVERVIEW));
        assert!(!is_workspace_id(1));
    }

    #[test]
    fn overview_cycle_key_follows_the_win_only_chord() {
        assert_eq!(defaults().overview_cycle_vk(), VK_TAB);
        let mut b = defaults();
        let ov = find_action("overview").unwrap();
        b.set(ov, vec![Chord::new(WIN, 0x5A), Chord::new(WIN | ALT, VK_SPACE)]);
        assert_eq!(b.overview_cycle_vk(), 0x5A);
        b.set(ov, vec![Chord::new(CTRL | ALT, 0x4F)]);
        assert_eq!(b.overview_cycle_vk(), 0); // Without Win: no cycling key.
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(vk_label(0x09), "Tab");
        assert_eq!(vk_label(0x5A), "Z");
        assert_eq!(vk_label(0x33), "3");
        assert_eq!(vk_label(0xBF), "/");
        assert_eq!(vk_label(0x70), "F1");
        assert_eq!(vk_label(0x7B), "F12");
        assert_eq!(Chord::new(WIN | CTRL | SHIFT, 0x25).label(), "Win + Ctrl + Shift + ←");
        let b = defaults();
        let idx = |n: &str| find_action(n).unwrap();
        let pair = [b.chords(idx("snap_left"))[0], b.chords(idx("snap_right"))[0]];
        assert_eq!(join_chords(&pair), "Win + ← / →");
        let arrows: Vec<Chord> = ["focus_left", "focus_right", "focus_up", "focus_down"]
            .iter()
            .map(|&n| b.chords(idx(n))[0])
            .collect();
        assert_eq!(join_chords(&arrows), "Win + Shift + arrows");
        let mixed = [Chord::new(WIN, 0x48), Chord::new(ALT, 0x4C)];
        assert_eq!(join_chords(&mixed), "Win + H / Alt + L");
    }
}
