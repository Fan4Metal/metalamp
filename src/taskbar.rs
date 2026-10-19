//! The window's taskbar button: the position as its progress (yellow when
//! paused) and Previous, Play/Pause and Next under its thumbnail
//! (metalmpv's, the tooltips naming tracks). Windows'
//! `ITaskbarList3`, declared by hand (`windows-sys` has no COM
//! interfaces); COM is already initialised on the UI thread (winit's
//! `OleInitialize` for drag and drop). The buttons can be added only once
//! Windows says the taskbar button exists (`TaskbarButtonCreated`, which
//! `win::hook_window` passes on as `Hooked::TaskbarButton`); their clicks
//! come back as `Hooked::Thumb`.

use std::ffi::c_void;

use windows_sys::core::GUID;

/// The ids of the thumbnail buttons (`WM_COMMAND`'s low word).
pub const PREVIOUS: u16 = 1;
pub const PLAY_PAUSE: u16 = 2;
pub const NEXT: u16 = 3;

const CLSID_TASKBAR_LIST: GUID = GUID::from_u128(0x56fdf344_fd6d_11d0_958a_006097c9a090);
const IID_ITASKBAR_LIST3: GUID = GUID::from_u128(0xea1afb91_9e28_4b86_90e9_9e9f8a5eefaf);

// TBPFLAG
const TBPF_NOPROGRESS: i32 = 0;
const TBPF_NORMAL: i32 = 2;
const TBPF_PAUSED: i32 = 8;
// THUMBBUTTONMASK, THUMBBUTTONFLAGS
const THB_ICON: u32 = 2;
const THB_TOOLTIP: u32 = 4;
const THB_FLAGS: u32 = 8;
const THBF_ENABLED: u32 = 0;
const THBF_DISABLED: u32 = 1;

#[repr(C)]
struct ThumbButton {
    mask: u32,
    id: u32,
    bitmap: u32,
    icon: isize,
    tip: [u16; 260],
    flags: u32,
}

/// `ITaskbarList3`'s methods in their order, those not used as `usize`.
#[repr(C)]
struct Vtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    hr_init: unsafe extern "system" fn(*mut c_void) -> i32,
    add_tab: unsafe extern "system" fn(*mut c_void, isize) -> i32,
    delete_tab: unsafe extern "system" fn(*mut c_void, isize) -> i32,
    activate_tab: usize,
    set_active_alt: usize,
    mark_fullscreen_window: usize,
    set_progress_value: unsafe extern "system" fn(*mut c_void, isize, u64, u64) -> i32,
    set_progress_state: unsafe extern "system" fn(*mut c_void, isize, i32) -> i32,
    register_tab: usize,
    unregister_tab: usize,
    set_tab_order: usize,
    set_tab_active: usize,
    thumb_bar_add_buttons:
        unsafe extern "system" fn(*mut c_void, isize, u32, *const ThumbButton) -> i32,
    thumb_bar_update_buttons:
        unsafe extern "system" fn(*mut c_void, isize, u32, *const ThumbButton) -> i32,
}

/// What the taskbar button shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    /// Per mille of the duration; `None`: no progress (nothing plays, a
    /// stream without a duration).
    pub progress: Option<u64>,
    pub paused: bool,
    pub has_previous: bool,
    pub has_next: bool,
}

pub struct Taskbar {
    list: *mut c_void,
    hwnd: isize,
    /// Play, pause, previous, next.
    icons: [isize; 4],
    /// The side the icons were drawn for and the language of the tooltips:
    /// another monitor's scale or another language makes them anew.
    made_for: (i32, crate::i18n::Lang),
    /// Windows said the taskbar button exists; before that the calls fail.
    ready: bool,
    buttons_added: bool,
    shown: Option<Shown>,
}

