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
/// such attribute and keeps them square. The border's colours follow the
/// theme (`caption::set_dark`), not set here: a light theme started with
/// a dark border.
pub fn round_corners(hwnd: isize) {
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;
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

/// The window shown at its restored rectangle `normal` (pixels, work-area
/// coordinates), maximized or not: back from the mini player.
pub fn show_placed(hwnd: isize, normal: [i32; 4], maximized: bool) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, SetWindowPlacement, WINDOWPLACEMENT,
    };
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = size_of::<WINDOWPLACEMENT>() as u32;
    unsafe { GetWindowPlacement(hwnd as _, &mut placement) };
    let [left, top, right, bottom] = normal;
    placement.rcNormalPosition = RECT {
        left,
        top,
        right,
        bottom,
    };
    placement.showCmd = if maximized {
        SW_SHOWMAXIMIZED
    } else {
        SW_SHOWNORMAL
    } as u32;
    placement.flags = 0;
    unsafe { SetWindowPlacement(hwnd as _, &placement) };
}

/// The window's rectangle on the screen, in pixels.
pub fn window_rect(hwnd: isize) -> Option<[i32; 4]> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r: RECT = unsafe { std::mem::zeroed() };
    (unsafe { GetWindowRect(hwnd as _, &mut r) } != 0).then_some([r.left, r.top, r.right, r.bottom])
}

/// The window, restored if maximized, moved and sized to `rect` (pixels,
/// screen coordinates), shown as it was.
pub fn set_rect(hwnd: isize, rect: [i32; 4]) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsZoomed, SW_RESTORE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos,
        ShowWindow,
    };
    unsafe {
        if IsZoomed(hwnd as _) != 0 {
            ShowWindow(hwnd as _, SW_RESTORE);
        }
        SetWindowPos(
            hwnd as _,
            std::ptr::null_mut(),
            rect[0],
            rect[1],
            rect[2] - rect[0],
            rect[3] - rect[1],
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
        );
    }
}

/// The work area (the screen without the taskbar, pixels) of the monitor
/// `rect` is mostly on, else of the nearest one.
pub fn work_area_at(rect: [i32; 4]) -> Option<[i32; 4]> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
    };
    let r = RECT {
        left: rect[0],
        top: rect[1],
        right: rect[2],
        bottom: rect[3],
    };
    let monitor = unsafe { MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST) };
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let w = info.rcWork;
    Some([w.left, w.top, w.right, w.bottom])
}

/// Whether `rect` (pixels) is at least partly on a monitor.
pub fn on_a_monitor(rect: [i32; 4]) -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
    let r = RECT {
        left: rect[0],
        top: rect[1],
        right: rect[2],
        bottom: rect[3],
    };
    !unsafe { MonitorFromRect(&r, MONITOR_DEFAULTTONULL) }.is_null()
}

/// The mini player's least side in pixels while the window is it (0: it
/// is not): its least size and its square shape are kept here
/// (`WM_GETMINMAXINFO`, `WM_SIZING`), since winit's least size is the whole
/// window's and its viewport command would come a frame late (metalmpv's
/// `set_mini`).
static MINI_LEAST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// The whole window's least size in points (f32 bits; 0: winit's), kept
/// here as the mini player's is: a layout's least size given to winit
/// comes a frame after the window is put at its size.
static LEAST: [std::sync::atomic::AtomicU32; 2] = [
    std::sync::atomic::AtomicU32::new(0),
    std::sync::atomic::AtomicU32::new(0),
];

