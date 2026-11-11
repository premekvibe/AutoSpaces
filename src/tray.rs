//! Tray icon and context menu.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, PostMessageW,
    SetForegroundWindow, TrackPopupMenu, IDI_APPLICATION, MF_CHECKED, MF_STRING,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_NULL,
};

pub const WM_TRAY: u32 = WM_APP + 1;
pub const CMD_TOGGLE: usize = 1;
pub const CMD_EXIT: usize = 2;

/// "Zapnuto" toggle, in memory only.
pub static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
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
    nid.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
    for (d, s) in nid.szTip.iter_mut().zip("AutoSpaces".encode_utf16()) {
        *d = s;
    }
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

pub fn remove(hwnd: HWND) {
    let nid = data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}

/// Shows the popup menu at the cursor; returns the chosen command (0 = none).
pub fn show_menu(hwnd: HWND) -> usize {
    unsafe {
        let menu = CreatePopupMenu();
        let on = if ENABLED.load(Relaxed) { MF_CHECKED } else { 0 };
        AppendMenuW(menu, MF_STRING | on, CMD_TOGGLE, wide("Zapnuto").as_ptr());
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