impl Taskbar {
    pub fn new(hwnd: isize) -> Option<Self> {
        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoCreateInstance(
                clsid: *const GUID,
                outer: *mut c_void,
                context: u32,
                iid: *const GUID,
                out: *mut *mut c_void,
            ) -> i32;
        }
        const CLSCTX_INPROC_SERVER: u32 = 1;
        let mut list = std::ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_TASKBAR_LIST,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ITASKBAR_LIST3,
                &mut list,
            )
        };
        if hr < 0 || list.is_null() {
            log::warn!("no taskbar list: {hr:#x}");
            return None;
        }
        let taskbar = Self {
            list,
            hwnd,
            icons: [0; 4],
            made_for: (0, crate::i18n::Lang::En),
            ready: false,
            buttons_added: false,
            shown: None,
        };
        unsafe { (taskbar.vtbl().hr_init)(list) };
        Some(taskbar)
    }

    fn vtbl(&self) -> &Vtbl {
        unsafe { &**(self.list as *const *const Vtbl) }
    }

    /// The window's taskbar button shown or taken away (the window
    /// minimized to the tray).
    pub fn set_tab(&self, shown: bool) {
        let vtbl = self.vtbl();
        // SAFETY: the list and the app's own window.
        unsafe {
            if shown {
                (vtbl.add_tab)(self.list, self.hwnd);
            } else {
                (vtbl.delete_tab)(self.list, self.hwnd);
            }
        }
    }

    /// The taskbar button now exists (again, after Explorer restarted):
    /// the thumbnail buttons are to be added.
    pub fn button_created(&mut self) {
        self.ready = true;
        self.buttons_added = false;
        self.shown = None;
    }

    /// Show `shown`, doing only what changed.
    pub fn show(&mut self, shown: Shown) {
        if !self.ready {
            return;
        }
        let made_for = (small_icon_size(self.hwnd), crate::i18n::lang());
        let remake = made_for != self.made_for;
        if self.shown == Some(shown) && self.buttons_added && !remake {
            return;
        }
        if remake {
            self.made_for = made_for;
            self.destroy_icons();
        }
        let vtbl = self.vtbl();
        let old = self.shown;
        if old.map(|o| (o.progress, o.paused)) != Some((shown.progress, shown.paused)) {
            unsafe {
                match shown.progress {
                    Some(per_mille) => {
                        (vtbl.set_progress_state)(
                            self.list,
                            self.hwnd,
                            if shown.paused {
                                TBPF_PAUSED
                            } else {
                                TBPF_NORMAL
                            },
                        );
                        (vtbl.set_progress_value)(self.list, self.hwnd, per_mille, 1000);
                    }
                    None => {
                        (vtbl.set_progress_state)(self.list, self.hwnd, TBPF_NOPROGRESS);
                    }
                }
            }
        }
        let buttons_changed = old.is_none_or(|o| {
            (o.paused, o.has_previous, o.has_next)
                != (shown.paused, shown.has_previous, shown.has_next)
        });
        if !self.buttons_added || buttons_changed || remake {
            self.set_buttons(&shown);
        }
        self.shown = Some(shown);
    }

    fn destroy_icons(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon;
        for icon in std::mem::take(&mut self.icons)
            .into_iter()
            .filter(|i| *i != 0)
        {
            unsafe { DestroyIcon(icon as _) };
        }
    }

    fn set_buttons(&mut self, shown: &Shown) {
        if self.icons[0] == 0 {
            let size = self.made_for.0;
            self.icons = [Glyph::Play, Glyph::Pause, Glyph::Previous, Glyph::Next]
                .map(|g| make_icon(g, size));
        }
        let button = |id: u16, icon: isize, tip: &str, enabled: bool| {
            let mut b = ThumbButton {
                mask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                id: u32::from(id),
                bitmap: 0,
                icon,
                tip: [0; 260],
                flags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
            };
            for (dst, src) in b.tip.iter_mut().zip(tip.encode_utf16().take(259)) {
                *dst = src;
            }
            b
        };
        let [play, pause, previous, next] = self.icons;
        let buttons = [
            button(
                PREVIOUS,
                previous,
                tr!("Previous track", "Предыдущий трек"),
                shown.has_previous,
            ),
            if shown.paused {
                button(PLAY_PAUSE, play, tr!("Play", "Воспроизвести"), true)
            } else {
                button(PLAY_PAUSE, pause, tr!("Pause", "Пауза"), true)
            },
            button(
                NEXT,
                next,
                tr!("Next track", "Следующий трек"),
                shown.has_next,
            ),
        ];
        let vtbl = self.vtbl();
        let hr = unsafe {
            if self.buttons_added {
                (vtbl.thumb_bar_update_buttons)(self.list, self.hwnd, 3, buttons.as_ptr())
            } else {
                (vtbl.thumb_bar_add_buttons)(self.list, self.hwnd, 3, buttons.as_ptr())
            }
        };
        if hr >= 0 {
            if !self.buttons_added {
                log::debug!("thumbnail buttons added");
            }
            self.buttons_added = true;
        } else {
            // Not again every frame: until Windows makes the button anew.
            log::warn!("thumbnail buttons: {hr:#x}");
            self.ready = false;
        }
    }
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        self.destroy_icons();
        unsafe { (self.vtbl().release)(self.list) };
    }
}

