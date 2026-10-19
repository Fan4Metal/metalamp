//! Watching the library's folders: one thread waits on Windows' change
//! notifications of every folder and its subfolders (a file or folder
//! made, removed or renamed, a file written), and says so; the app waits
//! for a quiet moment ([`QUIET`]) and reads the folders again, which takes
//! only the files whose size or time changed. Made anew with each scan, so
//! a folder added, or a disk plugged in and read again, is watched too.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long the folders must rest after a change before they are read
/// again: a copy of an album is one scan, not one per file.
pub const QUIET: Duration = Duration::from_secs(3);

/// Windows waits on at most 64 handles at once; one is the stop event.
const MAX_FOLDERS: usize = 63;

pub struct Watcher {
    changed: Arc<AtomicBool>,
    /// The event that stops the thread (a `HANDLE` as a number; 0: none).
    stop: usize,
    /// A thread waits on `stop`, and closes it when told to stop.
    threaded: bool,
}

impl Watcher {
    /// Watches `folders` (those that can be watched now); `wake` is woken
    /// after [`QUIET`] past each change.
    pub fn start(folders: &[PathBuf], wake: &super::scan::Wake) -> Watcher {
        use windows_sys::Win32::System::Threading::CreateEventW;
        let changed = Arc::new(AtomicBool::new(false));
        // SAFETY: a manual-reset event, not signalled, without a name.
        let stop = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) } as usize;
        let mut watcher = Watcher {
            changed: Arc::clone(&changed),
            stop,
            threaded: false,
        };
        if stop == 0 || folders.is_empty() {
            return watcher;
        }
        let folders = folders.to_vec();
        let flag = Arc::clone(&changed);
        let wake = Arc::clone(wake);
        let spawned = std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || watch(&folders, stop, &flag, &wake));
        match spawned {
            Ok(_) => watcher.threaded = true,
            Err(e) => log::warn!("cannot start watching the folders: {e}"),
        }
        watcher
    }

    /// Whether anything changed since the last call.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::Relaxed)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::SetEvent;
        if self.stop == 0 {
            return;
        }
        // SAFETY: the event made in `start`: a thread waiting on it is
        // told to stop and closes it itself; else it is closed here.
        unsafe {
            if self.threaded {
                SetEvent(self.stop as _);
            } else {
                CloseHandle(self.stop as _);
            }
        }
    }
}

/// The thread: waits on the folders' notifications and the stop event.
fn watch(folders: &[PathBuf], stop: usize, changed: &AtomicBool, wake: &super::scan::Wake) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE, FindCloseChangeNotification, FindFirstChangeNotificationW,
        FindNextChangeNotification,
    };
    use windows_sys::Win32::System::Threading::{INFINITE, WaitForMultipleObjects};

    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE;
    let mut handles: Vec<HANDLE> = vec![stop as HANDLE];
    for folder in folders.iter().take(MAX_FOLDERS) {
        let wide: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated path; the subtree is watched.
        let h = unsafe { FindFirstChangeNotificationW(wide.as_ptr(), 1, filter) };
        if h == INVALID_HANDLE_VALUE || h.is_null() {
            log::debug!("watch: cannot watch {}", folder.display());
            continue;
        }
        handles.push(h);
    }
    log::debug!("watch: {} folders", handles.len() - 1);
    loop {
        // SAFETY: valid handles, fewer than 64.
        let r =
            unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE) };
        let i = r.wrapping_sub(WAIT_OBJECT_0) as usize;
        if i == 0 || i >= handles.len() {
            // Stopped, or the wait failed.
            break;
        }
        changed.store(true, Ordering::Relaxed);
        // The app notes the change and waits out the rest itself.
        if let Some(ctx) = wake.get() {
            ctx.request_repaint();
        }
        // SAFETY: a notification handle of this thread.
        if unsafe { FindNextChangeNotification(handles[i]) } == 0 {
            // The folder went away (a disk unplugged): no more waiting on it.
            unsafe { FindCloseChangeNotification(handles[i]) };
            handles.remove(i);
        }
    }
    for &h in &handles[1..] {
        // SAFETY: notification handles of this thread, closed once.
        unsafe { FindCloseChangeNotification(h) };
    }
    // SAFETY: the stop event, no longer waited on; `Watcher` gave it up.
    unsafe { CloseHandle(handles[0]) };
}
