//! COM and Media Foundation for the audio thread (qrec's `win::com_init_mta`
//! and `encoder::Mf`), and Windows errors as text.

use windows::Win32::Media::MediaFoundation::{
    MF_API_VERSION, MF_SDK_VERSION, MFSTARTUP_FULL, MFShutdown, MFStartup,
};
use windows::core::Result;

/// COM in the multithreaded apartment for this thread, until dropped.
pub struct Com {
    initialised: bool,
}

pub fn com_init_mta() -> Com {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    Com {
        initialised: hr.is_ok(),
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.initialised {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// Media Foundation started, until dropped. Whatever holds MF objects must
/// be dropped first: keep this the first local of the thread (locals drop
/// in reverse), or the last field of a struct.
pub struct Mf;

impl Mf {
    pub fn start() -> Result<Mf> {
        unsafe { MFStartup((MF_SDK_VERSION << 16) | MF_API_VERSION, MFSTARTUP_FULL)? };
        Ok(Mf)
    }
}

impl Drop for Mf {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// A Windows error for the log and the window: its message and code.
pub fn describe(e: &windows::core::Error) -> String {
    let msg = e.message();
    let msg = msg.trim();
    if msg.is_empty() {
        format!("{:#x}", e.code().0)
    } else {
        format!("{msg} ({:#x})", e.code().0)
    }
}