pub fn set_least(points: [f32; 2]) {
    for (cell, value) in LEAST.iter().zip(points) {
        cell.store(value.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// The least size of [`set_least`] in pixels for `hwnd`, its frame
/// included; none before it is set.
fn least_pixels(hwnd: isize) -> Option<[i32; 2]> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: isize) -> u32;
    }
    let points = LEAST
        .each_ref()
        .map(|c| f32::from_bits(c.load(std::sync::atomic::Ordering::Relaxed)));
    if points[0] <= 0.0 {
        return None;
    }
    let scale = match unsafe { GetDpiForWindow(hwnd) } {
        0 => 1.0,
        dpi => dpi as f32 / 96.0,
    };
    let [bx, by] = border(hwnd);
    Some([
        (points[0] * scale).round() as i32 + bx,
        (points[1] * scale).round() as i32 + by,
    ])
}

pub fn set_mini(least: Option<i32>) {
    MINI_LEAST.store(least.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
}

/// How near an edge of the work area the mini player is drawn to it while
/// it moves, in pixels at 96 dpi (metalmpv's).
const EDGE_REACH: i32 = 16;

/// Where the window and the pointer were when a move began
/// (`WM_ENTERSIZEMOVE`).
static MOVE_START: std::sync::Mutex<Option<([i32; 4], [i32; 2])>> = std::sync::Mutex::new(None);

fn move_started(hwnd: isize) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut cursor: POINT = unsafe { std::mem::zeroed() };
    let start = window_rect(hwnd)
        .filter(|_| unsafe { GetCursorPos(&mut cursor) } != 0)
        .map(|rect| (rect, [cursor.x, cursor.y]));
    if let Ok(mut s) = MOVE_START.lock() {
        *s = start;
    }
}

/// `WM_MOVING` of the mini player: the rectangle at `lparam` put against
/// the edges of the work area it comes within [`EDGE_REACH`] of
/// (metalmpv's `snap`). The rectangle snapped is where the pointer alone
/// puts the window (the start plus the pointer's way): Windows works the
/// next rectangle out from the one given back, so a snapped one would keep
/// the window at the edge for as long as each step stays within the reach.
fn snap(hwnd: isize, lparam: windows_sys::Win32::Foundation::LPARAM) {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: isize) -> u32;
    }
    let rect = unsafe { &mut *(lparam as *mut RECT) };
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    let mut cursor: POINT = unsafe { std::mem::zeroed() };
    if unsafe { GetCursorPos(&mut cursor) } != 0
        && let Some((r, c)) = MOVE_START.lock().ok().and_then(|s| *s)
        && (r[2] - r[0], r[3] - r[1]) == (w, h)
    {
        let (dx, dy) = (cursor.x - c[0], cursor.y - c[1]);
        *rect = RECT {
            left: r[0] + dx,
            top: r[1] + dy,
            right: r[0] + dx + w,
            bottom: r[1] + dy + h,
        };
    }
    let moved = [rect.left, rect.top, rect.right, rect.bottom];
    let Some(work) = work_area_at(moved) else {
        return;
    };
    let dpi = match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi as i32,
    };
    let [left, top, right, bottom] = snapped_to_edges(moved, work, EDGE_REACH * dpi / 96);
    *rect = RECT {
        left,
        top,
        right,
        bottom,
    };
}

/// `rect` (a window being moved) put against each edge of `work` it comes
/// within `reach` pixels of: the mini player's magnet (metalmpv's).
pub fn snapped_to_edges(rect: [i32; 4], work: [i32; 4], reach: i32) -> [i32; 4] {
    let shift = |a: i32, b: i32, lo: i32, hi: i32| {
        if (lo - a).abs() <= reach {
            lo - a
        } else if (hi - b).abs() <= reach {
            hi - b
        } else {
            0
        }
    };
    let dx = shift(rect[0], rect[2], work[0], work[2]);
    let dy = shift(rect[1], rect[3], work[1], work[3]);
    [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]
}

/// The size of the window's frame around its client area, in pixels.
fn border(hwnd: isize) -> [i32; 2] {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
    let mut window: RECT = unsafe { std::mem::zeroed() };
    let mut client: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(hwnd as _, &mut window) } == 0
        || unsafe { GetClientRect(hwnd as _, &mut client) } == 0
    {
        return [0, 0];
    }
    [
        (window.right - window.left) - (client.right - client.left),
        (window.bottom - window.top) - (client.bottom - client.top),
    ]
}

