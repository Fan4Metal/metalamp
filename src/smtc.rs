//! Windows' media controls (`SystemMediaTransportControls`): the media keys
//! of keyboards, headsets and remotes reach the player when its window is
//! not in front, and Windows' media flyout (over the volume's) shows what
//! plays with Play/Pause, Previous and Next. WinRT declared by hand, as
//! `taskbar.rs` does COM (the interfaces' GUIDs and the methods' order from
//! the SDK's `windows.media.h`): `ISystemMediaTransportControlsInterop::
//! GetForWindow` gives the window's controls; a button pressed comes on a
//! thread of Windows' and is queued as `win::Hooked::Media` (metalmpv's;
//! here every track is shown as music).

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::core::GUID;

type HString = *mut c_void;

const IID_UNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_AGILE_OBJECT: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);
const IID_INTEROP: GUID = GUID::from_u128(0xddb0472d_c911_4a1f_86d9_dc3d71a95f5a);
const IID_CONTROLS: GUID = GUID::from_u128(0x99fa3ff4_1742_42a6_902e_087d41f965ec);
/// `TypedEventHandler<SystemMediaTransportControls,
/// SystemMediaTransportControlsButtonPressedEventArgs>`.
const IID_BUTTON_HANDLER: GUID = GUID::from_u128(0x0557e996_7b23_5bae_aa81_ea0d671143a4);

// MediaPlaybackStatus
const STATUS_CLOSED: i32 = 0;
const STATUS_STOPPED: i32 = 2;
const STATUS_PLAYING: i32 = 3;
const STATUS_PAUSED: i32 = 4;
// MediaPlaybackType
const TYPE_MUSIC: i32 = 1;
const TYPE_VIDEO: i32 = 2;
// SystemMediaTransportControlsButton
const BUTTON_PLAY: i32 = 0;
const BUTTON_PAUSE: i32 = 1;
const BUTTON_STOP: i32 = 2;
const BUTTON_NEXT: i32 = 6;
const BUTTON_PREVIOUS: i32 = 7;

const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;

type Put<T> = unsafe extern "system" fn(*mut c_void, T) -> i32;
type Get = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;

/// `IInspectable`'s methods, those not used as `usize`.
#[repr(C)]
struct Inspectable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_iids: usize,
    get_runtime_class_name: usize,
    get_trust_level: usize,
}

#[repr(C)]
struct InteropVtbl {
    base: Inspectable,
    get_for_window:
        unsafe extern "system" fn(*mut c_void, isize, *const GUID, *mut *mut c_void) -> i32,
}

/// `ISystemMediaTransportControls`; each flag a getter and a setter.
#[repr(C)]
struct ControlsVtbl {
    base: Inspectable,
    get_playback_status: usize,
    put_playback_status: Put<i32>,
    get_display_updater: Get,
    get_sound_level: usize,
    get_is_enabled: usize,
    put_is_enabled: Put<u8>,
    get_is_play_enabled: usize,
    put_is_play_enabled: Put<u8>,
    get_is_stop_enabled: usize,
    put_is_stop_enabled: Put<u8>,
    get_is_pause_enabled: usize,
    put_is_pause_enabled: Put<u8>,
    record: [usize; 2],
    fast_forward: [usize; 2],
    rewind: [usize; 2],
    get_is_previous_enabled: usize,
    put_is_previous_enabled: Put<u8>,
    get_is_next_enabled: usize,
    put_is_next_enabled: Put<u8>,
    channel_up: [usize; 2],
    channel_down: [usize; 2],
    add_button_pressed: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> i32,
    remove_button_pressed: Put<i64>,
}

