//! Low-level keyboard and Windows event hooks, with their key-action table.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::bindings::{self, ALT, CTRL, SHIFT, WIN};
use crate::consts::{
    DIRS, HOTKEY_OVERVIEW, HOTKEY_PICK, HOTKEY_QUIT, HOTKEY_WS_GO, HOTKEY_WS_NEXT, HOTKEY_WS_PREV,
    WM_ACTION, WM_JOIN_CANCEL, WM_JOIN_COMMIT, WM_PICK_ACCEPT, WM_PICK_CANCEL, WM_PICK_MOVE,
    WM_SNAP_CANCEL, WM_WINEVENT,
};
use crate::{overview, tray};

pub(crate) static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

pub(crate) static WIN_DOWN: AtomicBool = AtomicBool::new(false);

/// Key for cycling through windows while the overview is open and Win is held
/// (virtual-key code; 0 = disabled). Derived from the `overview` binding (see bindings.rs).
pub(crate) static OVERVIEW_VK: AtomicU16 = AtomicU16::new(0x09);

/// Shortcut key currently held, to recognize repeated events (auto-repeat).
pub(crate) static HELD_VK: AtomicU16 = AtomicU16::new(0);

/// Overview is open: tiling is disabled; Win+key only controls the overview.
pub(crate) static OV_OPEN: AtomicBool = AtomicBool::new(false);

/// Snap layouts window list is visible: Esc cancels it (the key is swallowed).
pub(crate) static SNAP_OPEN: AtomicBool = AtomicBool::new(false);

/// Selection has moved since the overview opened. If Win is released in this state,
/// jump directly to the selected window (Alt+Tab style).
pub(crate) static OV_MOVED: AtomicBool = AtomicBool::new(false);

/// Lift-and-drop (`pick`) mode is active: arrows move the marker, Enter / Space
/// drops, Esc cancels.
pub(crate) static PICK_OPEN: AtomicBool = AtomicBool::new(false);

/// `join` targeting mode is active (kept in case modifier-hold mode is used again):
/// releasing any modifier in `JOIN_MODS` sends `WM_JOIN_COMMIT`; Esc cancels.
pub(crate) static JOIN_ACTIVE: AtomicBool = AtomicBool::new(false);
static JOIN_MODS: AtomicU8 = AtomicU8::new(0);

fn modifier_bit(vk: u16) -> u8 {
    match VIRTUAL_KEY(vk) {
        VK_LWIN | VK_RWIN => WIN,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => SHIFT,
        VK_MENU | VK_LMENU | VK_RMENU => ALT,
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => CTRL,
        _ => 0,
    }
}

/// Win+... key action while the overview is open: (message, wParam, moves selection?).
/// `bound` = hotkey ID bound to this combination in the config (bindings.rs).
/// None = not an overview key; pass it through to Windows (no tiling action).
pub(crate) fn overview_hook_action(
    vk: u16,
    shift: bool,
    alt: bool,
    ctrl: bool,
    overview_vk: u16,
    bound: Option<i32>,
) -> Option<(u32, usize, bool)> {
    if vk == VK_ESCAPE.0 && ctrl {
        return Some((WM_ACTION, HOTKEY_QUIT as usize, false)); // Emergency exit.
    }
    // Workspace shortcuts (using current bindings): same as outside the overview,
    // but move the cursor.
    match bound {
        Some(HOTKEY_WS_PREV) => return Some((overview::WM_OV_WS, 0, true)),
        Some(HOTKEY_WS_NEXT) => return Some((overview::WM_OV_WS, 1, true)),
        Some(id) if (HOTKEY_WS_GO..HOTKEY_WS_GO + 9).contains(&id) => {
            return Some((overview::WM_OV_WS, 2 + (id - HOTKEY_WS_GO) as usize, true));
        }
        _ => {}
    }
    if let Some(i) = arrow_index(vk).filter(|_| !ctrl) {
        return Some((overview::WM_OV_MOVE, i as usize, true));
    }
    if vk == VK_RETURN.0 {
        return Some((overview::WM_OV_ACCEPT, 0, false));
    }
    if vk == VK_ESCAPE.0 {
        return Some((overview::WM_OV_CANCEL, 0, false));
    }
    if bound == Some(HOTKEY_OVERVIEW) {
        return Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false)); // Toggle / close.
    }
    let _ = (shift, alt, overview_vk);
    None
}

