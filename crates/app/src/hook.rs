//! Hook keyboard tingkat rendah dan event hook Windows, beserta tabel aksi tombolnya.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::consts::{
    DIRS, HOTKEY_CHEATSHEET, HOTKEY_CLOSE, HOTKEY_FLOAT, HOTKEY_INSPECT, HOTKEY_MAXIMIZE,
    HOTKEY_MINIMIZE, HOTKEY_OVERVIEW, HOTKEY_QUIT, HOTKEY_SPLIT_LEFT, HOTKEY_SPLIT_RESET,
    HOTKEY_SPLIT_RIGHT, WM_ACTION, WM_WINEVENT,
};
use crate::{overview, tray};

pub(crate) static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

pub(crate) static WIN_DOWN: AtomicBool = AtomicBool::new(false);

/// Tombol overview bersama Win (kode virtual-key; 0 = mati). Diatur dari konfigurasi.
pub(crate) static OVERVIEW_VK: AtomicU16 = AtomicU16::new(0x09);

/// Tombol cheatsheet bersama Win (0 = mati). Diatur dari konfigurasi.
pub(crate) static CHEAT_VK: AtomicU16 = AtomicU16::new(0xBF);

/// Tombol pintasan yang sedang ditahan, untuk mengenali event berulang (auto-repeat).
pub(crate) static HELD_VK: AtomicU16 = AtomicU16::new(0);

/// Overview sedang terbuka: tiling dimatikan, Win+tombol hanya mengendalikan overview.
pub(crate) static OV_OPEN: AtomicBool = AtomicBool::new(false);

/// Pilihan sudah digeser sejak overview dibuka. Kalau Win dilepas dalam keadaan ini,
/// overview langsung loncat ke window yang dipilih (gaya Alt+Tab).
pub(crate) static OV_MOVED: AtomicBool = AtomicBool::new(false);

/// Pintasan yang boleh berulang saat tombolnya ditahan: yang menggeser atau mengubah
/// ukuran. Sisanya (tutup, minimize, float, overview, cheatsheet, keluar, ...) hanya
/// sekali per tekan, supaya menahan Win+Q tidak menutup window berturut-turut.
pub(crate) fn repeat_allowed(id: i32) -> bool {
    matches!(id, 1..=8 | 10..=13 | HOTKEY_SPLIT_RIGHT | HOTKEY_SPLIT_LEFT)
}

/// Aksi tombol Win+... saat overview terbuka: (pesan, wParam, menggeser pilihan?).
/// None = bukan tombol overview; dibiarkan lewat ke Windows (tiling tidak ikut jalan).
pub(crate) fn overview_hook_action(
    vk: u16,
    shift: bool,
    alt: bool,
    ctrl: bool,
    overview_vk: u16,
) -> Option<(u32, usize, bool)> {
    if vk == VK_ESCAPE.0 && ctrl {
        return Some((WM_ACTION, HOTKEY_QUIT as usize, false)); // jalan keluar darurat
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
    if overview_vk != 0 && vk == overview_vk && !alt && !ctrl {
        return Some((overview::WM_OV_CYCLE, if shift { 0 } else { 1 }, true));
    }
    if vk == VK_SPACE.0 && alt {
        return Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false)); // tutup
    }
    None
}

pub(crate) fn arrow_index(vk: u16) -> Option<i32> {
    DIRS.iter().position(|(k, _)| k.0 == vk).map(|i| i as i32)
}

/// Selipkan tombol dummy supaya Windows tidak membuka Start Menu saat Win
/// dilepas (arrow-nya kita telan, jadi Windows mengira Win ditekan sendirian).
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