/// `ISystemMediaTransportControlsDisplayUpdater`.
#[repr(C)]
struct UpdaterVtbl {
    base: Inspectable,
    get_type: usize,
    put_type: Put<i32>,
    app_media_id: [usize; 2],
    thumbnail: [usize; 2],
    get_music_properties: Get,
    get_video_properties: Get,
    get_image_properties: usize,
    copy_from_file_async: usize,
    clear_all: unsafe extern "system" fn(*mut c_void) -> i32,
    update: unsafe extern "system" fn(*mut c_void) -> i32,
}

/// `IMusicDisplayProperties` and `IVideoDisplayProperties` begin alike:
/// the title, then the album artist or the subtitle, then the artist.
#[repr(C)]
struct PropertiesVtbl {
    base: Inspectable,
    get_title: usize,
    put_title: Put<HString>,
    get_second: usize,
    put_second: Put<HString>,
    get_artist: usize,
    put_artist: Put<HString>,
}

/// `ISystemMediaTransportControlsButtonPressedEventArgs`.
#[repr(C)]
struct ArgsVtbl {
    base: Inspectable,
    get_button: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
}

// raw-dylib: no import library needed (the SDK has none named combase).
#[link(name = "combase", kind = "raw-dylib")]
unsafe extern "system" {
    fn RoGetActivationFactory(class: HString, iid: *const GUID, factory: *mut *mut c_void) -> i32;
    fn WindowsCreateString(text: *const u16, length: u32, out: *mut HString) -> i32;
    fn WindowsDeleteString(text: HString) -> i32;
}

/// The vtable of a WinRT object.
///
/// # Safety
/// `object` is a live object whose vtable is a `T`.
unsafe fn vtbl<'a, T>(object: *mut c_void) -> &'a T {
    unsafe { &**(object as *const *const T) }
}

/// Release any WinRT object.
fn release(object: *mut c_void) {
    if !object.is_null() {
        unsafe { (vtbl::<Inspectable>(object).release)(object) };
    }
}

/// An HSTRING of `text`, deleted when dropped.
struct HStr(HString);

impl HStr {
    fn new(text: &str) -> Option<Self> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut s = std::ptr::null_mut();
        let hr = unsafe { WindowsCreateString(wide.as_ptr(), wide.len() as u32, &mut s) };
        (hr >= 0).then_some(Self(s))
    }
}

impl Drop for HStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WindowsDeleteString(self.0) };
        }
    }
}

/// What the media controls show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub status: Status,
    pub has_previous: bool,
    pub has_next: bool,
    /// The title, and the artist of a song.
    pub title: String,
    pub artist: Option<String>,
    /// Sound without a picture: shown as music.
    pub music: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing to play: the flyout goes.
    #[default]
    Closed,
    Stopped,
    Playing,
    Paused,
}

pub struct Smtc {
    controls: *mut c_void,
    /// The ButtonPressed handler's registration.
    token: i64,
    shown: Option<Shown>,
}

impl Smtc {
    /// The media controls of window `hwnd`, with Play, Pause and Stop on;
    /// `None` where Windows has none (before Windows 8.1, a server core).
    pub fn new(hwnd: isize) -> Option<Self> {
        let class = HStr::new("Windows.Media.SystemMediaTransportControls")?;
        let mut interop = std::ptr::null_mut();
        let hr = unsafe { RoGetActivationFactory(class.0, &IID_INTEROP, &mut interop) };
        if hr < 0 || interop.is_null() {
            log::info!("no media controls: {hr:#x}");
            return None;
        }
        let mut controls = std::ptr::null_mut();
        let hr = unsafe {
            (vtbl::<InteropVtbl>(interop).get_for_window)(
                interop,
                hwnd,
                &IID_CONTROLS,
                &mut controls,
            )
        };
        release(interop);
        if hr < 0 || controls.is_null() {
            log::info!("no media controls for the window: {hr:#x}");
            return None;
        }
        let mut smtc = Self {
            controls,
            token: 0,
            shown: None,
        };
        let v = smtc.vtbl();
        unsafe {
            (v.put_is_enabled)(controls, 1);
            (v.put_is_play_enabled)(controls, 1);
            (v.put_is_pause_enabled)(controls, 1);
            (v.put_is_stop_enabled)(controls, 1);
        }
        let handler = Handler::create();
        let hr = unsafe { (v.add_button_pressed)(controls, handler, &mut smtc.token) };
        // The controls hold their own reference.
        release(handler);
        if hr < 0 {
            log::warn!("media controls: no button handler: {hr:#x}");
            return None;
        }
        Some(smtc)
    }

