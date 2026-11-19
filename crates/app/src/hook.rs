//! Hook keyboard tingkat rendah dan event hook Windows, beserta tabel aksi tombolnya.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
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

/// Tombol untuk berputar antar window saat overview terbuka dan Win ditahan (kode
/// virtual-key; 0 = mati). Diturunkan dari ikatan `overview` (lihat bindings.rs).
pub(crate) static OVERVIEW_VK: AtomicU16 = AtomicU16::new(0x09);

/// Tombol pintasan yang sedang ditahan, untuk mengenali event berulang (auto-repeat).
pub(crate) static HELD_VK: AtomicU16 = AtomicU16::new(0);

/// Overview sedang terbuka: tiling dimatikan, Win+tombol hanya mengendalikan overview.
pub(crate) static OV_OPEN: AtomicBool = AtomicBool::new(false);

/// Daftar window snap layouts sedang tampil: Esc membatalkannya (tombolnya ditelan).
pub(crate) static SNAP_OPEN: AtomicBool = AtomicBool::new(false);

/// Pilihan sudah digeser sejak overview dibuka. Kalau Win dilepas dalam keadaan ini,
/// overview langsung loncat ke window yang dipilih (gaya Alt+Tab).
pub(crate) static OV_MOVED: AtomicBool = AtomicBool::new(false);

/// Mode angkat-dan-jatuhkan (`pick`) sedang aktif: panah menggeser marker, Enter / Space
/// menjatuhkan, Esc membatalkan.
pub(crate) static PICK_OPEN: AtomicBool = AtomicBool::new(false);

/// Mode bidik `join` aktif (disimpan untuk jaga-jaga kalau mode tahan-modifier dipakai
/// lagi): melepas salah satu modifier di `JOIN_MODS` mengirim `WM_JOIN_COMMIT`; Esc batal.
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

/// Aksi tombol Win+... saat overview terbuka: (pesan, wParam, menggeser pilihan?).
/// `bound` = id hotkey yang terikat pada kombinasi ini menurut konfigurasi (bindings.rs).
/// None = bukan tombol overview; dibiarkan lewat ke Windows (tiling tidak ikut jalan).
pub(crate) fn overview_hook_action(
    vk: u16,
    shift: bool,
    alt: bool,
    ctrl: bool,
    overview_vk: u16,
    bound: Option<i32>,
) -> Option<(u32, usize, bool)> {
    if vk == VK_ESCAPE.0 && ctrl {
        return Some((WM_ACTION, HOTKEY_QUIT as usize, false)); // jalan keluar darurat
    }
    // Pintasan workspace (sesuai ikatan sekarang): sama seperti di luar overview,
    // tapi menggeser kursor
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
        return Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false)); // toggle tutup
    }
    let _ = (shift, alt, overview_vk);
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

/// Modifier selain Win yang sedang ditahan (bitmask SHIFT / ALT / CTRL dari bindings.rs).
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

