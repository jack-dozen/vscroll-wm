//! Hide / show taskbar buttons for windows owned by other processes via ITaskbarList.
//!
//! Following LeopardWM (crates/platform_win32/src/taskbar.rs): DWM cloak is rejected
//! (E_ACCESSDENIED) for windows in other processes, and parking them off-screen does not
//! remove their buttons, so DeleteTab / AddTab are the only methods that work.
//! ITaskbarList is apartment-threaded, so a dedicated STA thread owns it and
//! receives commands over a channel. Best-effort: on failure, all functions here
//! do nothing (windows remain usable; only their taskbar buttons stay visible).
//!
//! GUESS (not verified here): how DeleteTab behaves for windows in other processes,
//! and whether Explorer adds their buttons back when a window's state changes.

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};

enum Cmd {
    /// Remove the button (only once per window until it is shown again).
    Hide(isize),
    /// Restore the button only if we hid it.
    Show(isize),
    /// Restore the button unconditionally (left over from a previous process that was force-killed).
    Restore(isize),
    /// The window has closed: forget it without AddTab (the HWND may be reused).
    Forget(isize),
    /// Restore all buttons we hid (pause).
    ShowAll,
}

struct Worker {
    tx: Sender<Cmd>,
    thread: JoinHandle<()>,
}

static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// Start the COM thread. Call once at startup.
pub fn init() {
    let (tx, rx) = mpsc::channel();
    match std::thread::Builder::new()
        .name("taskbar".into())
        .spawn(move || run(rx))
    {
        Ok(thread) => {
            if let Ok(mut g) = WORKER.lock() {
                *g = Some(Worker { tx, thread });
            }
        }
        Err(e) => crate::log_warn!("failed to spawn taskbar thread: {e}"),
    }
}

fn send(cmd: Cmd) {
    if let Ok(g) = WORKER.lock() {
        if let Some(w) = g.as_ref() {
            let _ = w.tx.send(cmd);
        }
    }
}

pub fn hide(h: HWND) {
    send(Cmd::Hide(h.0 as isize));
}

pub fn show(h: HWND) {
    send(Cmd::Show(h.0 as isize));
}

pub fn restore(h: HWND) {
    send(Cmd::Restore(h.0 as isize));
}

pub fn forget(h: HWND) {
    send(Cmd::Forget(h.0 as isize));
}

pub fn show_all() {
    send(Cmd::ShowAll);
}

/// Stop the thread; first restore all buttons we hid. Wait up to ~500 ms.
/// Safe to call from a panic hook (try_lock) and multiple times.
pub fn shutdown() {
    let worker = WORKER.try_lock().ok().and_then(|mut g| g.take());
    if let Some(w) = worker {
        drop(w.tx); // Closed channel -> run() restores the buttons and exits.
        for _ in 0..50 {
            if w.thread.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn hwnd(key: isize) -> HWND {
    HWND(key as *mut c_void)
}

fn run(rx: Receiver<Cmd>) {
    unsafe {
        // STA: ITaskbarList apartment-threaded
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ITaskbarList = match CoCreateInstance(&TaskbarList, None, CLSCTX_ALL) {
            Ok(l) => l,
            Err(e) => {
                crate::log_warn!("CoCreateInstance(TaskbarList) failed: {e}");
                CoUninitialize();
                return;
            }
        };
        if let Err(e) = list.HrInit() {
            crate::log_warn!("ITaskbarList::HrInit failed: {e}");
            CoUninitialize();
            return;
        }
        let mut hidden: HashSet<isize> = HashSet::new();
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Hide(k) => {
                    if hidden.insert(k) {
                        let _ = list.DeleteTab(hwnd(k));
                    }
                }
                Cmd::Show(k) => {
                    if hidden.remove(&k) {
                        let _ = list.AddTab(hwnd(k));
                    }
                }
                Cmd::Restore(k) => {
                    hidden.remove(&k);
                    let _ = list.AddTab(hwnd(k));
                }
                Cmd::Forget(k) => {
                    hidden.remove(&k);
                }
                Cmd::ShowAll => {
                    for k in hidden.drain() {
                        let _ = list.AddTab(hwnd(k));
                    }
                }
            }
        }
        // Channel closed: restore all buttons that are still hidden.
        for k in hidden.drain() {
            let _ = list.AddTab(hwnd(k));
        }
        drop(list);
        CoUninitialize();
    }
}
