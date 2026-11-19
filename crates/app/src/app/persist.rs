//! Persist layouts: save workspace layouts to a file and restore them when
//! vscroll-wm is restarted while its windows are still open (see layout_state.rs).
//!
//! Saved automatically by the poll timer (each change is saved within about one
//! second) and again on exit. Disabled via `remember_layout = off`.

use std::collections::HashMap;

use layout::WindowId;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

use super::App;
use crate::winutil::{collect, wid};
use crate::{layout_state, rules};

/// Window identity for matching saved entries: `exe|class` (lowercase).
unsafe fn identity(h: HWND) -> String {
    format!(
        "{}|{}",
        rules::exe_name(h).unwrap_or_default().to_lowercase(),
        rules::class_name(h)
    )
}

impl App {
    /// Restore the saved layout for open windows with matching identities.
    /// Called once at startup, before the first sync() (which adds the remaining windows).
    pub(crate) unsafe fn restore_layout(&mut self) {
        if !self.cfg.remember_layout {
            return;
        }
        let Some(path) = layout_state::path() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Some((saved, idents)) = layout_state::decode(&text) else {
            eprintln!("layout.state tidak dikenali, diabaikan");
            return;
        };
        let mut list: Vec<HWND> = Vec::new();
        let _ = EnumWindows(Some(collect), LPARAM(&mut list as *mut Vec<HWND> as isize));
        let live: HashMap<WindowId, String> =
            list.iter().map(|&h| (wid(h), identity(h))).collect();
        self.ws.restore(&saved, |id| {
            matches!((live.get(&id), idents.get(&id.0)), (Some(a), Some(b)) if a == b)
        });
        // The just-restored layout does not need to be written again yet.
        self.saved_layout = Some(self.ws.snapshot());
    }

    /// Write the layout to the file if it has changed since the last write.
    pub(crate) unsafe fn save_layout(&mut self) {
        if !self.cfg.remember_layout {
            return;
        }
        let snap = self.ws.snapshot();
        if self.saved_layout.as_ref() == Some(&snap) {
            return;
        }
        let Some(path) = layout_state::path() else {
            return;
        };
        // Compute each window's identity once (opening a process is not cheap).
        let ids: Vec<WindowId> = snap
            .strips
            .iter()
            .flat_map(|s| s.items.iter().map(|it| it.id))
            .collect();
        for id in &ids {
            if !self.idents.contains_key(id) {
                if let Some(&h) = self.map.get(id) {
                    self.idents.insert(*id, identity(h));
                }
            }
        }
        self.idents.retain(|id, _| ids.contains(id));
        let text = layout_state::encode(&snap, |id| self.idents.get(&id).cloned());
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            self.saved_layout = Some(snap);
        }
    }
}