    fn vtbl(&self) -> &ControlsVtbl {
        unsafe { vtbl(self.controls) }
    }

    /// Show `shown`, doing only what changed.
    pub fn show(&mut self, shown: Shown) {
        if self.shown.as_ref() == Some(&shown) {
            return;
        }
        // Everything is set the first time.
        let old = self.shown.take();
        let changed = |f: &dyn Fn(&Shown) -> bool| old.as_ref().is_none_or(f);
        let (c, v) = (self.controls, self.vtbl());
        unsafe {
            if changed(&|o| o.status != shown.status) {
                let status = match shown.status {
                    Status::Closed => STATUS_CLOSED,
                    Status::Stopped => STATUS_STOPPED,
                    Status::Playing => STATUS_PLAYING,
                    Status::Paused => STATUS_PAUSED,
                };
                (v.put_playback_status)(c, status);
            }
            if changed(&|o| o.has_previous != shown.has_previous) {
                (v.put_is_previous_enabled)(c, shown.has_previous.into());
            }
            if changed(&|o| o.has_next != shown.has_next) {
                (v.put_is_next_enabled)(c, shown.has_next.into());
            }
        }
        if changed(&|o| {
            (&o.title, &o.artist, o.music) != (&shown.title, &shown.artist, shown.music)
        }) {
            self.display(&shown);
        }
        self.shown = Some(shown);
    }

    /// The title (and the artist of a song) in Windows' flyout.
    fn display(&self, shown: &Shown) {
        let mut updater = std::ptr::null_mut();
        let hr = unsafe { (self.vtbl().get_display_updater)(self.controls, &mut updater) };
        if hr < 0 || updater.is_null() {
            return;
        }
        let u = unsafe { vtbl::<UpdaterVtbl>(updater) };
        unsafe { (u.clear_all)(updater) };
        let (kind, get) = if shown.music {
            (TYPE_MUSIC, u.get_music_properties)
        } else {
            (TYPE_VIDEO, u.get_video_properties)
        };
        let mut props = std::ptr::null_mut();
        if unsafe { (u.put_type)(updater, kind) } >= 0
            && unsafe { get(updater, &mut props) } >= 0
            && !props.is_null()
        {
            let p = unsafe { vtbl::<PropertiesVtbl>(props) };
            if let Some(title) = HStr::new(&shown.title) {
                unsafe { (p.put_title)(props, title.0) };
            }
            if shown.music
                && let Some(artist) = shown.artist.as_deref().and_then(HStr::new)
            {
                unsafe { (p.put_artist)(props, artist.0) };
            }
            release(props);
        }
        unsafe { (u.update)(updater) };
        release(updater);
    }
}

impl Drop for Smtc {
    /// The controls are the window's, not freed with this: left enabled
    /// and playing, Windows kept the player as the session the media keys
    /// and its flyout go to, with no handler (the setting turned off).
    fn drop(&mut self) {
        let (c, v) = (self.controls, self.vtbl());
        unsafe {
            (v.put_playback_status)(c, STATUS_CLOSED);
            let mut updater = std::ptr::null_mut();
            if (v.get_display_updater)(c, &mut updater) >= 0 && !updater.is_null() {
                let u = vtbl::<UpdaterVtbl>(updater);
                (u.clear_all)(updater);
                (u.update)(updater);
                release(updater);
            }
            (v.put_is_enabled)(c, 0);
            if self.token != 0 {
                (v.remove_button_pressed)(c, self.token);
            }
        }
        release(self.controls);
    }
}