/// `rect` (a window being sized by its `edge`, `WMSZ_*`) with a square
/// client area (`border` pixels less across and down): by a side edge or
/// a corner the width counts, by the top or the bottom edge the height;
/// the edges across from the one dragged stay (metalmpv's `kept_aspect`).
pub fn squared(rect: [i32; 4], edge: usize, border: [i32; 2]) -> [i32; 4] {
    // WMSZ_LEFT 1, RIGHT 2, TOP 3, TOPLEFT 4, TOPRIGHT 5, BOTTOM 6,
    // BOTTOMLEFT 7, BOTTOMRIGHT 8.
    let [left, mut top, mut right, mut bottom] = rect;
    match edge {
        1 | 2 | 4 | 5 | 7 | 8 => {
            let side = right - left - border[0];
            if matches!(edge, 4 | 5) {
                top = bottom - side - border[1];
            } else {
                bottom = top + side + border[1];
            }
        }
        3 | 6 => right = left + (bottom - top - border[1]) + border[0],
        _ => {}
    }
    [left, top, right, bottom]
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
    /// The tray icon clicked, or its menu's choice (`tray.rs`).
    Tray(crate::tray::TrayCmd),
    /// `WM_CLOSE` while the window is minimized or hidden (`taskkill`,
    /// `build.py`, the installer): eframe acts on a close only in a frame
    /// it paints, which such a window never gets, so the app saves and
    /// ends itself.
    CloseNow,
    /// Explorer started (again): the tray icon is to be added anew
    /// (`TaskbarCreated`).
    TrayCreated,
}

