//! Konstanta: id hotkey, pesan internal, dan durasi.

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

/// Pesan dari callback event Windows ke thread utama.
pub(crate) const WM_WINEVENT: u32 = WM_APP + 3;

/// Jeda (ms) mengumpulkan rentetan event sebelum menata ulang.
pub(crate) const EVENT_DEBOUNCE_MS: u32 = 40;

/// Cek ulang setelah window baru ditata (sebagian app mengubah ukurannya sendiri
/// beberapa saat setelah muncul).
pub(crate) const RECHECK_MS: u32 = 250;

/// Durasi animasi bawaan (ms). Diubah lewat vscroll-wm.conf (anim_ms).
pub(crate) const DEFAULT_ANIM_MS: f32 = 160.0;

pub(crate) const WM_ACTION: u32 = WM_APP + 1;
