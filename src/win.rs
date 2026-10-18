//! Win32 helpers shared by the apps of this family: a message box that
//! needs no window, the parent console for the command line modes, the
//! shell's "open", and an HTTPS GET through WinHTTP for the update check.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// A NUL-terminated UTF-16 string for the Win32 `W` functions.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// Whether this process runs elevated (as administrator): only then are
/// Explorer's messages to it, from below, held back (UIPI) and let
/// through by hand.
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ELEVATED.get_or_init(|| unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    })
}

/// `bytes` in the ANSI code page of Windows (Windows-1251 on a Russian
/// one) as text.
pub fn from_ansi(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};
    let Ok(len) = i32::try_from(bytes.len()) else {
        return String::from_utf8_lossy(bytes).into_owned();
    };
    if len == 0 {
        return String::new();
    }
    let n = unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if n <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; n as usize];
    let n = unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n) };
    String::from_utf16_lossy(&wide[..n.max(0) as usize])
}

/// Show `text` in a system message box with an error icon, waiting until
/// it is closed. It needs no window of the app, so it works from any thread
/// and while the app is failing (the panic hook). user32 is declared by
/// hand, to spare a `windows-sys` feature.
pub fn error_box(title: &str, text: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    const MB_SETFOREGROUND: u32 = 0x10000;
    let (text, title) = (wide(text), wide(title));
    unsafe {
        MessageBoxW(
            0,
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// The release build is a GUI-subsystem program with no console of its own,
/// so text printed by the command line modes (`--version`, `--help`,
/// `--bench`) would be lost when started from cmd or PowerShell. Attach to
/// the parent's console in that case; output already redirected (a pipe or
/// a file) is left untouched. A GUI program started by Python's
/// `subprocess` gets no handles either, hence the check.
pub fn attach_parent_console() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    if std::io::stdout().as_raw_handle().is_null() {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// Open `target` (a file, a folder or a URI such as `https://…` or
/// `ms-settings:…`) as Explorer would. eframe opens links only with its
/// `links` feature (the webbrowser crate), which is off, so About and the
/// update button go through this.
pub fn shell_open(target: impl AsRef<OsStr>) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = wide("open");
    let file = wide(target);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    result as isize > 32
}

/// HTTPS GET of `path` on `host` through WinHTTP, which brings the system's
/// TLS and proxy settings (no HTTP crate); `headers` are extra request
/// lines joined with `\r\n`. Returns the status code and the body, refused
/// past `limit` bytes. Blocks for up to `timeout_ms` per step, so call it
/// off the UI thread.
pub fn https_get(
    host: &str,
    path: &str,
    agent: &str,
    headers: &str,
    timeout_ms: i32,
    limit: usize,
) -> Result<(u32, Vec<u8>), String> {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Networking::WinHttp::*;

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
    fn handle(h: *mut c_void) -> Result<Handle, String> {
        if h.is_null() {
            Err(winhttp_error())
        } else {
            Ok(Handle(h))
        }
    }
    fn check(ok: windows_sys::core::BOOL) -> Result<(), String> {
        if ok == 0 {
            Err(winhttp_error())
        } else {
            Ok(())
        }
    }

    let (agent, host, path, headers) = (wide(agent), wide(host), wide(path), wide(headers));
    let verb = wide("GET");
    let t = timeout_ms;
    // Declared in this order, the handles close request first.
    unsafe {
        let session = handle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ))?;
        check(WinHttpSetTimeouts(session.0, t, t, t, t))?;
        let connection = handle(WinHttpConnect(
            session.0,
            host.as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?;
        let request = handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        check(WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            null(),
            0,
            0,
            0,
        ))?;
        check(WinHttpReceiveResponse(request.0, null_mut()))?;
        let mut status = 0u32;
        let mut len = size_of::<u32>() as u32;
        check(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&raw mut status).cast(),
            &mut len,
            null_mut(),
        ))?;
        let mut body = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            check(WinHttpReadData(
                request.0,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
            ))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > limit {
                return Err(format!("the answer is larger than {limit} bytes"));
            }
        }
        Ok((status, body))
    }
}

/// The last WinHTTP error in words; the system's message table lacks them.
fn winhttp_error() -> String {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    match code {
        12002 => "the server did not answer in time".into(),
        12007 => "the server name could not be resolved".into(),
        12029 | 12030 => "the connection to the server failed".into(),
        12175 => "the secure connection failed".into(),
        _ => format!("WinHTTP error {code}"),
    }
}

