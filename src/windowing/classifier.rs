//! Window classification (plan 6.2 basics).

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, IsIconic,
    IsWindow, IsWindowVisible, IsZoomed, GWL_EXSTYLE, GWL_STYLE, GW_OWNER, WS_CAPTION,
    WS_CHILD, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Maximized,
    Fullscreen,
    Minimized,
    Hidden,
    Destroyed,
}

pub fn window_state(hwnd: HWND) -> State {
    unsafe {
        if IsWindow(hwnd) == 0 {
            return State::Destroyed;
        }
        if IsIconic(hwnd) != 0 {
            return State::Minimized;
        }
        if IsWindowVisible(hwnd) == 0 {
            return State::Hidden;
        }
        if is_fullscreen(hwnd) {
            return State::Fullscreen;
        }
        if IsZoomed(hwnd) != 0 {
            return State::Maximized;
        }
        State::Normal
    }
}

unsafe fn is_fullscreen(hwnd: HWND) -> bool {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if style & WS_CAPTION == WS_CAPTION {
        return false;
    }
    let mut r: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return false;
    }
    let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(mon, &mut mi) == 0 {
        return false;
    }
    let m = mi.rcMonitor;
    r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

pub fn is_candidate(hwnd: HWND) -> bool {
    unsafe {
        if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 {
            return false;
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_CHILD != 0 {
            return false;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW != 0 {
            return false;
        }
        if !GetWindow(hwnd, GW_OWNER).is_null() && ex & WS_EX_APPWINDOW == 0 {
            return false;
        }
    }
    let class = class_name(hwnd);
    !SHELL_CLASSES.contains(&class.as_str())
}
