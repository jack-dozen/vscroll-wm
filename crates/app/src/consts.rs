//! Constants: hotkey IDs, internal messages, and durations.

use layout::Dir;

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

pub(crate) const DIRS: [(VIRTUAL_KEY, Dir); 4] = [
    (VK_LEFT, Dir::Left),
    (VK_RIGHT, Dir::Right),
    (VK_UP, Dir::Up),
    (VK_DOWN, Dir::Down),
];

pub(crate) const HOTKEY_MAXIMIZE: i32 = 9;

pub(crate) const HOTKEY_QUIT: i32 = 14;

pub(crate) const HOTKEY_FLOAT: i32 = 15;

pub(crate) const HOTKEY_INSPECT: i32 = 16;

pub(crate) const HOTKEY_MINIMIZE: i32 = 17;

pub(crate) const HOTKEY_CLOSE: i32 = 18;

pub(crate) const HOTKEY_OVERVIEW: i32 = 19;

pub(crate) const HOTKEY_SPLIT_RIGHT: i32 = 20;

pub(crate) const HOTKEY_SPLIT_LEFT: i32 = 21;

pub(crate) const HOTKEY_SPLIT_RESET: i32 = 22;

pub(crate) const HOTKEY_CHEATSHEET: i32 = 23;

/// Message from the Windows event callback to the main thread.
pub(crate) const WM_WINEVENT: u32 = WM_APP + 3;

/// Delay (ms) for collecting a burst of events before rearranging.
pub(crate) const EVENT_DEBOUNCE_MS: u32 = 40;

/// Recheck after a new window is arranged (some apps change their own size
/// shortly after appearing).
pub(crate) const RECHECK_MS: u32 = 250;

/// Default animation duration (ms). Set via vscroll-wm.conf (anim_ms).
pub(crate) const DEFAULT_ANIM_MS: f32 = 160.0;

pub(crate) const WM_ACTION: u32 = WM_APP + 1;

/// Win+Ctrl+Left / Right: previous / next workspace.
pub(crate) const HOTKEY_WS_PREV: i32 = 24;

pub(crate) const HOTKEY_WS_NEXT: i32 = 25;

/// Win+Ctrl+Shift+Left / Right: send the active window to the previous / next workspace.
pub(crate) const HOTKEY_WS_SEND_PREV: i32 = 26;

pub(crate) const HOTKEY_WS_SEND_NEXT: i32 = 27;

/// Win+Ctrl+D / Win+Ctrl+F4 (create / close a Windows virtual desktop): swallow the shortcut.
pub(crate) const HOTKEY_WS_NOOP: i32 = 28;

/// Snap layouts: select a window in the slot list (wParam = window ID, lParam = slot ID).
pub(crate) const WM_SNAP_PICK: u32 = WM_APP + 20;

/// Snap layouts: cancel the slot list (Esc, close button, right-click).
pub(crate) const WM_SNAP_CANCEL: u32 = WM_APP + 21;

/// `join` targeting mode: modifier released -> join the last target.
pub(crate) const WM_JOIN_COMMIT: u32 = WM_APP + 22;

/// `join` targeting mode: Esc pressed -> cancel and restore focus to the source window.
pub(crate) const WM_JOIN_CANCEL: u32 = WM_APP + 23;

/// Lift-and-drop (`pick`) mode: move the marker (wParam = index in `DIRS`).
pub(crate) const WM_PICK_MOVE: u32 = WM_APP + 24;

/// Lift-and-drop (`pick`) mode: drop the window at the selected marker.
pub(crate) const WM_PICK_ACCEPT: u32 = WM_APP + 25;

/// Lift-and-drop (`pick`) mode: cancel.
pub(crate) const WM_PICK_CANCEL: u32 = WM_APP + 26;

/// Interval (ms) for tracking the cursor while a window is dragged for the snap layouts menu.
pub(crate) const SNAP_TICK_MS: u32 = 16;

/// Win+Ctrl+1..9 -> HOTKEY_WS_GO + 0..8 (switch to workspace n).
pub(crate) const HOTKEY_WS_GO: i32 = 30;

/// Win+Ctrl+Shift+1..9 -> HOTKEY_WS_SEND + 0..8 (send the active window to workspace n).
pub(crate) const HOTKEY_WS_SEND: i32 = 40;

/// Win+Alt+Shift+Arrow -> join a neighbor (ordered as in `DIRS`: Left, Right, Up, Down).
pub(crate) const HOTKEY_JOIN_LEFT: i32 = 50;
pub(crate) const HOTKEY_JOIN_RIGHT: i32 = 51;
pub(crate) const HOTKEY_JOIN_UP: i32 = 52;
pub(crate) const HOTKEY_JOIN_DOWN: i32 = 53;

/// Win+Alt+Space -> lift the active window and choose a destination with the arrows + overlay.
pub(crate) const HOTKEY_PICK: i32 = 54;

/// Win+Alt+S -> sticky overlay (always on top across all workspaces).
pub(crate) const HOTKEY_STICKY: i32 = 55;

/// Parking x-position for windows in other workspaces: the USER32 lower bound (i16::MIN);
/// smaller values are clamped to this. Deliberately not -32000 (the old "minimized" position).
pub(crate) const PARK_X: i32 = i16::MIN as i32;