/// Set a DWM window attribute of 32-bit `value`; whether it was taken.
fn dwm_set(hwnd: isize, attribute: u32, value: i32) -> bool {
    // dwmapi is declared by hand, to spare another `windows-sys` feature.
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    unsafe { DwmSetWindowAttribute(hwnd, attribute, (&raw const value).cast(), 4) >= 0 }
}

/// Round the corners of the frameless window as Windows 11 does its own
/// (`DWMWA_WINDOW_CORNER_PREFERENCE` = `DWMWCP_ROUND`); Windows 10 has no
/// such attribute and keeps them square. Also the dark frame colours for
/// the window's border on Windows 11.
pub fn round_corners(hwnd: isize) {
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;
    dwm_set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
    dwm_set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
}

/// Hide the window from the screen while it keeps working (`DWMWA_CLOAK`),
/// or show it again.
pub fn cloak(hwnd: isize, on: bool) {
    const DWMWA_CLOAK: u32 = 13;
    dwm_set(hwnd, DWMWA_CLOAK, i32::from(on));
}

/// The window's restored rectangle in pixels (left, top, right, bottom, in
/// work-area coordinates), also while it is maximized, and whether it is
/// maximized (or minimized from maximized: it comes back maximized).
pub fn placement(hwnd: isize) -> Option<([i32; 4], bool)> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, WINDOWPLACEMENT,
        WPF_RESTORETOMAXIMIZED,
    };
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = size_of::<WINDOWPLACEMENT>() as u32;
    if unsafe { GetWindowPlacement(hwnd as _, &mut placement) } == 0 {
        return None;
    }
    let r = placement.rcNormalPosition;
    let maximized = placement.showCmd == SW_SHOWMAXIMIZED as u32
        || (placement.showCmd == SW_SHOWMINIMIZED as u32
            && placement.flags & WPF_RESTORETOMAXIMIZED != 0);
    (r.right > r.left && r.bottom > r.top)
        .then_some(([r.left, r.top, r.right, r.bottom], maximized))
}

/// Put the window, still hidden, at its restored rectangle `normal`
/// (pixels, work-area coordinates; or `default` points centred where it
/// is), maximized or not. Windows moves a rectangle that is on no monitor
/// any more onto one. A hidden window stays hidden (eframe shows it after
/// its first frame); a maximized one is shown maximized in one step, as
/// winit shows a window created maximized before it is painted (a white
/// flash) and maximizing after eframe shows it would take two steps: the
/// caller cloaks it until its first maximized frame (qview's way).
pub fn place(hwnd: isize, normal: Option<[i32; 4]>, default: [f32; 2], maximized: bool) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_HIDE, SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT,
    };
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: isize) -> u32;
    }
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = size_of::<WINDOWPLACEMENT>() as u32;
    unsafe { GetWindowPlacement(hwnd as _, &mut placement) };
    let [left, top, right, bottom] = normal.unwrap_or_else(|| {
        let r = placement.rcNormalPosition;
        let scale = match unsafe { GetDpiForWindow(hwnd) } {
            0 => 1.0,
            dpi => dpi as f32 / 96.0,
        };
        let (w, h) = ((default[0] * scale) as i32, (default[1] * scale) as i32);
        let (x, y) = ((r.left + r.right - w) / 2, (r.top + r.bottom - h) / 2);
        [x, y, x + w, y + h]
    });
    placement.rcNormalPosition = RECT {
        left,
        top,
        right,
        bottom,
    };
    placement.showCmd = if maximized { SW_SHOWMAXIMIZED } else { SW_HIDE } as u32;
    placement.flags = 0;
    unsafe { SetWindowPlacement(hwnd as _, &placement) };
}

/// The double-click time of Windows in seconds (egui assumes 0.3).
pub fn double_click_time() -> f64 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDoubleClickTime() -> u32;
    }
    unsafe { GetDoubleClickTime() as f64 / 1000.0 }
}

/// What the window's messages asked for, for the app to take each frame
/// ([`take_hooked`]).
static HOOKED: std::sync::Mutex<Vec<Hooked>> = std::sync::Mutex::new(Vec::new());

/// Wakes the app when something is hooked ([`hook_window`]).
static WAKE: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// A key of a keyboard, a headset or a remote that egui does not see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKey {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
}

/// The way a media key came. One press may come more than one way: with
/// the window in front, Windows sends `WM_APPCOMMAND` and then the key,
/// and Windows' media controls may report it too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// `WM_KEYDOWN` with a `VK_MEDIA_*` key.
    Key,
    /// `WM_APPCOMMAND`.
    Command,
    /// Windows' media controls (`smtc.rs`).
    Controls,
}