/// Low-level keyboard hook. Harus cepat: kerjanya cuma menelan tombol dan
/// mengirim pesan ke thread utama, eksekusinya di message loop.
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
            if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
                WIN_DOWN.store(down, Ordering::Relaxed);
                if !down {
                    HELD_VK.store(0, Ordering::Relaxed);
                }
                // Gaya Alt+Tab: Win dilepas setelah pilihan digeser = loncat ke window itu
                if !down
                    && OV_OPEN.load(Ordering::Relaxed)
                    && OV_MOVED.swap(false, Ordering::Relaxed)
                {
                    let _ = PostThreadMessageW(
                        MAIN_THREAD.load(Ordering::Relaxed),
                        overview::WM_OV_ACCEPT,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            } else if WIN_DOWN.load(Ordering::Relaxed)
                && (GetAsyncKeyState(VK_LWIN.0 as i32) < 0
                    || GetAsyncKeyState(VK_RWIN.0 as i32) < 0)
            {
                let shift = GetAsyncKeyState(VK_SHIFT.0 as i32) < 0;
                let alt = GetAsyncKeyState(VK_MENU.0 as i32) < 0;
                let ctrl = GetAsyncKeyState(VK_CONTROL.0 as i32) < 0;
                if OV_OPEN.load(Ordering::Relaxed) {
                    // Overview terbuka: tiling mati. Panah menggeser pilihan, Tab berputar,
                    // Enter / Esc memilih / membatalkan. Selain itu dibiarkan ke Windows.
                    let ov_vk = OVERVIEW_VK.load(Ordering::Relaxed);
                    return match overview_hook_action(vk, shift, alt, ctrl, ov_vk) {
                        Some((msg, wp, moves)) => {
                            if down {
                                let repeat = HELD_VK.swap(vk, Ordering::Relaxed) == vk;
                                if repeat && msg == WM_ACTION {
                                    return LRESULT(1); // toggle / keluar: sekali per tekan
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
                        None => CallNextHookEx(None, code, wparam, lparam),
                    };
                }
                // Win+Ctrl+Arrow dibiarkan lewat: itu pindah virtual desktop bawaan Windows
                let action = if let Some(i) = arrow_index(vk).filter(|_| !ctrl) {
                    // Win+Alt: tukar posisi, Win+Shift: fokus, Win: snap
                    Some(if alt {
                        i + 10
                    } else if shift {
                        i + 5
                    } else {
                        i + 1
                    })
                } else if vk == VK_RETURN.0 {
                    Some(HOTKEY_MAXIMIZE)
                } else if vk == VK_ESCAPE.0 && ctrl {
                    Some(HOTKEY_QUIT)
                } else if vk == VK_Q.0 && !shift && !alt && !ctrl {
                    Some(HOTKEY_CLOSE)
                } else if vk == VK_M.0 && !shift && !alt && !ctrl {
                    Some(HOTKEY_MINIMIZE)
                } else if vk == VK_OEM_6.0 && !shift && !alt && !ctrl {
                    Some(HOTKEY_SPLIT_RIGHT)
                } else if vk == VK_OEM_4.0 && !shift && !alt && !ctrl {
                    Some(HOTKEY_SPLIT_LEFT)
                } else if vk == VK_OEM_5.0 && !shift && !alt && !ctrl {
                    Some(HOTKEY_SPLIT_RESET)
                } else if vk != 0
                    && vk == OVERVIEW_VK.load(Ordering::Relaxed)
                    && !shift
                    && !alt
                    && !ctrl
                {
                    Some(HOTKEY_OVERVIEW)
                } else if vk != 0
                    && vk == CHEAT_VK.load(Ordering::Relaxed)
                    && !shift
                    && !alt
                    && !ctrl
                {
                    Some(HOTKEY_CHEATSHEET)
                } else if vk == VK_F.0 && alt {
                    Some(HOTKEY_FLOAT)
                } else if vk == VK_I.0 && alt {
                    Some(HOTKEY_INSPECT)
                } else if vk == VK_SPACE.0 && alt {
                    Some(HOTKEY_OVERVIEW)
                } else {
                    None
                };
                if let Some(id) = action {
                    if down {
                        let repeat = HELD_VK.swap(vk, Ordering::Relaxed) == vk;
                        if repeat && !repeat_allowed(id) {
                            return LRESULT(1); // tombol ditahan: abaikan pengulangan
                        }
                        if id == HOTKEY_OVERVIEW {
                            // optimistis: tombol berikutnya (mis. Tab kedua) harus sudah
                            // dianggap "overview terbuka" sebelum thread utama selesai membukanya
                            OV_OPEN.store(true, Ordering::Relaxed);
                            OV_MOVED.store(false, Ordering::Relaxed);
                        }
                        let _ = PostThreadMessageW(
                            MAIN_THREAD.load(Ordering::Relaxed),
                            WM_ACTION,
                            WPARAM(id as usize),
                            LPARAM(0),
                        );
                        cancel_start_menu();
                    }
                    return LRESULT(1); // telan tombolnya
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Callback event Windows. Dipanggil di thread utama lewat message loop;
/// kerjanya sengaja minimal: teruskan sebagai pesan, penyaringan di `on_event`.
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
        return; // hanya window itu sendiri, bukan elemen di dalamnya
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

    const TAB: u16 = 0x09;

    fn act(vk: u16, shift: bool, alt: bool, ctrl: bool) -> Option<(u32, usize, bool)> {
        overview_hook_action(vk, shift, alt, ctrl, TAB)
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
        // Win+Ctrl+Panah (virtual desktop bawaan) tidak disentuh
        assert_eq!(act(VK_LEFT.0, false, false, true), None);
    }

    #[test]
    fn tab_cycles_forward_and_shift_tab_backward() {
        assert_eq!(
            act(TAB, false, false, false),
            Some((overview::WM_OV_CYCLE, 1, true))
        );
        assert_eq!(
            act(TAB, true, false, false),
            Some((overview::WM_OV_CYCLE, 0, true))
        );
        assert_eq!(overview_hook_action(TAB, false, false, false, 0), None); // overview_key off
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
        assert_eq!(
            act(VK_SPACE.0, false, true, false),
            Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false))
        );
    }

    #[test]
    fn emergency_quit_still_works_and_other_wm_keys_pass_through() {
        assert_eq!(
            act(VK_ESCAPE.0, false, false, true),
            Some((WM_ACTION, HOTKEY_QUIT as usize, false))
        );
        assert_eq!(act(VK_Q.0, false, false, false), None); // tutup window: mati
        assert_eq!(act(VK_M.0, false, false, false), None); // minimize: mati
        assert_eq!(act(VK_F.0, false, true, false), None); // float: mati
    }

    #[test]
    fn only_movement_and_resize_shortcuts_repeat_when_held() {
        for id in 1..=8 {
            assert!(repeat_allowed(id), "{id}");
        }
        for id in 10..=13 {
            assert!(repeat_allowed(id), "{id}");
        }
        assert!(repeat_allowed(HOTKEY_SPLIT_RIGHT) && repeat_allowed(HOTKEY_SPLIT_LEFT));
        // sekali per tekan: tutup, minimize, float, overview, cheatsheet, keluar, dll.
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
        ] {
            assert!(!repeat_allowed(id), "{id}");
        }
    }
}