/// The ButtonPressed handler: a COM object of its own, called on a thread
/// of Windows'.
#[repr(C)]
struct Handler {
    vtbl: &'static HandlerVtbl,
    refs: AtomicU32,
}

#[repr(C)]
struct HandlerVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    invoke: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
}

static HANDLER_VTBL: HandlerVtbl = HandlerVtbl {
    query_interface: handler_query_interface,
    add_ref: handler_add_ref,
    release: handler_release,
    invoke: handler_invoke,
};

impl Handler {
    /// A new handler with one reference, the caller's.
    fn create() -> *mut c_void {
        Box::into_raw(Box::new(Handler {
            vtbl: &HANDLER_VTBL,
            refs: AtomicU32::new(1),
        })) as *mut c_void
    }
}

fn same(a: &GUID, b: &GUID) -> bool {
    (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
}

unsafe extern "system" fn handler_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_NOINTERFACE;
    }
    let iid = unsafe { &*iid };
    // Agile: Windows calls it from any thread.
    if [IID_UNKNOWN, IID_AGILE_OBJECT, IID_BUTTON_HANDLER]
        .iter()
        .any(|known| same(known, iid))
    {
        unsafe {
            handler_add_ref(this);
            *out = this;
        }
        0
    } else {
        unsafe { *out = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn handler_add_ref(this: *mut c_void) -> u32 {
    let handler = unsafe { &*(this as *const Handler) };
    handler.refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn handler_release(this: *mut c_void) -> u32 {
    let left = {
        let handler = unsafe { &*(this as *const Handler) };
        handler.refs.fetch_sub(1, Ordering::AcqRel) - 1
    };
    if left == 0 {
        drop(unsafe { Box::from_raw(this as *mut Handler) });
    }
    left
}

unsafe extern "system" fn handler_invoke(
    _this: *mut c_void,
    _sender: *mut c_void,
    args: *mut c_void,
) -> i32 {
    if args.is_null() {
        return 0;
    }
    let mut button = -1;
    if unsafe { (vtbl::<ArgsVtbl>(args).get_button)(args, &mut button) } < 0 {
        return 0;
    }
    use crate::win::MediaKey;
    let key = match button {
        BUTTON_PLAY => MediaKey::Play,
        BUTTON_PAUSE => MediaKey::Pause,
        BUTTON_STOP => MediaKey::Stop,
        BUTTON_NEXT => MediaKey::Next,
        BUTTON_PREVIOUS => MediaKey::Previous,
        _ => return 0,
    };
    crate::win::hook_media(key);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handler_counts_its_references() {
        let h = Handler::create();
        let mut out = std::ptr::null_mut();
        unsafe {
            assert_eq!(handler_query_interface(h, &IID_BUTTON_HANDLER, &mut out), 0);
            assert_eq!(out, h);
            assert_eq!(
                handler_query_interface(h, &IID_CONTROLS, &mut out),
                E_NOINTERFACE
            );
            assert!(out.is_null());
            assert_eq!(handler_release(h), 1);
            assert_eq!(handler_release(h), 0);
        }
    }

    /// `cargo test media_controls -- --ignored --nocapture`: the controls
    /// of a hidden window of this process, shown once (Windows' flyout
    /// may appear).
    #[test]
    #[ignore = "uses Windows' media controls"]
    fn media_controls_of_a_window() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WS_OVERLAPPEDWINDOW,
        };
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        let mut smtc = Smtc::new(hwnd as isize).expect("media controls");
        smtc.show(Shown {
            status: Status::Paused,
            has_previous: false,
            has_next: true,
            title: "Test".into(),
            artist: Some("MetalAMP".into()),
            music: true,
        });
        drop(smtc);
        unsafe { DestroyWindow(hwnd) };
    }
}
