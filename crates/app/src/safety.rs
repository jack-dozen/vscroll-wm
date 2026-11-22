//! Window recovery: restore all windows to their original positions when the app
//! exits (Ctrl+C, emergency hotkey, console closed, or panic), so no window is
//! left off-screen.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use layout::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Sent to the main thread by the Ctrl+C / console-close handler.
pub const WM_QUIT_REQUEST: u32 = WM_APP + 2;

/// Set to true by the main thread after all windows have been restored.
pub static EXIT_DONE: AtomicBool = AtomicBool::new(false);

/// DWM value for the "default border" (DWMWA_COLOR_DEFAULT).
pub const DWM_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

/// True after the app has colored any borders; during recovery, restore all window
/// borders to their defaults (including after a crash).
pub static BORDER_TOUCHED: AtomicBool = AtomicBool::new(false);

static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
struct Saved {
    hwnd: usize,
    rect: RECT,
    zoomed: bool,
}

/// Original position of each window we have managed. Global so it can be used
/// by the panic hook and console handler (other threads).
static SAVED: Mutex<Vec<Saved>> = Mutex::new(Vec::new());

fn hwnd_of(key: usize) -> HWND {
    HWND(key as *mut c_void)
}

pub unsafe fn alive(h: HWND) -> bool {
    let mut r = RECT::default();
    GetWindowRect(h, &mut r).is_ok()
}

/// If the rect is almost entirely outside `work` (e.g. a window left off-screen
/// after a crash), move it to the center of the work area. Otherwise, leave it.
fn rescue(r: RECT, work: Rect) -> RECT {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let vis_w = (r.right.min(work.x + work.w) - r.left.max(work.x)).max(0);
    let vis_h = (r.bottom.min(work.y + work.h) - r.top.max(work.y)).max(0);
    if vis_w >= w.min(300) && vis_h >= h.min(200) {
        return r;
    }
    let (nw, nh) = (w.min(work.w), h.min(work.h));
    let x = work.x + (work.w - nw) / 2;
    let y = work.y + (work.h - nh) / 2;
    RECT {
        left: x,
        top: y,
        right: x + nw,
        bottom: y + nh,
    }
}

/// Save the window's original position. Only once per window, so minimizing and
/// restoring it does not overwrite its original position.
pub unsafe fn remember(h: HWND, work: Rect) {
    let key = h.0 as usize;
    let Ok(mut list) = SAVED.lock() else { return };
    if list.iter().any(|s| s.hwnd == key) {
        return;
    }
    let mut r = RECT::default();
    if GetWindowRect(h, &mut r).is_err() {
        return;
    }
    list.push(Saved {
        hwnd: key,
        rect: rescue(r, work),
        zoomed: IsZoomed(h).as_bool(),
    });
}

/// Forget a window that has closed.
pub fn forget(h: HWND) {
    if let Ok(mut list) = SAVED.lock() {
        let key = h.0 as usize;
        list.retain(|s| s.hwnd != key);
    }
}

/// Restore all windows to their original positions. Safe to call from any thread,
/// including a panic hook (uses try_lock to avoid deadlocks).
pub unsafe fn restore_all() {
    let list: Vec<Saved> = match SAVED.try_lock() {
        Ok(g) => g.clone(),
        Err(_) => return,
    };
    let touched = BORDER_TOUCHED.load(Ordering::Relaxed);
    for s in list {
        let h = hwnd_of(s.hwnd);
        if !alive(h) || IsHungAppWindow(h).as_bool() {
            continue;
        }
        if touched {
            let color = DWM_COLOR_DEFAULT;
            let _ = DwmSetWindowAttribute(
                h,
                DWMWA_BORDER_COLOR,
                &color as *const u32 as *const c_void,
                4,
            );
        }
        if IsIconic(h).as_bool() {
            continue;
        }
        if s.zoomed {
            let _ = ShowWindow(h, SW_MAXIMIZE);
            continue;
        }
        if IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetWindowPos(
            h,
            None,
            s.rect.left,
            s.rect.top,
            s.rect.right - s.rect.left,
            s.rect.bottom - s.rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Called by Windows on a separate thread for Ctrl+C / Ctrl+Break / console
/// close / logoff / shutdown. Ask the main thread to exit cleanly, then wait
/// (up to ~4 seconds) for recovery to finish before terminating the process.
unsafe extern "system" fn console_handler(_ctrl: u32) -> BOOL {
    let _ = PostThreadMessageW(
        MAIN_THREAD.load(Ordering::Relaxed),
        WM_QUIT_REQUEST,
        WPARAM(0),
        LPARAM(0),
    );
    for _ in 0..80 {
        if EXIT_DONE.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    BOOL(1)
}

/// Install safeguards: console handler and panic hook.
pub unsafe fn install(main_thread: u32) {
    MAIN_THREAD.store(main_thread, Ordering::Relaxed);
    if let Err(e) = SetConsoleCtrlHandler(Some(console_handler), true) {
        crate::log_warn!("failed to install Ctrl+C handler: {e}");
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::log::write_crash_report(info);
        unsafe {
            restore_all();
            crate::taskbar::shutdown(); // Restore the taskbar buttons we hid.
        }
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    fn rc(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    #[test]
    fn visible_window_is_kept() {
        let r = rc(100, 100, 900, 700);
        let out = rescue(r, WORK);
        assert_eq!(
            (out.left, out.top, out.right, out.bottom),
            (100, 100, 900, 700)
        );
    }

    #[test]
    fn parked_offscreen_window_is_centered_on_work_area() {
        let out = rescue(rc(100, -1080, 900, -380), WORK); // 800x700 off-screen above.
        assert_eq!((out.right - out.left, out.bottom - out.top), (800, 700));
        assert_eq!(out.left, 560);
        assert_eq!(out.top, 190);
    }

    #[test]
    fn oversized_offscreen_window_is_clamped_to_work_area() {
        let out = rescue(rc(0, 5000, 3000, 7000), WORK);
        assert_eq!((out.right - out.left, out.bottom - out.top), (1920, 1080));
    }
}