pub(crate) fn arrow_index(vk: u16) -> Option<i32> {
    DIRS.iter().position(|(k, _)| k.0 == vk).map(|i| i as i32)
}

/// Inject a dummy key so Windows does not open the Start Menu when Win is
/// released (we swallow the arrow, so Windows thinks Win was pressed alone).
pub(crate) unsafe fn cancel_start_menu() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0xE8),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

/// Non-Win modifiers currently held (SHIFT / ALT / CTRL bitmask from bindings.rs).
unsafe fn held_mods() -> u8 {
    let mut m = 0u8;
    if GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 {
        m |= SHIFT;
    }
    if GetAsyncKeyState(VK_MENU.0 as i32) < 0 {
        m |= ALT;
    }
    if GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 {
        m |= CTRL;
    }
    m
}

/// Send action `id` to the main thread and swallow the key. Repeats from holding
/// the key are ignored unless the action allows repetition.
unsafe fn fire(vk: u16, id: i32, down: bool, _mods: u8) -> LRESULT {
    if down {
        let repeat = HELD_VK.swap(vk, Ordering::Relaxed) == vk;
        if repeat && !bindings::repeat_allowed(id) {
            return LRESULT(1); // Key held: ignore repeats.
        }
        if id == HOTKEY_OVERVIEW && !OV_OPEN.load(Ordering::Relaxed) {
            // Optimistically mark the overview as open before the main thread
            // finishes opening it, so the next key (e.g. a second Tab) is handled correctly.
            OV_OPEN.store(true, Ordering::Relaxed);
            OV_MOVED.store(false, Ordering::Relaxed);
        }
        // Quick join executes immediately; to return to modifier-hold mode:
        // if (HOTKEY_JOIN_LEFT..=HOTKEY_JOIN_DOWN).contains(&id) {
        //     JOIN_MODS.store(_mods, Ordering::Relaxed);
        //     JOIN_ACTIVE.store(true, Ordering::Relaxed);
        // }
        let _ = PostThreadMessageW(
            MAIN_THREAD.load(Ordering::Relaxed),
            WM_ACTION,
            WPARAM(id as usize),
            LPARAM(0),
        );
        cancel_start_menu();
    }
    LRESULT(1) // telan tombolnya
}

