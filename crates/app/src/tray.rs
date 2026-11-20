//! Tray icon (right-click menu: Pause / Run at login / Quit) and
//! autostart settings via the HKCU\...\Run registry key.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Message from the tray icon to our hidden window.
const WM_TRAY: u32 = WM_APP + 4;
/// Sent to the main thread: toggle the paused state.
pub const WM_TRAY_PAUSE: u32 = WM_APP + 5;
/// Sent to the main thread: reload the configuration file.
pub const WM_TRAY_RELOAD: u32 = WM_APP + 6;
/// Sent to the main thread: open / close the cheatsheet.
pub const WM_TRAY_HELP: u32 = WM_APP + 12;
/// Sent to the main thread on logoff / shutdown (`wParam`: 0 = query, 1 = ending, 2 = cancelled).
pub const WM_TRAY_SESSION_END: u32 = WM_APP + 27;

/// True = the app is paused: windows are restored to their original positions
/// and all hotkeys are passed through to Windows as usual.
pub static PAUSED: AtomicBool = AtomicBool::new(false);

static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static SESSION_WAS_PAUSED: AtomicBool = AtomicBool::new(false);

const ID_PAUSE: usize = 1;
const ID_AUTOSTART: usize = 2;
const ID_QUIT: usize = 3;
const ID_RELOAD: usize = 4;
const ID_HELP: usize = 5;

const RUN_SUBKEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("vscroll-wm");

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- autostart

pub fn autostart_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

pub fn set_autostart(on: bool) -> bool {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else {
                return false;
            };
            let data = wide(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_SUBKEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(data.as_ptr() as *const c_void),
                (data.len() * std::mem::size_of::<u16>()) as u32,
            )
            .is_ok()
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_SUBKEY, RUN_VALUE).is_ok()
        }
    }
}

// --------------------------------------------------------------------- icons

/// A 32x32 Windows 11 Fluent-style icon: a rounded background and three rounded
/// panels using the active Windows accent color (anti-aliased).
unsafe fn make_icon() -> Option<HICON> {
    const N: usize = 32;
    let pal = crate::theme::Palette::resolve(crate::config::OverviewColors::default());
    let (ar, ag, ab) = (
        (pal.accent & 0xFF) as f32,
        ((pal.accent >> 8) & 0xFF) as f32,
        ((pal.accent >> 16) & 0xFF) as f32,
    );
    let (br, bg, bb) = (
        (pal.surface & 0xFF) as f32,
        ((pal.surface >> 8) & 0xFF) as f32,
        ((pal.surface >> 16) & 0xFF) as f32,
    );

    let rrect_cov = |px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| -> f32 {
        let cx = (x0 + x1) * 0.5;
        let cy = (y0 + y1) * 0.5;
        let hx = (x1 - x0) * 0.5 - rad;
        let hy = (y1 - y0) * 0.5 - rad;
        let qx = (px - cx).abs() - hx;
        let qy = (py - cy).abs() - hy;
        let dist = (qx.max(0.0).hypot(qy.max(0.0))) + qx.max(qy).min(0.0) - rad;
        (0.5 - dist).clamp(0.0, 1.0)
    };

    let mut xor = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let bg_cov = rrect_cov(px, py, 1.0, 1.0, 31.0, 31.0, 6.0);
            let p1 = rrect_cov(px, py, 4.5, 4.5, 14.5, 14.5, 2.5);
            let p2 = rrect_cov(px, py, 4.5, 17.5, 14.5, 27.5, 2.5);
            let p3 = rrect_cov(px, py, 17.5, 4.5, 27.5, 27.5, 2.5);
            let pane_cov = p1.max(p2).max(p3);

            let r = br + (ar - br) * pane_cov;
            let g = bg + (ag - bg) * pane_cov;
            let b = bb + (ab - bb) * pane_cov;
            let a = (bg_cov * 255.0).round() as u8;

            let i = (y * N + x) * 4;
            xor[i] = b.round() as u8;
            xor[i + 1] = g.round() as u8;
            xor[i + 2] = r.round() as u8;
            xor[i + 3] = a;
        }
    }
    let and = vec![0u8; N * N / 8];
    CreateIcon(None, N as i32, N as i32, 1, 32, and.as_ptr(), xor.as_ptr()).ok()
}

unsafe fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut n: NOTIFYICONDATAW = std::mem::zeroed();
    n.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n
}

fn copy_tip(n: &mut NOTIFYICONDATAW, text: &str) {
    let w: Vec<u16> = text.encode_utf16().take(127).collect();
    n.szTip = [0; 128];
    n.szTip[..w.len()].copy_from_slice(&w);
}

unsafe fn add_icon(hwnd: HWND) {
    let icon = HICON(ICON.load(Ordering::Relaxed) as *mut _);
    let mut n = nid(hwnd);
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    n.uCallbackMessage = WM_TRAY;
    n.hIcon = icon;
    copy_tip(&mut n, "vscroll-wm");
    let _ = Shell_NotifyIconW(NIM_ADD, &n);
}