/// The side of a small icon at the window's DPI.
fn small_icon_size(hwnd: isize) -> i32 {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDpiForWindow(hwnd: isize) -> u32;
        fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    }
    const SM_CXSMICON: i32 = 49;
    let dpi = match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi,
    };
    match unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) } {
        n if n > 0 => n,
        _ => 16,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glyph {
    Play,
    Pause,
    Previous,
    Next,
}

/// Whether the point (x, y) of the square -1..1 is inside `glyph`.
fn inside(glyph: Glyph, x: f32, y: f32) -> bool {
    // A triangle pointing right from x0 to x1, half `h` high.
    let right = |x: f32, y: f32, x0: f32, x1: f32, h: f32| {
        x >= x0 && x <= x1 && y.abs() <= h * (x1 - x) / (x1 - x0)
    };
    let rect = |x: f32, y: f32, x0: f32, x1: f32, h: f32| x >= x0 && x <= x1 && y.abs() <= h;
    match glyph {
        Glyph::Play => right(x, y, -0.55, 0.75, 0.75),
        Glyph::Pause => rect(x, y, -0.6, -0.15, 0.7) || rect(x, y, 0.15, 0.6, 0.7),
        Glyph::Next => right(x, y, -0.7, 0.4, 0.7) || rect(x, y, 0.45, 0.7, 0.7),
        Glyph::Previous => inside(Glyph::Next, -x, y),
    }
}

/// The white glyph on a transparent ground, `size` pixels square, as BGRA
/// rows from the top, its edges smoothed (4×4 samples a pixel).
fn glyph_pixels(glyph: Glyph, size: i32) -> Vec<u8> {
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for py in 0..n {
        for px in 0..n {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = ((px as f32 + (sx as f32 + 0.5) / 4.0) / n as f32) * 2.0 - 1.0;
                    let y = ((py as f32 + (sy as f32 + 0.5) / 4.0) / n as f32) * 2.0 - 1.0;
                    hits += u32::from(inside(glyph, x, y));
                }
            }
            let i = (py * n + px) * 4;
            out[i..i + 3].fill(255);
            out[i + 3] = (hits * 255 / 16) as u8;
        }
    }
    out
}

/// An icon of `glyph` (0 if Windows would not make it).
fn make_icon(glyph: Glyph, size: i32) -> isize {
    use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
    let color_bits = glyph_pixels(glyph, size);
    let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    unsafe {
        let color = CreateBitmap(size, size, 1, 32, color_bits.as_ptr().cast());
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon as isize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_are_drawn() {
        for g in [Glyph::Play, Glyph::Pause, Glyph::Previous, Glyph::Next] {
            let px = glyph_pixels(g, 16);
            let covered = px.chunks(4).filter(|p| p[3] > 128).count();
            assert!((40..200).contains(&covered), "{g:?}: {covered}");
        }
        // Previous mirrors Next.
        assert!(inside(Glyph::Next, 0.6, 0.0) && inside(Glyph::Previous, -0.6, 0.0));
    }

    #[test]
    fn thumb_button_layout() {
        // THUMBBUTTON on x64: 3 × u32, padding, HICON, WCHAR[260], DWORD.
        assert_eq!(size_of::<ThumbButton>(), 552);
    }
}
