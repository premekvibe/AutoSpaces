//! Tray icon and context menu.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
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
/// Command id of the first learned process in the submenu (index = id - base).
pub const CMD_FORGET_BASE: usize = 1000;

/// "Zapnuto" toggle, in memory only.
pub static ENABLED: AtomicBool = AtomicBool::new(true);

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
    for (d, s) in nid.szTip.iter_mut().zip("AutoSpaces".encode_utf16()) {
        *d = s;
    }
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
            AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, wide("(žádné)").as_ptr());
        }
        for (i, name) in learned.iter().enumerate() {
            AppendMenuW(sub, MF_STRING, CMD_FORGET_BASE + i, wide(name).as_ptr());
        }
        AppendMenuW(sub, MF_SEPARATOR, 0, std::ptr::null());
        let all = if learned.is_empty() { MF_GRAYED } else { 0 };
        AppendMenuW(sub, MF_STRING | all, CMD_FORGET_ALL, wide("Zapomenout vše").as_ptr());
        let menu = CreatePopupMenu();
        let on = if ENABLED.load(Relaxed) { MF_CHECKED } else { 0 };
        AppendMenuW(menu, MF_STRING | on, CMD_TOGGLE, wide("Zapnuto").as_ptr());
        AppendMenuW(menu, MF_POPUP, sub as usize, wide("Naučené výjimky").as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_SETTINGS, wide("Nastavení…").as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_EXIT, wide("Ukončit").as_ptr());
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
