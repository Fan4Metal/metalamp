//! Explorer's way to hand a player a selection of many files: the open
//! verb's `DropTarget` (`assoc.rs` writes it, with this class as a local
//! server, `CLSID\{…}\LocalServer32` the exe). With a command alone
//! Explorer runs it once per file and only for up to 15 files; with more
//! it left MetalAMP out and played them in Windows Media Player, whose Play
//! verb takes any number. With the drop target Explorer makes the class's
//! object out of process and calls `IDropTarget::Drop` once with all the
//! files: the copy running takes them (`serve`, the class registered on
//! the UI thread, whose message loop COM's calls come through), else COM
//! starts one with `-Embedding`, which registers it as it starts.

use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER, POINTL};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoRegisterClassObject, CoRevokeClassObject, DVASPECT_CONTENT, FORMATETC,
    IClassFactory, IClassFactory_Impl, IDataObject, REGCLS_MULTIPLEUSE, TYMED_HGLOBAL,
};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
    ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::{BOOL, GUID, IUnknown, Interface, Ref, implement};

/// The class Explorer asks for (`assoc.rs` registers it).
pub const CLSID: GUID = GUID::from_u128(0xab084876_b527_4e83_9c34_7b30eb91a1ff);
/// The same as the registry writes it.
pub const CLSID_TEXT: &str = "{AB084876-B527-4E83-9C34-7B30EB91A1FF}";

/// The class's registration with COM while this copy takes the files.
static COOKIE: Mutex<Option<u32>> = Mutex::new(None);

/// This copy takes Explorer's files from now on: called on the UI thread
/// (COM is up there, winit's `OleInitialize`), only by the copy the file
/// types are registered for (a development build running beside the
/// installed one would take its files).
pub fn serve() {
    let factory: IClassFactory = Factory.into();
    match unsafe { CoRegisterClassObject(&CLSID, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE) }
    {
        Ok(cookie) => {
            if let Ok(mut kept) = COOKIE.lock() {
                *kept = Some(cookie);
            }
        }
        Err(e) => log::warn!("cannot take Explorer's files: {e}"),
    }
}

/// No more files: the player is closing (COM starts another copy for the
/// next ones).
pub fn revoke() {
    let cookie = COOKIE.lock().ok().and_then(|mut c| c.take());
    if let Some(cookie) = cookie {
        let _ = unsafe { CoRevokeClassObject(cookie) };
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        out: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = std::ptr::null_mut() };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let target: IDropTarget = Target.into();
        unsafe { target.query(iid, out).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IDropTarget)]
struct Target;

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        _data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        set(effect, DROPEFFECT_COPY);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        set(effect, DROPEFFECT_COPY);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let mut paths = data.as_ref().map(paths_of).unwrap_or_default();
        log::debug!("Explorer handed over {} files", paths.len());
        // As the folder shows them, sorted by name: Explorer lists the
        // focused file first (the last clicked of a Shift range), and the
        // playlist starts with the first file.
        paths.sort_by(|a, b| {
            crate::format::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())
        });
        let taken = !paths.is_empty()
            && crate::instance::hand_over(paths).is_some_and(|main| {
                crate::win::bring_back(main);
                true
            });
        set(effect, if taken { DROPEFFECT_COPY } else { DROPEFFECT_NONE });
        Ok(())
    }
}

fn set(effect: *mut DROPEFFECT, value: DROPEFFECT) {
    if !effect.is_null() {
        unsafe { *effect = value };
    }
}

