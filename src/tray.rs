//! Tray icon and context menu.

use crate::i18n::{t, Key};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, GetSystemMetrics,
    LoadIconW, LoadImageW, PostMessageW, SetForegroundWindow, TrackPopupMenu, HICON,
    IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, SM_CXSMICON,
    SM_CYSMICON, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_NULL,
};

pub const WM_TRAY: u32 = WM_APP + 1;
pub const CMD_TOGGLE: usize = 1;
pub const CMD_EXIT: usize = 2;
pub const CMD_FORGET_ALL: usize = 3;
pub const CMD_SETTINGS: usize = 4;
pub const CMD_ABOUT: usize = 5;
/// Command id of the first learned process in the submenu (index = id - base).
pub const CMD_FORGET_BASE: usize = 1000;

/// "Zapnuto" toggle, in memory only.
pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// Opens the diagnostics written when no desktop backend is available (plan 5.3).
pub const CMD_DIAGNOSTICS: usize = 6;

/// Backend state shown by the icon: 0 = normal, 1 = unsupported Windows, 2 = switched off in settings.
pub const MODE_NORMAL: usize = 0;
pub const MODE_UNSUPPORTED: usize = 1;
pub const MODE_FORCED_OFF: usize = 2;
static MODE: AtomicUsize = AtomicUsize::new(MODE_NORMAL);

fn tip_key() -> Key {
    match MODE.load(Relaxed) {
        MODE_UNSUPPORTED => Key::TrayTipUnsupported,
        MODE_FORCED_OFF => Key::TrayTipForcedOff,
        _ => Key::TrayTip,
    }
}

fn copy_into(dst: &mut [u16], s: &str) {
    let n = dst.len() - 1; // keep the terminating zero
    for (d, c) in dst[..n].iter_mut().zip(s.encode_utf16().chain(std::iter::repeat(0))) {
        *d = c;
    }
}

/// Changes the icon's mode (tooltip; the diagnostics item of the menu).
pub fn set_mode(hwnd: HWND, mode: usize) {
    MODE.store(mode, Relaxed);
    let mut nid = data(hwnd);
    nid.uFlags = NIF_TIP;
    copy_into(&mut nid.szTip, t(tip_key()));
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

pub fn mode() -> usize {
    MODE.load(Relaxed)
}

/// Balloon / toast notification from the icon.
pub fn notify(hwnd: HWND, text: &str) {
    let mut nid = data(hwnd);
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_WARNING;
    copy_into(&mut nid.szInfoTitle, t(Key::AppName));
    copy_into(&mut nid.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Icon handle we own (loaded from the embedded resource); 0 = none.
static OWNED_ICON: AtomicUsize = AtomicUsize::new(0);

/// Loads embedded icon resource 1 at the current small-icon size, or null.
fn load_app_icon() -> HICON {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        LoadImageW(
            hinst,
            std::ptr::without_provenance::<u16>(1), // MAKEINTRESOURCE(1)
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        ) as HICON
    }
}

fn release_icon() {
    let old = OWNED_ICON.swap(0, Relaxed);
    if old != 0 {
        unsafe { DestroyIcon(old as HICON) };
    }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

pub fn add(hwnd: HWND) {
    let mut nid = data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    let icon = load_app_icon();
    nid.hIcon = if icon.is_null() {
        unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) } // shared, not owned
    } else {
        icon
    };
    copy_into(&mut nid.szTip, t(tip_key()));
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    // The shell keeps its own copy; swap out the handle from any earlier add.
    release_icon();
    if !icon.is_null() {
        OWNED_ICON.store(icon as usize, Relaxed);
    }
}

pub fn remove(hwnd: HWND) {
    let nid = data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    release_icon();
}

/// Shows the popup menu at the cursor; returns the chosen command (0 = none).
pub fn show_menu(hwnd: HWND, learned: &[String]) -> usize {
    unsafe {
        let sub = CreatePopupMenu();
        if learned.is_empty() {
            AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, wide(t(Key::MenuNone)).as_ptr());
        }
        for (i, name) in learned.iter().enumerate() {
            AppendMenuW(sub, MF_STRING, CMD_FORGET_BASE + i, wide(name).as_ptr());
        }
        AppendMenuW(sub, MF_SEPARATOR, 0, std::ptr::null());
        let all = if learned.is_empty() { MF_GRAYED } else { 0 };
        AppendMenuW(sub, MF_STRING | all, CMD_FORGET_ALL, wide(t(Key::MenuForgetAll)).as_ptr());
        let menu = CreatePopupMenu();
        let on = if ENABLED.load(Relaxed) { MF_CHECKED } else { 0 };
        AppendMenuW(menu, MF_STRING | on, CMD_TOGGLE, wide(t(Key::MenuEnabled)).as_ptr());
        AppendMenuW(menu, MF_POPUP, sub as usize, wide(t(Key::MenuLearned)).as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_SETTINGS, wide(t(Key::MenuSettings)).as_ptr());
        if MODE.load(Relaxed) == MODE_UNSUPPORTED {
            AppendMenuW(menu, MF_STRING, CMD_DIAGNOSTICS, wide(t(Key::MenuDiagnostics)).as_ptr());
        }
        AppendMenuW(menu, MF_STRING, CMD_ABOUT, wide(t(Key::MenuAbout)).as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_EXIT, wide(t(Key::MenuExit)).as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as usize
    }
}
