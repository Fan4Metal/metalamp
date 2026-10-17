//! One window per copy of the player: a second start hands its files to the
//! window already open, brings that window forward and exits (taken from
//! qview's `instance.rs`).
//!
//! The first start owns a named mutex; once its window exists it also has a
//! message-only window of a class with the same name, which receives the
//! paths in one `WM_COPYDATA` (separated by NULs). Both names are derived from the path of the exe,
//! so an installed copy and a development build do not take each other's
//! files. Explorer starts one process per file of a multiple selection:
//! paths that arrive close together make one playlist (`App`).

use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, FindWindowExW, HWND_MESSAGE, IsIconic, RegisterClassW,
    SMTO_ABORTIFHUNG, SMTO_BLOCK, SW_RESTORE, SendMessageTimeoutW, SetForegroundWindow, ShowWindow,
    WM_COPYDATA, WNDCLASSW,
};

use crate::win::wide;

/// Marks a `WM_COPYDATA` as the player's (`COPYDATASTRUCT::dwData`).
const MAGIC: usize = 0x6d61_6d70;
/// How long a second start waits for the window of the first one, which
/// appears soon after its start, or for a closing one to exit.
const WAIT: Duration = Duration::from_secs(5);
/// How long the first start may take to answer.
const ANSWER_MS: u32 = 3000;

/// Paths handed over since `App` last took them, with the time each came.
static RECEIVED: Mutex<Vec<(PathBuf, Instant)>> = Mutex::new(Vec::new());
/// The UI to wake when a path arrives, and the main window.
static TARGET: OnceLock<(egui::Context, isize)> = OnceLock::new();
/// The player is closing: paths handed over now would never be taken, so
/// the later start is told no and plays them itself.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Take no more paths: the player is closing.
pub fn closing() {
    CLOSING.store(true, Ordering::Relaxed);
}

/// Name of the mutex and of the window class, the same for every start of
/// this exe in this session.
fn name() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    exe.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("metalamp.{:016x}", hasher.finish())
}

/// Called at start-up. If the player is already running from this exe,
/// hand it `paths` (none: only bring its window forward) and return true:
/// this process has nothing more to do. Otherwise this process is the first
/// one and stays so until it exits.
pub fn forward(paths: &[PathBuf]) -> bool {
    let name = name();
    let mutex_name = wide(format!("Local\\{name}"));
    let class = wide(&name);
    let deadline = Instant::now() + WAIT;
    loop {
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            log::warn!("CreateMutexW failed: {}", std::io::Error::last_os_error());
            return false;
        }
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            // The first start: the handle stays open while the process lives.
            return false;
        }
        unsafe { CloseHandle(mutex) };
        let window = unsafe {
            FindWindowExW(
                HWND_MESSAGE,
                std::ptr::null_mut(),
                class.as_ptr(),
                std::ptr::null(),
            )
        };
        if !window.is_null() {
            // A first start that does not answer (hung) is left alone and
            // this one runs on its own.
            return send(window, paths);
        }
        // The first start has not made its window yet, or is closing.
        if Instant::now() >= deadline {
            log::warn!("the player is running but its window was not found");
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Send `paths` to the message window `to` in one message (all taken or
/// none), then bring the main window it answers with to the foreground:
/// this process was started by the user, so it may.
fn send(to: HWND, paths: &[PathBuf]) -> bool {
    let text = joined(paths);
    let bytes = text.len() * 2;
    let data = COPYDATASTRUCT {
        dwData: MAGIC,
        cbData: bytes as u32,
        lpData: text.as_ptr() as *mut _,
    };
    let mut answer = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            to,
            WM_COPYDATA,
            0,
            &raw const data as LPARAM,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            ANSWER_MS,
            &mut answer,
        )
    };
    if ok == 0 || answer == 0 {
        log::warn!("the running player did not take the file");
        return false;
    }
    unsafe { SetForegroundWindow(answer as HWND) };
    true
}

/// `paths` as UTF-16, separated by NULs, without a terminating one.
fn joined(paths: &[PathBuf]) -> Vec<u16> {
    let mut text = Vec::new();
    for path in paths {
        if !text.is_empty() {
            text.push(0);
        }
        text.extend(path.as_os_str().encode_wide());
    }
    text
}

/// The paths of a message's UTF-16 text (empty parts skipped).
fn split(units: &[u16]) -> Vec<PathBuf> {
    units
        .split(|&u| u == 0)
        .filter(|part| !part.is_empty())
        .map(|part| PathBuf::from(OsString::from_wide(part)))
        .collect()
}

/// Make this process the one later starts hand their files to: a
/// message-only window on the UI thread, whose messages the event loop
/// dispatches. `main` is the app's window.
pub fn listen(ctx: &egui::Context, main: isize) {
    if TARGET.set((ctx.clone(), main)).is_err() {
        return;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ChangeWindowMessageFilterEx(
            hwnd: HWND,
            message: u32,
            action: u32,
            filter: *mut core::ffi::c_void,
        ) -> i32;
    }
    const MSGFLT_ALLOW: u32 = 1;
    let class = wide(name());
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            log::warn!("RegisterClassW failed: {}", std::io::Error::last_os_error());
            return;
        }
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if window.is_null() {
            log::warn!(
                "CreateWindowExW failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        // A copy running elevated still takes files from Explorer (UIPI).
        ChangeWindowMessageFilterEx(window, WM_COPYDATA, MSGFLT_ALLOW, std::ptr::null_mut());
    }
}

/// The paths handed over since the previous call, with their arrival times.
pub fn take() -> Vec<(PathBuf, Instant)> {
    std::mem::take(&mut *RECEIVED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Takes a path from a later start, restores the main window if it is
/// minimized and answers with it (0: not a message of the player).
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg != WM_COPYDATA || lparam == 0 {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let data = unsafe { &*(lparam as *const COPYDATASTRUCT) };
    let Some((ctx, main)) = TARGET
        .get()
        .filter(|_| data.dwData == MAGIC && !CLOSING.load(Ordering::Relaxed))
    else {
        return 0;
    };
    if data.cbData > 0 && !data.lpData.is_null() {
        // Read as bytes: the copy the system makes need not be aligned.
        let bytes =
            unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        let now = Instant::now();
        RECEIVED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(split(&units).into_iter().map(|p| (p, now)));
    }
    if unsafe { IsIconic(*main as HWND) } != 0 {
        unsafe { ShowWindow(*main as HWND, SW_RESTORE) };
    }
    ctx.request_repaint();
    *main as LRESULT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_go_in_one_message() {
        let paths = vec![
            PathBuf::from(r"C:\a b\один.mkv"),
            PathBuf::from(r"D:\x.mp4"),
        ];
        assert_eq!(split(&joined(&paths)), paths);
        assert!(joined(&[]).is_empty());
        assert!(split(&[]).is_empty());
    }
}