/// The files of a data object (CF_HDROP; Explorer's items outside the file
/// system have none).
fn paths_of(data: &IDataObject) -> Vec<PathBuf> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else {
        return Vec::new();
    };
    let drop = HDROP(unsafe { medium.u.hGlobal.0 });
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = unsafe { DragQueryFileW(drop, i, None) } as usize;
        let mut buf = vec![0u16; len + 1];
        let got = unsafe { DragQueryFileW(drop, i, Some(&mut buf)) } as usize;
        if got > 0 {
            use std::os::windows::ffi::OsStringExt;
            paths.push(PathBuf::from(std::ffi::OsString::from_wide(&buf[..got])));
        }
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
        STGMEDIUM, STGMEDIUM_0,
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut core::ffi::c_void;
        fn GlobalLock(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GlobalUnlock(memory: *mut core::ffi::c_void) -> i32;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHCreateDataObject(
            folder: *const core::ffi::c_void,
            count: u32,
            items: *const *const core::ffi::c_void,
            inner: *mut core::ffi::c_void,
            iid: *const GUID,
            out: *mut *mut core::ffi::c_void,
        ) -> windows::core::HRESULT;
    }

    /// A data object holding `paths` as Explorer's CF_HDROP: a DROPFILES
    /// header, then the paths as UTF-16, each ended by a NUL, and a NUL.
    fn data_of(paths: &[&str]) -> IDataObject {
        let mut text: Vec<u16> = Vec::new();
        for p in paths {
            text.extend(p.encode_utf16());
            text.push(0);
        }
        text.push(0);
        // DROPFILES: pFiles, pt (x, y), fNC, fWide.
        let header = 20usize;
        let bytes = header + text.len() * 2;
        unsafe {
            let memory = GlobalAlloc(0x0042, bytes); // GHND: movable, zeroed
            assert!(!memory.is_null());
            let at = GlobalLock(memory) as *mut u8;
            (at as *mut u32).write_unaligned(header as u32);
            (at.add(16) as *mut u32).write_unaligned(1);
            std::ptr::copy_nonoverlapping(text.as_ptr() as *const u8, at.add(header), text.len() * 2);
            GlobalUnlock(memory);
            let mut raw = std::ptr::null_mut();
            SHCreateDataObject(
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
                &IDataObject::IID,
                &mut raw,
            )
            .unwrap();
            let data = IDataObject::from_raw(raw);
            let format = FORMATETC {
                cfFormat: CF_HDROP.0,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            };
            let medium = STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 {
                    hGlobal: HGLOBAL(memory),
                },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            };
            data.SetData(&format, &medium, true).unwrap();
            data
        }
    }

    #[test]
    fn the_files_come_out_of_explorers_data_object() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let paths = [r"D:\Music\Группа\01 - Первая.mp3", r"D:\Music\Группа\02 - Вторая.mp3"];
        let data = data_of(&paths);
        let got = paths_of(&data);
        assert_eq!(got, paths.map(PathBuf::from));
    }

    /// `cargo test explorer_reaches_the_drop_target -- --ignored`: the
    /// class served by a thread of this process, made through COM as
    /// Explorer makes it (out of process, `CLSCTX_LOCAL_SERVER`), its
    /// calls coming through proxies; no files are handed over (the app is
    /// not running), so Drop takes none.
    #[test]
    #[ignore = "registers MetalAMP's class with COM for a moment"]
    fn explorer_reaches_the_drop_target() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let (up, ready) = std::sync::mpsc::channel();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.unwrap();
            serve();
            up.send(()).unwrap();
            while stopped.try_recv().is_err() {
                unsafe {
                    let mut msg: MSG = std::mem::zeroed();
                    while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            revoke();
        });
        ready.recv().unwrap();
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let target: IDropTarget =
            unsafe { CoCreateInstance(&CLSID, None, CLSCTX_LOCAL_SERVER) }.unwrap();
        let data = data_of(&[r"C:\nothing.mp3"]);
        let keys = MODIFIERKEYS_FLAGS(0);
        let mut effect = DROPEFFECT_NONE;
        unsafe { target.DragEnter(&data, keys, POINTL::default(), &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_COPY);
        unsafe { target.Drop(&data, keys, POINTL::default(), &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_NONE);
        drop(target);
        stop.send(()).unwrap();
        server.join().unwrap();
    }
}
