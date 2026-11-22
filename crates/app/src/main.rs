//! v1: single monitor. Track windows via Windows events (SetWinEventHook) plus
//! slow polling every second as a safety net.
//!
//! Win+Arrow             -> snap the active window (hook swallows built-in Win+Arrow)
//! Win+Shift+Arrow       -> move focus
//! Win+Enter             -> restore the active window to full size
//! Win+Alt+Arrow         -> swap positions with a neighboring window
//! Win+Alt+Shift+Arrow   -> quickly join a neighboring window (split / stack)
//! Win+Alt+Space         -> lift a window and choose a destination (arrows + overlay marker)
//! Win+Alt+F             -> float / tile the active window
//! Win+Alt+I             -> print active window info (class / exe / title) to the console
//! Win+Tab               -> overview: strip map (key: overview_key)
//! Win+] / Win+[         -> widen / narrow the active window's column (move divider)
//! Win+\                 -> reset the column divider to 50/50
//! Win+/                 -> keyboard shortcuts cheatsheet (key: cheatsheet_key)
//! Win+Q                 -> close the active window
//! Win+M                 -> minimize the active window
//! Win+Ctrl+Left/Right   -> previous / next workspace (+Shift: send active window)
//! Win+Ctrl+1..9         -> switch to workspace n (+Shift: send active window there)
//! Win+Ctrl+Esc          -> exit and restore all windows (also Ctrl+C)
//! Ctrl+Alt+... (fallback, same actions)
//!
//! Original bindings:
//! Ctrl+Alt+Arrow        -> snap the active window
//! Ctrl+Alt+Shift+Arrow  -> move focus
//! Ctrl+Alt+Enter        -> restore the active window to full size (width + height)
//! Exit with Ctrl+C in the console.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bindings;
mod cheatsheet;
mod config;
mod consts;
mod hook;
mod layout_state;
mod log;
mod osd;
mod overview;
mod pick_overlay;
mod rules;
mod safety;
mod snap_menu;
mod snap_slots;
mod taskbar;
mod theme;
mod tray;
mod winutil;

use std::ffi::c_void;
use std::sync::atomic::Ordering;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::App;
use crate::consts::{
    DIRS, HOTKEY_FLOAT, HOTKEY_INSPECT, HOTKEY_MAXIMIZE, HOTKEY_OVERVIEW, HOTKEY_QUIT, RECHECK_MS,
    WM_ACTION, WM_JOIN_CANCEL, WM_JOIN_COMMIT, WM_PICK_ACCEPT, WM_PICK_CANCEL, WM_PICK_MOVE,
    WM_SNAP_CANCEL, WM_SNAP_PICK, WM_WINEVENT,
};
use crate::hook::{win_event_proc, KeyboardHookThread, MAIN_THREAD};
use crate::winutil::{single_instance, work_area};