/// `TaskbarButtonCreated`, registered by [`hook_window`].
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// `TaskbarCreated` (Explorer started), registered by [`hook_window`].
static EXPLORER_STARTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

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
        let started = wide("TaskbarCreated");
        let started = unsafe { RegisterWindowMessageW(started.as_ptr()) };
        EXPLORER_STARTED.store(started, std::sync::atomic::Ordering::Relaxed);
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
                started,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND,
                crate::tray::WM_TRAY,
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
        DefWindowProcW, ENDSESSION_CLOSEAPP, MINMAXINFO, PBT_APMSUSPEND, WM_APPCOMMAND, WM_CLOSE,
        WM_COMMAND, WM_ENDSESSION, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_GETMINMAXINFO, WM_KEYDOWN,
        WM_KEYUP, WM_MOVING, WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_SIZING,
    };
    // THBN_CLICKED in WM_COMMAND's high word.
    const THBN_CLICKED: usize = 0x1800;
    if msg != 0 && msg == TASKBAR_CREATED.load(std::sync::atomic::Ordering::Relaxed) {
        hook(Hooked::TaskbarButton);
    }
    if msg != 0 && msg == EXPLORER_STARTED.load(std::sync::atomic::Ordering::Relaxed) {
        hook(Hooked::TrayCreated);
    }
    // The tray icon's events (`NOTIFYICON_VERSION_4`: the event in the low
    // word of `lparam`, the pointer in `wparam`).
    if msg == crate::tray::WM_TRAY {
        const NIN_SELECT: u32 = 0x400;
        const NIN_KEYSELECT: u32 = 0x401;
        const WM_CONTEXTMENU: u32 = 0x007B;
        match lparam as u32 & 0xFFFF {
            // At once, without waiting for a frame of the minimized window.
            NIN_SELECT | NIN_KEYSELECT => bring_back(hwnd as isize),
            WM_CONTEXTMENU => {
                let x = (wparam & 0xFFFF) as u16 as i16 as i32;
                let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                match crate::tray::menu(hwnd as isize, x, y) {
                    Some(crate::tray::TrayCmd::Show) => bring_back(hwnd as isize),
                    Some(cmd) => hook(Hooked::Tray(cmd)),
                    None => {}
                }
            }
            _ => {}
        }
        return 0;
    }
    let media = MEDIA_KEYS.load(std::sync::atomic::Ordering::Relaxed);
    // While the window lets the keys go, a media key goes to
    // `DefWindowProc`, which makes it the `WM_APPCOMMAND` other players
    // take: winit answers every key itself without it.
    if !media && matches!(msg, WM_KEYDOWN | WM_KEYUP) && media_vk(wparam as u32).is_some() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    match msg {
        WM_CLOSE
            if is_minimized(hwnd as isize)
                || unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd)
                } == 0 =>
        {
            hook(Hooked::CloseNow);
            return 0;
        }
        // The mini player's least size, else the layout's, after winit's.
        WM_GETMINMAXINFO if lparam != 0 => {
            let mini = MINI_LEAST.load(std::sync::atomic::Ordering::Relaxed);
            let least = if mini > 0 {
                let [bx, by] = border(hwnd as isize);
                Some([mini + bx, mini + by])
            } else {
                least_pixels(hwnd as isize)
            };
            if let Some([x, y]) = least {
                let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
                let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
                info.ptMinTrackSize.x = x;
                info.ptMinTrackSize.y = y;
                return result;
            }
        }
        // The mini player is drawn to the work area's edges as it moves.
        WM_ENTERSIZEMOVE => move_started(hwnd as isize),
        WM_EXITSIZEMOVE => {
            if let Ok(mut start) = MOVE_START.lock() {
                *start = None;
            }
        }
        WM_MOVING if lparam != 0 && MINI_LEAST.load(std::sync::atomic::Ordering::Relaxed) > 0 => {
            snap(hwnd as isize, lparam);
            return 1;
        }
        // The mini player stays square while its edges are dragged.
        WM_SIZING if lparam != 0 && MINI_LEAST.load(std::sync::atomic::Ordering::Relaxed) > 0 => {
            let r = unsafe { &mut *(lparam as *mut windows_sys::Win32::Foundation::RECT) };
            let [left, top, right, bottom] = squared(
                [r.left, r.top, r.right, r.bottom],
                wparam,
                border(hwnd as isize),
            );
            *r = windows_sys::Win32::Foundation::RECT {
                left,
                top,
                right,
                bottom,
            };
            return 1;
        }
        WM_COMMAND if (wparam >> 16) & 0xFFFF == THBN_CLICKED => {
            hook(Hooked::Thumb((wparam & 0xFFFF) as u16));
            return 0;
        }
        // Answered as DefWindowProc does (yes); the app saves before the
        // process is ended. The Restart Manager (an installer replacing
        // the exe) asks the app to close and waits for it.
        WM_QUERYENDSESSION if lparam as u32 & ENDSESSION_CLOSEAPP != 0 => hook(Hooked::Close),
        WM_QUERYENDSESSION => hook(Hooked::EndSession),
        // The session ends now: the process may be ended once this is
        // answered, before a frame saves what EndSession asked for.
        WM_ENDSESSION if wparam != 0 && lparam as u32 & ENDSESSION_CLOSEAPP == 0 => {
            crate::settings::Settings::save_kept();
        }
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

/// The top-level windows of this thread, hidden ones too (the program's
/// and its viewports', made hidden: `ui::OwnWindow`).
pub fn thread_windows() -> Vec<isize> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumThreadWindows;
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: `lparam` is the pointer to `out` passed below, and the
        // callback runs only during the call.
        let out = unsafe { &mut *(lparam as *mut Vec<isize>) };
        out.push(hwnd as isize);
        1
    }
    let mut out: Vec<isize> = Vec::new();
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(collect),
            (&raw mut out) as LPARAM,
        )
    };
    out
}

/// Whether `hwnd` is still a window (not destroyed).
pub fn is_window(hwnd: isize) -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as _) != 0 }
}