pub unsafe fn set_tip(text: &str) {
    let hwnd = HWND(TRAY_HWND.load(Ordering::Relaxed) as *mut _);
    let mut n = nid(hwnd);
    n.uFlags = NIF_TIP;
    copy_tip(&mut n, text);
    let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
}

pub unsafe fn remove() {
    let hwnd = HWND(TRAY_HWND.load(Ordering::Relaxed) as *mut _);
    if !hwnd.0.is_null() {
        let n = nid(hwnd);
        let _ = Shell_NotifyIconW(NIM_DELETE, &n);
    }
}

// --------------------------------------------------------------------- menu

unsafe fn post_main(msg: u32) {
    let _ = PostThreadMessageW(
        MAIN_THREAD.load(Ordering::Relaxed),
        msg,
        WPARAM(0),
        LPARAM(0),
    );
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let paused = PAUSED.load(Ordering::Relaxed);
    let pause_txt = wide(if paused { "Resume" } else { "Pause" });
    let reload_txt = wide("Reload configuration");
    let help_txt = wide("Keyboard shortcuts");
    let auto_txt = wide("Run at login");
    let quit_txt = wide("Quit (restore windows)");

    let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE, PCWSTR(pause_txt.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, ID_RELOAD, PCWSTR(reload_txt.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, ID_HELP, PCWSTR(help_txt.as_ptr()));
    let auto_flags = if autostart_enabled() {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING
    };
    let _ = AppendMenuW(menu, auto_flags, ID_AUTOSTART, PCWSTR(auto_txt.as_ptr()));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(quit_txt.as_ptr()));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Required to close the menu when the user clicks elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        Some(0),
        hwnd,
        None,
    );
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);

    match cmd.0 as usize {
        ID_PAUSE => post_main(WM_TRAY_PAUSE),
        ID_RELOAD => post_main(WM_TRAY_RELOAD),
        ID_HELP => post_main(WM_TRAY_HELP),
        ID_AUTOSTART => {
            let _ = set_autostart(!autostart_enabled());
        }
        ID_QUIT => post_main(crate::safety::WM_QUIT_REQUEST),
        _ => {}
    }
}

unsafe extern "system" fn tray_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_TRAY {
        let ev = lparam.0 as u32;
        if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP || ev == WM_CONTEXTMENU {
            show_menu(hwnd);
        }
        return LRESULT(0);
    }
    let tb = TASKBAR_CREATED.load(Ordering::Relaxed);
    if tb != 0 && msg == tb {
        add_icon(hwnd); // Explorer restarted: add the icon again.
        return LRESULT(0);
    }
    if msg == WM_QUERYENDSESSION {
        // Pulihkan semua window secara sinkron sebelum app lain menyimpan posisinya.
        let was_paused = PAUSED.swap(true, Ordering::Relaxed);
        SESSION_WAS_PAUSED.store(was_paused, Ordering::Relaxed);
        crate::safety::restore_all();
        crate::taskbar::show_all();
        let _ = PostThreadMessageW(
            MAIN_THREAD.load(Ordering::Relaxed),
            WM_TRAY_SESSION_END,
            WPARAM(0),
            LPARAM(0),
        );
        return LRESULT(1); // TRUE: izinkan logoff / shutdown
    }
    if msg == WM_ENDSESSION {
        if wparam.0 != 0 {
            crate::safety::restore_all();
            crate::taskbar::shutdown();
            let _ = PostThreadMessageW(
                MAIN_THREAD.load(Ordering::Relaxed),
                WM_TRAY_SESSION_END,
                WPARAM(1),
                LPARAM(0),
            );
        } else if !SESSION_WAS_PAUSED.load(Ordering::Relaxed) {
            // Shutdown dibatalkan: aktifkan kembali tiling.
            PAUSED.store(false, Ordering::Relaxed);
            let _ = PostThreadMessageW(
                MAIN_THREAD.load(Ordering::Relaxed),
                WM_TRAY_SESSION_END,
                WPARAM(2),
                LPARAM(0),
            );
        }
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Create a hidden window to receive tray messages and add its icon.
/// Must be called from the thread running the message loop.
pub unsafe fn create(main_thread: u32) -> bool {
    MAIN_THREAD.store(main_thread, Ordering::Relaxed);
    let Ok(hmod) = GetModuleHandleW(None) else {
        return false;
    };
    let hinst = HINSTANCE(hmod.0);
    let class = w!("VScrollWmTray");

    let wc = WNDCLASSW {
        lpfnWndProc: Some(tray_proc),
        hInstance: hinst,
        lpszClassName: class,
        ..Default::default()
    };
    if RegisterClassW(&wc) == 0 {
        return false;
    }
    // Use a top-level window (not a message-only window) to receive the "TaskbarCreated" broadcast.
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class,
        w!("vscroll-wm"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        Some(hinst),
        None,
    ) else {
        return false;
    };
    TRAY_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    TASKBAR_CREATED.store(
        RegisterWindowMessageW(w!("TaskbarCreated")),
        Ordering::Relaxed,
    );

    let icon = make_icon().unwrap_or_else(|| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
    ICON.store(icon.0 as isize, Ordering::Relaxed);
    add_icon(hwnd);
    true
}