fn main() {
    unsafe {
        if !single_instance() {
            #[cfg(debug_assertions)]
            eprintln!("vscroll-wm is already running.");
            return;
        }
        log::init();
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut app = App::new();
        let report = config::load();
        app.apply_config(&report.cfg);

        let mut failed_hotkeys: Vec<String> = Vec::new();
        for (i, (vk, _)) in DIRS.iter().enumerate() {
            let base = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
            if RegisterHotKey(None, i as i32 + 1, base, vk.0 as u32).is_err() {
                let name = format!("Ctrl+Alt+{vk:?}");
                log_warn!("failed to register {name} (in use by another app?)");
                failed_hotkeys.push(name);
            }
            if RegisterHotKey(None, i as i32 + 5, base | MOD_SHIFT, vk.0 as u32).is_err() {
                let name = format!("Ctrl+Alt+Shift+{vk:?}");
                log_warn!("failed to register {name} (in use by another app?)");
                failed_hotkeys.push(name);
            }
        }
        if RegisterHotKey(
            None,
            HOTKEY_MAXIMIZE,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_RETURN.0 as u32,
        )
        .is_err()
        {
            log_warn!("failed to register Ctrl+Alt+Enter (in use by another app?)");
            failed_hotkeys.push("Ctrl+Alt+Enter".into());
        }
        if RegisterHotKey(
            None,
            HOTKEY_QUIT,
            MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
            VK_Q.0 as u32,
        )
        .is_err()
        {
            log_warn!("failed to register Ctrl+Alt+Shift+Q (in use by another app?)");
            failed_hotkeys.push("Ctrl+Alt+Shift+Q".into());
        }
        if RegisterHotKey(
            None,
            HOTKEY_FLOAT,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_F.0 as u32,
        )
        .is_err()
        {
            log_warn!("failed to register Ctrl+Alt+F (in use by another app?)");
            failed_hotkeys.push("Ctrl+Alt+F".into());
        }
        if RegisterHotKey(
            None,
            HOTKEY_INSPECT,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_I.0 as u32,
        )
        .is_err()
        {
            log_warn!("failed to register Ctrl+Alt+I (in use by another app?)");
            failed_hotkeys.push("Ctrl+Alt+I".into());
        }
        MAIN_THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
        safety::install(GetCurrentThreadId());
        theme::init();
        taskbar::init();
        if !overview::init(GetCurrentThreadId()) {
            log_warn!("failed to create overview window");
        }
        if !cheatsheet::init() {
            log_warn!("failed to create cheatsheet window");
        }
        if !osd::init() {
            log_warn!("failed to create workspace OSD window");
        }
        if !pick_overlay::init() {
            log_warn!("failed to create pick overlay window");
        }
        let dpi_scale = GetDpiForSystem() as f32 / 96.0;
        if !snap_menu::init(dpi_scale) {
            log_warn!("failed to create snap layouts menu window");
        }
        if !snap_slots::init(GetCurrentThreadId(), dpi_scale) {
            log_warn!("failed to create snap layouts window list");
        }
        if RegisterHotKey(
            None,
            HOTKEY_OVERVIEW,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_SPACE.0 as u32,
        )
        .is_err()
        {
            log_warn!("failed to register Ctrl+Alt+Space (in use by another app?)");
            failed_hotkeys.push("Ctrl+Alt+Space".into());
        }
        if !tray::create(GetCurrentThreadId()) {
            log_warn!("failed to create tray icon");
        }
        let mut kb_hook_status = String::from("ok");
        let hook = match KeyboardHookThread::spawn() {
            Ok(h) => Some(h),
            Err(e) => {
                log_warn!("failed to install keyboard hook (Win+Arrow inactive): {e}");
                kb_hook_status = format!("failed ({e})");
                None
            }
        };
        let mut event_hooks: Vec<HWINEVENTHOOK> = Vec::new();
        for (lo, hi) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ] {
            let h = SetWinEventHook(
                lo,
                hi,
                None,
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if h.is_invalid() {
                log_warn!("failed to install event hook {lo:#x}..{hi:#x}");
            } else {
                event_hooks.push(h);
            }
        }
        // With event hooks, polling is only a safety net (e.g. for windows titled
        // shortly after appearing). Without hooks, use frequent polling instead.
        let poll_ms = if event_hooks.is_empty() { 300 } else { 1000 };
        SetTimer(None, 1, poll_ms, None);

        let cfg_str = report
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<default>".into());
        let state_str = layout_state::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".into());
        let hk_str = if failed_hotkeys.is_empty() {
            "none".into()
        } else {
            failed_hotkeys.join(", ")
        };
        log_info!(
            "startup vscroll-wm v{} | elevated={} | config={} | state={} | log={} | kb_hook={} | event_hooks={}/5 (poll {}ms) | failed_hotkeys={}",
            env!("CARGO_PKG_VERSION"),
            log::is_elevated(),
            cfg_str,
            state_str,
            log::log_path().display(),
            kb_hook_status,
            event_hooks.len(),
            poll_ms,
            hk_str
        );
        app.restore_layout(); // Layout from the previous session, for windows that are still open.
        let changed = app.sync();
        app.restore_all_tabs(); // Taskbar buttons left behind by a force-killed previous process.
        if changed {
            app.apply();
        }

        app::show_config_problems(&report); // Show config typos immediately.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                safety::WM_QUIT_REQUEST => break,
                WM_HOTKEY | WM_ACTION => {
                    let id = msg.wParam.0 as i32;
                    if id == HOTKEY_QUIT {
                        break;
                    }
                    app.on_hotkey(id);
                }
                tray::WM_TRAY_PAUSE => app.toggle_pause(),
                tray::WM_TRAY_RELOAD => app.reload_config(),
                tray::WM_TRAY_HELP => cheatsheet::toggle(work_area(), &app.cfg),
                tray::WM_TRAY_SESSION_END => match msg.wParam.0 {
                    0 => {
                        app.save_layout();
                        app.unstick_all();
                        app.parked.clear();
                        app.leaving.clear();
                        app.border_applied = None;
                    }
                    1 => {
                        app.save_layout();
                        app.unstick_all();
                    }
                    2 => {
                        if app.sync() {
                            app.apply();
                        }
                    }
                    _ => {}
                },
                overview::WM_OV_MOVE => app.overview_move(msg.wParam.0),
                overview::WM_OV_CYCLE => app.overview_cycle(msg.wParam.0 != 0),
                overview::WM_OV_WS => app.overview_workspace(msg.wParam.0),
                overview::WM_OV_ACCEPT => app.close_overview(true),
                overview::WM_OV_CANCEL => app.close_overview(false),
                overview::WM_OV_PICK => app.overview_pick(msg.wParam.0 as u64),
                overview::WM_OV_DROP => app.overview_drop(),
                overview::WM_OV_ZOOM => app.overview_zoom(),
                WM_SNAP_PICK => app.snap_assist_pick(msg.wParam.0 as u64, msg.lParam.0 as u64),
                WM_SNAP_CANCEL => app.snap_assist_end(true),
                WM_JOIN_COMMIT => app.join_commit(),
                WM_JOIN_CANCEL => app.join_cancel(),
                WM_PICK_MOVE => app.pick_move(msg.wParam.0),
                WM_PICK_ACCEPT => app.pick_accept(),
                WM_PICK_CANCEL => app.pick_cancel(),
                WM_WINEVENT => {
                    if !tray::PAUSED.load(Ordering::Relaxed) {
                        app.on_event(msg.wParam.0 as u32, HWND(msg.lParam.0 as *mut c_void));
                    }
                }
                WM_TIMER if !msg.hwnd.0.is_null() => {
                    // Timer owned by a window (workspace OSD): forward it to that window's procedure.
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                WM_TIMER => {
                    let id = msg.wParam.0;
                    let paused = tray::PAUSED.load(Ordering::Relaxed);
                    if app.anim_timer != 0 && id == app.anim_timer {
                        app.tick_anim();
                    } else if app.snap_timer != 0 && id == app.snap_timer {
                        app.tick_snap_drag();
                    } else if app.sync_timer != 0 && id == app.sync_timer {
                        app.stop_sync_timer();
                        if !paused {
                            if app.sync() {
                                app.apply();
                            }
                            if std::mem::take(&mut app.recheck) {
                                app.schedule_sync(RECHECK_MS);
                            }
                        }
                    } else if !paused {
                        if app.sync() {
                            app.apply(); // Safety net.
                        }
                        app.save_layout(); // Write only if the layout changed.
                    }
                }
                _ => {
                    // Message for the tray window (menu, etc.).
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        // Exit: save the final layout, then restore all windows to their original positions.
        app.save_layout();
        app.stop_anim_timer();
        app.stop_sync_timer();
        for h in event_hooks {
            let _ = UnhookWinEvent(h);
        }
        if let Some(h) = hook {
            h.stop();
        }
        tray::remove();
        app.unstick_all();
        safety::restore_all();
        taskbar::shutdown();
        theme::shutdown();
        safety::EXIT_DONE.store(true, Ordering::Relaxed);
        log_info!("all windows restored; exiting");
    }
}