/// The window of this thread titled `title` (a viewport of the program's:
/// eframe gives no handle for it), hidden or not.
pub fn thread_window(title: &str) -> Option<isize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    thread_windows().into_iter().find(|&h| {
        let mut text = [0u16; 128];
        let n = unsafe { GetWindowTextW(h as _, text.as_mut_ptr(), text.len() as i32) };
        n > 0 && String::from_utf16_lossy(&text[..n as usize]) == title
    })
}

/// A window of the program's own (the settings'): owned by `owner`, so it
/// stays over it and is minimized with it, rounded as it is, and centred
/// over it within its work area.
pub fn adopt(hwnd: isize, owner: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWLP_HWNDPARENT, SetWindowLongPtrW};
    unsafe { SetWindowLongPtrW(hwnd as _, GWLP_HWNDPARENT, owner) };
    round_corners(hwnd);
    if let (Some(main), Some(own)) = (window_rect(owner), window_rect(hwnd)) {
        let (w, h) = (own[2] - own[0], own[3] - own[1]);
        let x = (main[0] + main[2] - w) / 2;
        let y = (main[1] + main[3] - h) / 2;
        let mut rect = [x, y, x + w, y + h];
        if let Some(work) = work_area_at(main) {
            let dx = (work[2] - rect[2]).min(0) + (work[0] - rect[0]).max(0);
            let dy = (work[3] - rect[3]).min(0) + (work[1] - rect[1]).max(0);
            rect = [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy];
        }
        set_rect(hwnd, rect);
    }
}

/// The window is minimized.
pub fn is_minimized(hwnd: isize) -> bool {
    // SAFETY: a question about the app's own window.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsIconic(hwnd as _) != 0 }
}

/// The window back from minimized (or as it was) and in front.
pub fn bring_back(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    // SAFETY: the app's own window.
    unsafe {
        if is_minimized(hwnd) {
            ShowWindow(hwnd as _, SW_RESTORE);
        }
        SetForegroundWindow(hwnd as _);
    }
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

/// The local calendar day (year, month, day) of Unix seconds, by Windows'
/// time zone rules for that day; the UTC day if Windows cannot say.
pub fn local_date(unix_secs: u64) -> (u16, u16, u16) {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // 100 ns ticks since 1601-01-01.
    let ticks = (unix_secs + 11_644_473_600) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    // SAFETY: plain structs filled by Windows; a null zone is the current one.
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut utc) == 0 {
            return (1970, 1, 1);
        }
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return (utc.wYear, utc.wMonth, utc.wDay);
        }
        (local.wYear, local.wMonth, local.wDay)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_mini_player_is_drawn_to_the_edges() {
        use super::snapped_to_edges;
        let work = [0, 0, 1920, 1040];
        // Near the right and bottom edges: against them.
        assert_eq!(
            snapped_to_edges([1500, 620, 1910, 1030], work, 16),
            [1510, 630, 1920, 1040]
        );
        // Near the left edge only.
        assert_eq!(
            snapped_to_edges([12, 300, 312, 600], work, 16),
            [0, 300, 300, 600]
        );
        // Away from them: as it is.
        let free = [400, 300, 700, 600];
        assert_eq!(snapped_to_edges(free, work, 16), free);
    }

    #[test]
    fn the_mini_player_stays_square() {
        use super::squared;
        // By the right edge: the width counts, the bottom follows.
        assert_eq!(
            squared([100, 100, 400, 350], 2, [0, 0]),
            [100, 100, 400, 400]
        );
        // By the top left corner: the bottom stays, the top moves.
        assert_eq!(squared([90, 120, 400, 400], 4, [0, 0]), [90, 90, 400, 400]);
        // By the bottom edge: the height counts, the right edge follows;
        // with a frame around the client area.
        assert_eq!(squared([0, 0, 300, 316], 6, [16, 8]), [0, 0, 324, 316]);
    }

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
