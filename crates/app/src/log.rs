//! File logging (`%LOCALAPPDATA%\vscroll-wm\vscroll-wm.log`) with simple size rotation,
//! crash report writer for the panic hook, and helper to open the log folder.

use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Maximum log file size before rotating to `vscroll-wm.log.old` (1 MiB).
const MAX_LOG_BYTES: u64 = 1024 * 1024;

static LOG_LOCK: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Directory for logs: `%LOCALAPPDATA%\vscroll-wm` (fallback to temp dir).
pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("vscroll-wm");
        }
    }
    std::env::temp_dir().join("vscroll-wm")
}

/// Full path to the active log file.
pub fn log_path() -> PathBuf {
    log_dir().join("vscroll-wm.log")
}

/// Initialize file logging: ensure directory exists and rotate if the log exceeds 1 MiB.
pub fn init() {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("vscroll-wm.log");
    rotate_if_needed(&path, MAX_LOG_BYTES);
    if let Ok(mut g) = LOG_LOCK.lock() {
        *g = Some(path);
    }
}

/// If `path` is larger than `max_bytes`, move it to `<path>.old` (overwriting any previous `.old`).
fn rotate_if_needed(path: &Path, max_bytes: u64) {
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.len() < max_bytes {
        return;
    }
    let mut old = path.as_os_str().to_os_string();
    old.push(".old");
    let old_path = PathBuf::from(old);
    let _ = fs::remove_file(&old_path);
    let _ = fs::rename(path, &old_path);
}

/// Format UTC timestamp `YYYY-MM-DD HH:MM:SSZ` from UNIX seconds (no external crate).
fn format_unix_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3_600;
    let min = (rem % 3_600) / 60;
    let sec = rem % 60;

    // Howard Hinnant's civil-from-days algorithm (Gregorian calendar).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + i64::from(m <= 2);

    format!("{year:04}-{m:02}-{d:02} {hour:02}:{min:02}:{sec:02}Z")
}

fn timestamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_secs(secs)
}

fn append_raw(path: &Path, text: &str) {
    rotate_if_needed(path, MAX_LOG_BYTES);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// Write a single formatted log line (`INFO` / `WARN` / `ERROR`).
pub fn write(level: &str, msg: &str) {
    let line = format!("[{}] [{level}] {msg}\n", timestamp_now());
    #[cfg(debug_assertions)]
    {
        if level == "INFO" {
            println!("{msg}");
        } else {
            eprintln!("{msg}");
        }
    }
    if let Ok(g) = LOG_LOCK.lock() {
        if let Some(ref path) = *g {
            append_raw(path, &line);
        }
    }
}

/// Write a crash report block from the panic hook before restoring windows.
/// Uses `try_lock` so panicking while holding `LOG_LOCK` cannot deadlock.
pub fn write_crash_report(info: &PanicHookInfo<'_>) {
    let version = env!("CARGO_PKG_VERSION");
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".into());
    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".into()
    };
    let block = format!(
        "================ CRASH REPORT ================\n\
         time    : {}\n\
         version : {}\n\
         location: {}\n\
         message : {}\n\
         ==============================================\n",
        timestamp_now(),
        version,
        location,
        payload
    );

    let path = LOG_LOCK
        .try_lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(log_path);
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(block.as_bytes());
        let _ = f.flush();
    } else if let Ok(mut f) = File::create(&path) {
        let _ = f.write_all(block.as_bytes());
    }
}

/// True if the current process is running with elevated (administrator) privileges.
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elev = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut TOKEN_ELEVATION as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elev.TokenIsElevated != 0
    }
}

/// Open the log directory in Windows Explorer (creates it first if missing).
pub fn open_log_folder() {
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    let wide: Vec<u16> = dir
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("explore"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::write("INFO", &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::write("WARN", &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_unix_epoch_and_known_dates() {
        assert_eq!(format_unix_secs(0), "1970-01-01 00:00:00Z");
        // 2024-02-29 12:34:56 UTC (leap day)
        assert_eq!(format_unix_secs(1_709_210_096), "2024-02-29 12:34:56Z");
    }

    #[test]
    fn rotates_file_when_exceeding_limit() {
        let dir = std::env::temp_dir().join(format!(
            "vscroll-wm-log-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.log");
        let old = dir.join("test.log.old");

        fs::write(&path, "short").unwrap();
        rotate_if_needed(&path, 100);
        assert!(path.exists());
        assert!(!old.exists());

        fs::write(&path, vec![b'x'; 128]).unwrap();
        rotate_if_needed(&path, 100);
        assert!(!path.exists());
        assert!(old.exists());
        assert_eq!(fs::metadata(&old).unwrap().len(), 128);

        let _ = fs::remove_dir_all(&dir);
    }
}
