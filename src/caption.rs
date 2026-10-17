//! The window's dark frame. The window is frameless with an always dark
//! title row of the app's own, so the frame's colour is set once, while
//! the window is still hidden (`App::new`): set after egui's first frame,
//! Windows 10 would keep the border of the Windows theme. The family's
//! `refresh` (repainting the caption after a theme change) and
//! `thread_windows` (child viewports) are left out: the theme never
//! changes and there are no child viewports.

/// The window's handle, if it is a Win32 window. `eframe::CreationContext`
/// and `eframe::Frame` both implement `HasWindowHandle`.
pub fn hwnd(window: &impl raw_window_handle::HasWindowHandle) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }
}

/// Give window `hwnd` the dark or light caption and frame
/// (`DWMWA_USE_IMMERSIVE_DARK_MODE`). Called while the window is hidden, it
/// is shown in these colours from the start.
#[cfg(windows)]
pub fn set_dark(hwnd: isize, dark: bool) {
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
    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    // The attribute's number before Windows 10 20H1 (1809 to 1909).
    const DWMWA_USE_IMMERSIVE_DARK_MODE_OLD: u32 = 19;
    let value = i32::from(dark);
    let set = |attribute| unsafe {
        DwmSetWindowAttribute(hwnd, attribute, (&raw const value).cast(), 4) >= 0
    };
    if !set(DWMWA_USE_IMMERSIVE_DARK_MODE) {
        set(DWMWA_USE_IMMERSIVE_DARK_MODE_OLD);
    }
}

#[cfg(not(windows))]
pub fn set_dark(_hwnd: isize, _dark: bool) {}
