//! The icon in the notification area ("Minimize to the tray", a setting):
//! there while the setting is on; the window minimized leaves the taskbar
//! (`Taskbar::set_tab`) and stays minimized, not hidden (a hidden window
//! is not closed by `WM_CLOSE`, which the installer, `build.py` and
//! `taskkill` send). A click on the icon brings the window back; its menu
//! (the right button) has Show, Play/Pause, Stop, Previous, Next and Exit.
//! The messages come to the window's subclass (`win::window_proc`), which
//! shows the menu and queues what was chosen (`win::Hooked::Tray`).

use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};

/// The message the icon's events come with (`WM_APP` + 0x51).
pub const WM_TRAY: u32 = 0x8000 + 0x51;

/// What the icon's click or menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCmd {
    Show,
    PlayPause,
    Stop,
    Previous,
    Next,
    Exit,
}

impl TrayCmd {
    /// The menu's items, in its order (a separator before Exit).
    pub const MENU: [TrayCmd; 6] = [
        TrayCmd::Show,
        TrayCmd::PlayPause,
        TrayCmd::Stop,
        TrayCmd::Previous,
        TrayCmd::Next,
        TrayCmd::Exit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TrayCmd::Show => tr!("Show MetalAMP", "Показать MetalAMP"),
            TrayCmd::PlayPause => tr!("Play / Pause", "Воспроизведение / пауза"),
            TrayCmd::Stop => tr!("Stop", "Стоп"),
            TrayCmd::Previous => tr!("Previous", "Предыдущий"),
            TrayCmd::Next => tr!("Next", "Следующий"),
            TrayCmd::Exit => tr!("Exit", "Выход"),
        }
    }

    /// The menu item's id (1-based), and back.
    pub fn id(self) -> u32 {
        Self::MENU
            .iter()
            .position(|&c| c == self)
            .map_or(0, |i| i as u32 + 1)
    }

    pub fn of_id(id: u32) -> Option<TrayCmd> {
        Self::MENU.get((id as usize).checked_sub(1)?).copied()
    }
}

pub struct Tray {
    hwnd: isize,
    icon: isize,
    tip: String,
}

impl Tray {
    /// The icon added for `hwnd`; `None` if the shell refused it.
    pub fn new(hwnd: isize) -> Option<Tray> {
        let mut tray = Tray {
            hwnd,
            icon: load_icon(),
            tip: "MetalAMP".to_owned(),
        };
        tray.add().then_some(tray)
    }

    fn data(&self) -> NOTIFYICONDATAW {
        // SAFETY: a plain struct, all zeros a valid start.
        let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = self.hwnd as _;
        d.uID = 1;
        d
    }

    /// Adds the icon (again, after Explorer restarted); whether it worked.
    pub fn add(&mut self) -> bool {
        let mut d = self.data();
        d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = self.icon as _;
        put_tip(&mut d, &self.tip);
        // SAFETY: a filled NOTIFYICONDATAW of this window.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &d) } != 0;
        if added {
            d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            unsafe { Shell_NotifyIconW(NIM_SETVERSION, &d) };
        }
        added
    }

    /// The tooltip: what plays, or the name.
    pub fn set_tip(&mut self, tip: &str) {
        if tip == self.tip {
            return;
        }
        self.tip = tip.to_owned();
        let mut d = self.data();
        d.uFlags = NIF_TIP | NIF_SHOWTIP;
        put_tip(&mut d, tip);
        // SAFETY: as in `add`.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let d = self.data();
        // SAFETY: the icon of this window, removed once.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
        if self.icon != 0 {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(self.icon as _) };
        }
    }
}

/// `tip` in the struct's 128 UTF-16 units, cut and NUL-ended.
fn put_tip(d: &mut NOTIFYICONDATAW, tip: &str) {
    let wide: Vec<u16> = tip.encode_utf16().take(d.szTip.len() - 1).collect();
    d.szTip.fill(0);
    d.szTip[..wide.len()].copy_from_slice(&wide);
}

/// The exe's icon (resource 1) at the small icon size.
fn load_icon() -> isize {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXSMICON, SM_CYSMICON,
    };
    // SAFETY: the exe's own module and resource; a missing one is null.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        LoadImageW(
            module,
            1 as _,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        ) as isize
    }
}

/// The icon's menu at the pointer (`x`, `y` in screen pixels) for
/// `hwnd`, which is made the foreground window first, so the menu closes
/// when clicked away from; the command chosen.
pub fn menu(hwnd: isize, x: i32, y: i32) -> Option<TrayCmd> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, MF_SEPARATOR, MF_STRING, PostMessageW,
        SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
        WM_NULL,
    };
    // SAFETY: a menu made, shown and destroyed here, on the window's
    // thread (this runs in its window procedure).
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for cmd in TrayCmd::MENU {
            if cmd == TrayCmd::Exit || cmd == TrayCmd::PlayPause {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            }
            let label: Vec<u16> = cmd.label().encode_utf16().chain(Some(0)).collect();
            AppendMenuW(menu, MF_STRING, cmd.id() as usize, label.as_ptr());
        }
        SetForegroundWindow(hwnd as _);
        let chosen = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            x,
            y,
            hwnd as _,
            std::ptr::null(),
        );
        PostMessageW(hwnd as _, WM_NULL, 0, 0);
        DestroyMenu(menu);
        TrayCmd::of_id(chosen as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip() {
        for cmd in TrayCmd::MENU {
            assert_eq!(TrayCmd::of_id(cmd.id()), Some(cmd));
        }
        assert_eq!(TrayCmd::of_id(0), None);
        assert_eq!(TrayCmd::of_id(99), None);
    }
}