/// What the window's messages asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hooked {
    /// A media key and the way it came.
    Media(MediaKey, Via),
    /// The taskbar button exists (`TaskbarButtonCreated`): the thumbnail
    /// buttons can be added (`taskbar.rs`).
    TaskbarButton,
    /// A thumbnail button was clicked (its id).
    Thumb(u16),
    /// Windows is ending the session (`WM_QUERYENDSESSION`): winit closes
    /// nothing, so the settings are saved now.
    EndSession,
    /// The Restart Manager asks the app to close (`WM_QUERYENDSESSION`
    /// with `ENDSESSION_CLOSEAPP`): an installer is to replace its files.
    Close,
    /// The computer is going to sleep (`WM_POWERBROADCAST` with
    /// `PBT_APMSUSPEND`): saved now too, as it may not wake.
    Suspend,
}

/// `TaskbarButtonCreated`, registered by [`hook_window`].
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The window takes the media keys ([`set_media_keys`]).
static MEDIA_KEYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Subclass the window once: media keys (`WM_KEYDOWN` with a `VK_MEDIA_*`
/// key, which egui drops, and `WM_APPCOMMAND`, which headsets and remotes
/// send), the taskbar button made and its thumbnail buttons clicked, the
/// end of the session and sleep are queued for [`take_hooked`] and `wake`
/// is called (metalmpv's `hook_window`, without its snapping and
/// fullscreen parts).
pub fn hook_window(hwnd: isize, wake: impl Fn() + Send + Sync + 'static) {
    use std::sync::Once;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    static SUBCLASSED: Once = Once::new();
    SUBCLASSED.call_once(|| {
        use windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW;
        let name = wide("TaskbarButtonCreated");
        let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
        TASKBAR_CREATED.store(message, std::sync::atomic::Ordering::Relaxed);
        let _ = WAKE.set(Box::new(wake));
        // A copy running elevated still hears Explorer (UIPI): the taskbar
        // button made and the thumbnail buttons' clicks.
        #[link(name = "user32")]
        unsafe extern "system" {
            fn ChangeWindowMessageFilterEx(
                hwnd: windows_sys::Win32::Foundation::HWND,
                message: u32,
                action: u32,
                filter: *mut core::ffi::c_void,
            ) -> i32;
        }
        const MSGFLT_ALLOW: u32 = 1;
        // Not elevated, Explorer's messages come through anyway, and the
        // filter is left as it is (closed to processes below this one).
        if is_elevated() {
            for message in [
                message,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND,
            ] {
                unsafe {
                    ChangeWindowMessageFilterEx(
                        hwnd as _,
                        message,
                        MSGFLT_ALLOW,
                        std::ptr::null_mut(),
                    )
                };
            }
        }
        unsafe { SetWindowSubclass(hwnd as _, Some(window_proc), 1, 0) };
    });
}

/// Whether the window takes the media keys; off, the keys and
/// `WM_APPCOMMAND` go on to Windows (and through it to other programs).
pub fn set_media_keys(on: bool) {
    MEDIA_KEYS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// A media key pressed elsewhere (Windows' media controls, `smtc.rs`),
/// queued as one of the window's.
pub fn hook_media(key: MediaKey) {
    hook(Hooked::Media(key, Via::Controls));
}

/// What the window's messages asked for since the last call.
pub fn take_hooked() -> Vec<Hooked> {
    HOOKED
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

fn hook(what: Hooked) {
    if let Ok(mut q) = HOOKED.lock() {
        q.push(what);
    }
    if let Some(wake) = WAKE.get() {
        wake();
    }
}

/// The media key of a virtual key code.
fn media_vk(vk: u32) -> Option<MediaKey> {
    match vk {
        0xB0 => Some(MediaKey::Next),      // VK_MEDIA_NEXT_TRACK
        0xB1 => Some(MediaKey::Previous),  // VK_MEDIA_PREV_TRACK
        0xB2 => Some(MediaKey::Stop),      // VK_MEDIA_STOP
        0xB3 => Some(MediaKey::PlayPause), // VK_MEDIA_PLAY_PAUSE
        _ => None,
    }
}

/// The media key of a `WM_APPCOMMAND` command.
fn media_command(command: u32) -> Option<MediaKey> {
    match command {
        11 => Some(MediaKey::Next),      // APPCOMMAND_MEDIA_NEXTTRACK
        12 => Some(MediaKey::Previous),  // APPCOMMAND_MEDIA_PREVIOUSTRACK
        13 => Some(MediaKey::Stop),      // APPCOMMAND_MEDIA_STOP
        14 => Some(MediaKey::PlayPause), // APPCOMMAND_MEDIA_PLAY_PAUSE
        46 => Some(MediaKey::Play),      // APPCOMMAND_MEDIA_PLAY
        47 => Some(MediaKey::Pause),     // APPCOMMAND_MEDIA_PAUSE
        _ => None,
    }
}

unsafe extern "system" fn window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, ENDSESSION_CLOSEAPP, PBT_APMSUSPEND, WM_APPCOMMAND, WM_COMMAND, WM_KEYDOWN,
        WM_KEYUP, WM_POWERBROADCAST, WM_QUERYENDSESSION,
    };
    // THBN_CLICKED in WM_COMMAND's high word.
    const THBN_CLICKED: usize = 0x1800;
    if msg != 0 && msg == TASKBAR_CREATED.load(std::sync::atomic::Ordering::Relaxed) {
        hook(Hooked::TaskbarButton);
    }
    let media = MEDIA_KEYS.load(std::sync::atomic::Ordering::Relaxed);
    // While the window lets the keys go, a media key goes to
    // `DefWindowProc`, which makes it the `WM_APPCOMMAND` other players
    // take: winit answers every key itself without it.
    if !media && matches!(msg, WM_KEYDOWN | WM_KEYUP) && media_vk(wparam as u32).is_some() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    match msg {
        WM_COMMAND if (wparam >> 16) & 0xFFFF == THBN_CLICKED => {
            hook(Hooked::Thumb((wparam & 0xFFFF) as u16));
            return 0;
        }
        // Answered as DefWindowProc does (yes); the app saves before the
        // process is ended. The Restart Manager (an installer replacing
        // the exe) asks the app to close and waits for it.
        WM_QUERYENDSESSION if lparam as u32 & ENDSESSION_CLOSEAPP != 0 => hook(Hooked::Close),
        WM_QUERYENDSESSION => hook(Hooked::EndSession),
        WM_POWERBROADCAST if wparam == PBT_APMSUSPEND as usize => hook(Hooked::Suspend),
        WM_KEYDOWN if media => {
            if let Some(key) = media_vk(wparam as u32) {
                // Held keys repeat; one press is one command.
                if lparam & (1 << 30) == 0 {
                    hook(Hooked::Media(key, Via::Key));
                }
                return 0;
            }
        }
        WM_APPCOMMAND if media => {
            // GET_APPCOMMAND_LPARAM: the high word without the device bits.
            let command = ((lparam as u32) >> 16) & !0xF000;
            if let Some(key) = media_command(command) {
                hook(Hooked::Media(key, Via::Command));
                return 1;
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Open the folder of `file` in Explorer with the file selected
/// (`SHOpenFolderAndSelectItems`: `explorer /select` leaves the window
/// behind the app's). On a thread of its own with COM, as the shell may
/// take a while; the functions are declared by hand to spare features.
pub fn show_in_folder(file: &std::path::Path) {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHParseDisplayName(
            name: *const u16,
            bind: *mut core::ffi::c_void,
            pidl: *mut *mut core::ffi::c_void,
            sfgao_in: u32,
            sfgao_out: *mut u32,
        ) -> i32;
        fn SHOpenFolderAndSelectItems(
            folder: *const core::ffi::c_void,
            count: u32,
            items: *const *const core::ffi::c_void,
            flags: u32,
        ) -> i32;
        fn ILFree(pidl: *const core::ffi::c_void);
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *const core::ffi::c_void, coinit: u32) -> i32;
        fn CoUninitialize();
    }
    const COINIT_APARTMENTTHREADED: u32 = 2;
    let path = wide(file);
    let spawned = std::thread::Builder::new()
        .name("show in folder".into())
        .spawn(move || unsafe {
            let com = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED) >= 0;
            let mut pidl = std::ptr::null_mut();
            let parsed = SHParseDisplayName(
                path.as_ptr(),
                std::ptr::null_mut(),
                &mut pidl,
                0,
                std::ptr::null_mut(),
            );
            if parsed >= 0 && !pidl.is_null() {
                let hr = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
                if hr < 0 {
                    log::warn!("cannot show the file in its folder: 0x{hr:08x}");
                }
                ILFree(pidl);
            }
            if com {
                CoUninitialize();
            }
        });
    if let Err(e) = spawned {
        log::warn!("cannot show the file in its folder: {e}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn media_keys() {
        use super::{MediaKey, media_command, media_vk};
        assert_eq!(media_vk(0xB3), Some(MediaKey::PlayPause));
        assert_eq!(media_vk(0xB0), Some(MediaKey::Next));
        assert_eq!(media_vk(0x41), None);
        assert_eq!(media_command(46), Some(MediaKey::Play));
        assert_eq!(media_command(8), None);
    }
}