/// Low-level keyboard hook. Must be fast: it only swallows keys and sends messages
/// to the main thread, which executes them in the message loop.
pub(crate) unsafe extern "system" fn kb_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && !tray::PAUSED.load(Ordering::Relaxed) {
        let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if k.flags.0 & LLKHF_INJECTED.0 == 0 {
            let m = wparam.0 as u32;
            let down = m == WM_KEYDOWN || m == WM_SYSKEYDOWN;
            let vk = k.vkCode as u16;
            if !down && HELD_VK.load(Ordering::Relaxed) == vk {
                HELD_VK.store(0, Ordering::Relaxed);
            }
            // Lift-and-drop (`pick`) mode: arrows move the marker, Enter / Space
            // drops, Esc / pick key cancels. Win+Ctrl+Esc remains the emergency exit.
            if PICK_OPEN.load(Ordering::Relaxed) {
                let win = WIN_DOWN.load(Ordering::Relaxed);
                let held = held_mods();
                let emergency_quit = vk == VK_ESCAPE.0 && win && (held & CTRL != 0);
                let bound = if win {
                    bindings::lookup_active(WIN | held, vk)
                } else if held != 0 {
                    bindings::lookup_active(held, vk)
                } else {
                    None
                };
                if !emergency_quit {
                    let action = if bound == Some(HOTKEY_PICK) {
                        Some((WM_PICK_CANCEL, 0usize, false))
                    } else if let Some(i) = arrow_index(vk).filter(|_| held & CTRL == 0) {
                        Some((WM_PICK_MOVE, i as usize, true))
                    } else if vk == VK_RETURN.0
                        || (vk == VK_SPACE.0 && !win && held == 0)
                    {
                        Some((WM_PICK_ACCEPT, 0, false))
                    } else if vk == VK_ESCAPE.0 {
                        Some((WM_PICK_CANCEL, 0, false))
                    } else {
                        None
                    };
                    if let Some((msg, wp, repeat_ok)) = action {
                        if down {
                            let repeat = HELD_VK.swap(vk, Ordering::Relaxed) == vk;
                            if repeat && !repeat_ok {
                                return LRESULT(1);
                            }
                            let _ = PostThreadMessageW(
                                MAIN_THREAD.load(Ordering::Relaxed),
                                msg,
                                WPARAM(wp),
                                LPARAM(0),
                            );
                            if win {
                                cancel_start_menu();
                            }
                        }
                        return LRESULT(1);
                    }
                }
            }
            // Join targeting mode: Esc cancels (except Win+Ctrl+Esc, the emergency exit);
            // releasing any held modifier immediately executes the join.
            if JOIN_ACTIVE.load(Ordering::Relaxed) {
                let emergency_quit =
                    vk == VK_ESCAPE.0 && WIN_DOWN.load(Ordering::Relaxed) && (held_mods() & CTRL != 0);
                if down && vk == VK_ESCAPE.0 && !emergency_quit {
                    JOIN_ACTIVE.store(false, Ordering::Relaxed);
                    JOIN_MODS.store(0, Ordering::Relaxed);
                    let _ = PostThreadMessageW(
                        MAIN_THREAD.load(Ordering::Relaxed),
                        WM_JOIN_CANCEL,
                        WPARAM(0),
                        LPARAM(0),
                    );
                    if WIN_DOWN.load(Ordering::Relaxed) {
                        cancel_start_menu();
                    }
                    return LRESULT(1);
                }
                if !down {
                    let bit = modifier_bit(vk);
                    if bit != 0
                        && (JOIN_MODS.load(Ordering::Relaxed) & bit) != 0
                        && JOIN_ACTIVE.swap(false, Ordering::Relaxed)
                    {
                        JOIN_MODS.store(0, Ordering::Relaxed);
                        let _ = PostThreadMessageW(
                            MAIN_THREAD.load(Ordering::Relaxed),
                            WM_JOIN_COMMIT,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            // Esc without modifiers closes the snap layouts window list.
            if down
                && vk == VK_ESCAPE.0
                && SNAP_OPEN.load(Ordering::Relaxed)
                && !WIN_DOWN.load(Ordering::Relaxed)
                && held_mods() == 0
            {
                let _ = PostThreadMessageW(
                    MAIN_THREAD.load(Ordering::Relaxed),
                    WM_SNAP_CANCEL,
                    WPARAM(0),
                    LPARAM(0),
                );
                return LRESULT(1);
            }
            if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
                WIN_DOWN.store(down, Ordering::Relaxed);
                if !down {
                    HELD_VK.store(0, Ordering::Relaxed);
                }
            } else if WIN_DOWN.load(Ordering::Relaxed)
                && (GetAsyncKeyState(VK_LWIN.0 as i32) < 0
                    || GetAsyncKeyState(VK_RWIN.0 as i32) < 0)
            {
                let held = held_mods();
                let (shift, alt, ctrl) = (held & SHIFT != 0, held & ALT != 0, held & CTRL != 0);
                // Action bound to this combination (reserved keys first, then config).
                let bound = bindings::lookup_active(WIN | held, vk);
                if OV_OPEN.load(Ordering::Relaxed) {
                    // Overview is open: tiling is disabled. Arrows move the selection, the
                    // overview key cycles, Enter / Esc selects / cancels. Pass other keys to Windows.
                    let ov_vk = OVERVIEW_VK.load(Ordering::Relaxed);
                    return match overview_hook_action(vk, shift, alt, ctrl, ov_vk, bound) {
                        Some((msg, wp, moves)) => {
                            if down {
                                let repeat = HELD_VK.swap(vk, Ordering::Relaxed) == vk;
                                if repeat && msg == WM_ACTION {
                                    return LRESULT(1); // Toggle / exit: once per press.
                                }
                                if moves {
                                    OV_MOVED.store(true, Ordering::Relaxed);
                                }
                                let _ = PostThreadMessageW(
                                    MAIN_THREAD.load(Ordering::Relaxed),
                                    msg,
                                    WPARAM(wp),
                                    LPARAM(0),
                                );
                                cancel_start_menu();
                            }
                            LRESULT(1) // telan tombolnya
                        }
                        None if bound.is_some_and(bindings::is_workspace_id) => {
                            // Windows virtual desktops are disabled: do not pass through.
                            if down {
                                cancel_start_menu();
                            }
                            LRESULT(1)
                        }
                        None => CallNextHookEx(None, code, wparam, lparam),
                    };
                }
                if let Some(id) = bound {
                    return fire(vk, id, down, WIN | held);
                }
            } else {
                // Shortcuts without Win (e.g. Alt+H or Ctrl+Alt+O): only when a modifier
                // is held. When the overview is open, only its own key applies (to close it);
                // other keys are handled by the overview window.
                let held = held_mods();
                if held != 0 {
                    if let Some(id) = bindings::lookup_active(held, vk) {
                        if !OV_OPEN.load(Ordering::Relaxed) || id == HOTKEY_OVERVIEW {
                            return fire(vk, id, down, held);
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Thread khusus yang memasang `WH_KEYBOARD_LL` dan memompa pesannya sendiri, supaya
/// pekerjaan thread utama (sync / apply / animasi / menu tray) tidak pernah menahan
/// callback keyboard hook melewati `LowLevelHooksTimeout`.
pub(crate) struct KeyboardHookThread {
    thread_id: u32,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl KeyboardHookThread {
    pub(crate) fn spawn() -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel::<Result<u32, String>>(1);
        let handle = std::thread::Builder::new()
            .name("vscroll-kb-hook".into())
            .spawn(move || unsafe {
                let tid = GetCurrentThreadId();
                // Pastikan antrean pesan thread ini sudah terbuat sebelum hook / WM_QUIT.
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), None, 0) {
                    Ok(h) => {
                        let _ = tx.send(Ok(tid));
                        h
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let _ = UnhookWindowsHookEx(hook);
            })
            .map_err(|e| e.to_string())?;
        match rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                handle: Some(handle),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(e) => {
                let _ = handle.join();
                Err(e.to_string())
            }
        }
    }

    pub(crate) fn stop(mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Windows event callback. Called on the main thread through the message loop;
/// deliberately does minimal work: forward as a message, filtering happens in `on_event`.
pub(crate) unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != 0 || id_child != 0 || hwnd.0.is_null() {
        return; // The window itself only, not its child elements.
    }
    let _ = PostThreadMessageW(
        MAIN_THREAD.load(Ordering::Relaxed),
        WM_WINEVENT,
        WPARAM(event as usize),
        LPARAM(hwnd.0 as isize),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::Bindings;

    const TAB: u16 = 0x09;

    /// Win + the given modifier combination, using the default binding.
    fn act(vk: u16, shift: bool, alt: bool, ctrl: bool) -> Option<(u32, usize, bool)> {
        let mods = WIN
            | if shift { SHIFT } else { 0 }
            | if alt { ALT } else { 0 }
            | if ctrl { CTRL } else { 0 };
        let bound = Bindings::defaults().lookup(mods, vk);
        overview_hook_action(vk, shift, alt, ctrl, TAB, bound)
    }

    #[test]
    fn arrows_move_the_overview_cursor_instead_of_tiling() {
        assert_eq!(
            act(VK_LEFT.0, false, false, false),
            Some((overview::WM_OV_MOVE, 0, true))
        );
        assert_eq!(
            act(VK_RIGHT.0, false, false, false),
            Some((overview::WM_OV_MOVE, 1, true))
        );
        assert_eq!(
            act(VK_UP.0, true, false, false),
            Some((overview::WM_OV_MOVE, 2, true))
        );
        assert_eq!(
            act(VK_DOWN.0, false, true, false),
            Some((overview::WM_OV_MOVE, 3, true))
        );
        // Win+Ctrl+Left/Right / number keys = jump workspaces (cursor).
        assert_eq!(act(VK_LEFT.0, false, false, true), Some((overview::WM_OV_WS, 0, true)));
        assert_eq!(act(VK_RIGHT.0, false, false, true), Some((overview::WM_OV_WS, 1, true)));
        assert_eq!(act(0x31, false, false, true), Some((overview::WM_OV_WS, 2, true)));
        assert_eq!(act(0x39, false, false, true), Some((overview::WM_OV_WS, 10, true)));
        assert_eq!(act(VK_LEFT.0, true, false, true), None); // Send window from overview: not implemented.
    }

    #[test]
    fn workspace_keys_in_the_overview_follow_custom_bindings() {
        // workspace_next moved to Win+Ctrl+L: in overview, also move the cursor.
        let mut b = Bindings::defaults();
        let next = bindings::find_action("workspace_next").unwrap();
        b.set(next, vec![bindings::Chord::new(WIN | CTRL, 0x4C)]);
        let bound = b.lookup(WIN | CTRL, 0x4C);
        assert_eq!(
            overview_hook_action(0x4C, false, false, true, TAB, bound),
            Some((overview::WM_OV_WS, 1, true))
        );
        // Its old key no longer means next workspace.
        let old = b.lookup(WIN | CTRL, VK_RIGHT.0);
        assert_eq!(overview_hook_action(VK_RIGHT.0, false, false, true, TAB, old), None);
    }

    #[test]
    fn overview_key_toggles_close_instead_of_cycling_while_holding_win() {
        assert_eq!(
            act(TAB, false, false, false),
            Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false))
        );
        // Win+Shift+Tab is not an overview-toggle shortcut.
        assert_eq!(act(TAB, true, false, false), None);
        // overview_key off: Win+Tab does nothing.
        assert_eq!(overview_hook_action(TAB, false, false, false, 0, None), None);
    }

    #[test]
    fn enter_escape_and_toggle_keys() {
        assert_eq!(
            act(VK_RETURN.0, false, false, false),
            Some((overview::WM_OV_ACCEPT, 0, false))
        );
        assert_eq!(
            act(VK_ESCAPE.0, false, false, false),
            Some((overview::WM_OV_CANCEL, 0, false))
        );
        // The overview can still be closed with a custom chord.
        assert_eq!(
            overview_hook_action(VK_SPACE.0, false, true, false, TAB, Some(HOTKEY_OVERVIEW)),
            Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false))
        );
    }

    #[test]
    fn emergency_quit_still_works_and_other_wm_keys_pass_through() {
        assert_eq!(
            act(VK_ESCAPE.0, false, false, true),
            Some((WM_ACTION, HOTKEY_QUIT as usize, false))
        );
        assert_eq!(act(VK_Q.0, false, false, false), None); // Close window: disabled.
        assert_eq!(act(VK_M.0, false, false, false), None); // Minimize: disabled.
        assert_eq!(act(VK_F.0, false, true, false), None); // Float: disabled.
    }
}