/// Kirim aksi `id` ke thread utama dan telan tombolnya. Pengulangan karena tombol
/// ditahan diabaikan kecuali aksinya memang boleh berulang.
unsafe fn fire(vk: u16, id: i32, down: bool, _mods: u8) -> LRESULT {
    if down {
        let repeat = HELD_VK.swap(vk, Ordering::Relaxed) == vk;
        if repeat && !bindings::repeat_allowed(id) {
            return LRESULT(1); // tombol ditahan: abaikan pengulangan
        }
        if id == HOTKEY_OVERVIEW && !OV_OPEN.load(Ordering::Relaxed) {
            // optimistis: tombol berikutnya (mis. Tab kedua) harus sudah
            // dianggap "overview terbuka" sebelum thread utama selesai membukanya
            OV_OPEN.store(true, Ordering::Relaxed);
            OV_MOVED.store(false, Ordering::Relaxed);
        }
        // Quick join langsung eksekusi; untuk kembali ke mode tahan-modifier:
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
            // Mode angkat-dan-jatuhkan (`pick`): panah geser marker, Enter / Space
            // jatuhkan, Esc / tombol pick membatalkan. Win+Ctrl+Esc tetap keluar darurat.
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
            // Mode bidik join: Esc membatalkan (kecuali Win+Ctrl+Esc keluar darurat);
            // melepas salah satu modifier yang ditahan langsung mengeksekusi join.
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
            // Esc tanpa modifier menutup daftar window snap layouts
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
                // aksi yang terikat pada kombinasi ini (tombol terkunci dulu, lalu konfigurasi)
                let bound = bindings::lookup_active(WIN | held, vk);
                if OV_OPEN.load(Ordering::Relaxed) {
                    // Overview terbuka: tiling mati. Panah menggeser pilihan, tombol overview
                    // berputar, Enter / Esc memilih / membatalkan. Selain itu dibiarkan ke Windows.
                    let ov_vk = OVERVIEW_VK.load(Ordering::Relaxed);
                    return match overview_hook_action(vk, shift, alt, ctrl, ov_vk, bound) {
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
                        None if bound.is_some_and(bindings::is_workspace_id) => {
                            // desktop virtual Windows dimatikan: jangan diteruskan
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
                // Pintasan tanpa Win (mis. Alt+H atau Ctrl+Alt+O): hanya kalau ada modifier
                // yang ditahan. Saat overview terbuka hanya tombol overview-nya sendiri
                // yang berlaku (menutupnya); sisanya untuk jendela overview.
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
    use crate::bindings::Bindings;

    const TAB: u16 = 0x09;

    /// Kombinasi Win + modifier yang diberikan, lewat ikatan bawaan.
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
        // Win+Ctrl+Kiri/Kanan / angka = lompat workspace (kursor)
        assert_eq!(act(VK_LEFT.0, false, false, true), Some((overview::WM_OV_WS, 0, true)));
        assert_eq!(act(VK_RIGHT.0, false, false, true), Some((overview::WM_OV_WS, 1, true)));
        assert_eq!(act(0x31, false, false, true), Some((overview::WM_OV_WS, 2, true)));
        assert_eq!(act(0x39, false, false, true), Some((overview::WM_OV_WS, 10, true)));
        assert_eq!(act(VK_LEFT.0, true, false, true), None); // kirim window dari overview: belum ada
    }

    #[test]
    fn workspace_keys_in_the_overview_follow_custom_bindings() {
        // workspace_next dipindah ke Win+Ctrl+L: di overview ikut menggeser kursor
        let mut b = Bindings::defaults();
        let next = bindings::find_action("workspace_next").unwrap();
        b.set(next, vec![bindings::Chord::new(WIN | CTRL, 0x4C)]);
        let bound = b.lookup(WIN | CTRL, 0x4C);
        assert_eq!(
            overview_hook_action(0x4C, false, false, true, TAB, bound),
            Some((overview::WM_OV_WS, 1, true))
        );
        // tombol lamanya tidak lagi berarti workspace berikutnya
        let old = b.lookup(WIN | CTRL, VK_RIGHT.0);
        assert_eq!(overview_hook_action(VK_RIGHT.0, false, false, true, TAB, old), None);
    }

    #[test]
    fn overview_key_toggles_close_instead_of_cycling_while_holding_win() {
        assert_eq!(
            act(TAB, false, false, false),
            Some((WM_ACTION, HOTKEY_OVERVIEW as usize, false))
        );
        // Win+Shift+Tab bukan shortcut toggle overview
        assert_eq!(act(TAB, true, false, false), None);
        // overview_key off: tidak ada aksi Win+Tab
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
        // overview dengan custom chord tetap bisa menutup overview
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
        assert_eq!(act(VK_Q.0, false, false, false), None); // tutup window: mati
        assert_eq!(act(VK_M.0, false, false, false), None); // minimize: mati
        assert_eq!(act(VK_F.0, false, true, false), None); // float: mati
    }
}
