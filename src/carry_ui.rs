//! Carrying a window (plan 12.13), the Win32 side: the shortcut (RegisterHotKey), the low-level
//! keyboard hook that exists only while a window is carried, and the hint for a window that cannot\n//! be moved (the row of desktop previews in overlay_ui is the carry's indicator).
//!
//! The hook records and logs nothing: it only looks at the three modifiers (released = drop) and
//! Esc (swallowed, = take the window back); every other key passes untouched.

use crate::tray::wide;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetMonitorInfoW, InvalidateRect, MonitorFromWindow, SelectObject, SetBkMode, SetTextColor, SetWindowRgn,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FW_SEMIBOLD, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LEFT,
    VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RIGHT, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, PostMessageW,
    RegisterClassW, SetLayeredWindowAttributes, SetWindowPos, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx,
    HC_ACTION, HTTRANSPARENT, HWND_TOPMOST, KBDLLHOOKSTRUCT, LLKHF_UP, LWA_ALPHA, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_SHOWNOACTIVATE, WH_KEYBOARD_LL, WM_APP, WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Hotkey ids (RegisterHotKey on the hidden window).
pub const HK_LEFT: i32 = 1;
pub const HK_RIGHT: i32 = 2;
/// Posted by the hook to the hidden window: drop the carried window (`wParam` 1 = Esc, take it back).
pub const WM_CARRY_DROP: u32 = WM_APP + 30;
/// Marks the one key this app injects (to keep the Start menu from opening), so the hook skips it.
const MARK: usize = 0x4153_4352; // "ASCR"
/// An unassigned virtual key: pressed with Win held, it tells the shell "Win was used with a key".
const VK_DUMMY: u16 = 0xFF;

static MAIN: AtomicUsize = AtomicUsize::new(0);
static HOOK: AtomicUsize = AtomicUsize::new(0);
static REGISTERED: AtomicBool = AtomicBool::new(false);
static REGISTER_FAILED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------- shortcut

/// Registers Win+Ctrl+Shift+Left/Right (both or neither). False = another app holds one of them.
pub fn register(main: HWND) -> bool {
    if REGISTERED.load(Relaxed) {
        return true;
    }
    let mods = MOD_WIN | MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
    let ok = unsafe {
        let l = RegisterHotKey(main, HK_LEFT, mods, VK_LEFT as u32) != 0;
        let r = RegisterHotKey(main, HK_RIGHT, mods, VK_RIGHT as u32) != 0;
        if !(l && r) {
            if l {
                UnregisterHotKey(main, HK_LEFT);
            }
            if r {
                UnregisterHotKey(main, HK_RIGHT);
            }
        }
        l && r
    };
    REGISTERED.store(ok, Relaxed);
    REGISTER_FAILED.store(!ok, Relaxed);
    ok
}

pub fn unregister(main: HWND) {
    if REGISTERED.swap(false, Relaxed) {
        unsafe {
            UnregisterHotKey(main, HK_LEFT);
            UnregisterHotKey(main, HK_RIGHT);
        }
    }
    REGISTER_FAILED.store(false, Relaxed);
}

pub fn registered() -> bool {
    REGISTERED.load(Relaxed)
}

/// The last registration attempt failed (shown in the settings dialog).
pub fn register_failed() -> bool {
    REGISTER_FAILED.load(Relaxed)
}

// ---------------------------------------------------------------- keyboard hook (only while carrying)

fn down(vk: u16) -> bool {
    let state = unsafe { GetAsyncKeyState(vk as i32) };
    state < 0
}

/// Win, Ctrl and Shift are all held.
pub fn modifiers_held() -> bool {
    down(VK_CONTROL) && down(VK_SHIFT) && (down(VK_LWIN) || down(VK_RWIN))
}

/// Presses and releases an unassigned key while Win is held, so that releasing Win does not open
/// the Start menu (Esc was swallowed; the arrows went to the shortcut).
pub fn mask_win_key() {
    if !(down(VK_LWIN) || down(VK_RWIN)) {
        return;
    }
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_DUMMY, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: MARK } },
    };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

unsafe extern "system" fn ll_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lp != 0 {
        let k = &*(lp as *const KBDLLHOOKSTRUCT);
        if k.dwExtraInfo != MARK {
            let up = k.flags & LLKHF_UP != 0;
            let main = MAIN.load(Relaxed) as HWND;
            match k.vkCode as u16 {
                VK_ESCAPE => {
                    if !up {
                        PostMessageW(main, WM_CARRY_DROP, 1, 0);
                    }
                    return 1; // Esc belongs to the carry, the foreground app never sees it
                }
                VK_LWIN | VK_RWIN | VK_LCONTROL | VK_RCONTROL | VK_CONTROL | VK_LSHIFT | VK_RSHIFT | VK_SHIFT if up => {
                    PostMessageW(main, WM_CARRY_DROP, 0, 0);
                }
                _ => {}
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wp, lp)
}

/// Installs the hook (once). When the modifiers were released before it existed, the drop is
/// posted at once.
pub fn hook_on(main: HWND) {
    MAIN.store(main as usize, Relaxed);
    if HOOK.load(Relaxed) == 0 {
        let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), GetModuleHandleW(std::ptr::null()), 0) };
        HOOK.store(h as usize, Relaxed);
    }
    if !modifiers_held() {
        unsafe { PostMessageW(main, WM_CARRY_DROP, 0, 0) };
    }
}

pub fn hook_off() {
    let h = HOOK.swap(0, Relaxed);
    if h != 0 {
        unsafe { UnhookWindowsHookEx(h as _) };
    }
}

pub fn hook_installed() -> bool {
    HOOK.load(Relaxed) != 0
}

// ---------------------------------------------------------------- Win+Ctrl+Shift held (Raw Input)

// The preview (plan 12.13) needs to know when Win, Ctrl and Shift are held alone. Raw Input
// (RIDEV_INPUTSINK on the hidden window) delivers a copy of every key event asynchronously, also
// when another app is in front: nothing waits for us (unlike a low-level hook, which every key
// would pass through) and nothing can be blocked or changed – other Win+Ctrl+Shift shortcuts work
// as before. Only the kind of key and up/down are looked at; nothing is stored or logged.

static RAW: AtomicBool = AtomicBool::new(false);

/// Starts receiving keyboard Raw Input on `main` (false = refused).
pub fn raw_on(main: HWND) -> bool {
    use windows_sys::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK};
    if RAW.load(Relaxed) {
        return true;
    }
    let dev = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget: main };
    let ok = unsafe { RegisterRawInputDevices(&dev, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) } != 0;
    RAW.store(ok, Relaxed);
    ok
}

pub fn raw_off() {
    use windows_sys::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_REMOVE};
    if RAW.swap(false, Relaxed) {
        let dev = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_REMOVE, hwndTarget: std::ptr::null_mut() };
        unsafe { RegisterRawInputDevices(&dev, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) };
    }
}

/// The key of a `WM_INPUT` message (`lParam`) as the preview sees it, and whether it went down.
/// None = not a keyboard event, the key this app injects, or a fake Shift of the keyboard driver.
pub fn raw_key(lp: LPARAM) -> Option<(crate::core::carry::ChordKey, bool)> {
    use crate::core::carry::ChordKey;
    use windows_sys::Win32::UI::Input::{GetRawInputData, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEKEYBOARD};
    use windows_sys::Win32::UI::WindowsAndMessaging::{RI_KEY_BREAK, RI_KEY_E0};
    let mut raw: RAWINPUT = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let n = unsafe {
        GetRawInputData(lp as _, RID_INPUT, (&mut raw as *mut RAWINPUT).cast(), &mut size, std::mem::size_of::<RAWINPUTHEADER>() as u32)
    };
    if n == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD {
        return None;
    }
    let k = unsafe { raw.data.keyboard };
    if k.ExtraInformation as usize == MARK || k.VKey == VK_DUMMY || k.VKey == 0xFF {
        return None;
    }
    let flags = k.Flags as u32;
    let down = flags & RI_KEY_BREAK == 0;
    let e0 = flags & RI_KEY_E0 != 0;
    let key = match k.VKey {
        VK_LWIN => ChordKey::Win { right: false },
        VK_RWIN => ChordKey::Win { right: true },
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => ChordKey::Ctrl { right: e0 || k.VKey == VK_RCONTROL },
        // Shift with E0 is the fake Shift some keys send (Num Lock, Print Screen): not a key.
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT if e0 => return None,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => ChordKey::Shift { right: k.MakeCode == 0x36 || k.VKey == VK_RSHIFT },
        VK_LEFT | VK_RIGHT => ChordKey::Arrow,
        _ => ChordKey::Other,
    };
    Some((key, down))
}

// ---------------------------------------------------------------- hint

// The row of desktop previews (`overlay_ui`) is the only indicator of a carry, for every method.
// This short hint only says that the foreground window cannot be moved: a rounded label at the
// bottom of the work area, where the row would be.

const HINT_CLASS: &str = "AutoSpacesCarryHint";
static HINT: AtomicUsize = AtomicUsize::new(0);
static mut HINT_STATE: Option<Hint> = None;

#[derive(Clone)]
struct Hint {
    scale: f32,
    dark: bool,
    text: String,
}

/// Windows uses the dark theme for the system (taskbar, Start, flyouts).
pub(crate) fn system_dark() -> bool {
    // Debug builds: `AUTOSPACES_OSD_THEME=dark|light` (screenshots of both looks).
    #[cfg(debug_assertions)]
    if let Ok(t) = std::env::var("AUTOSPACES_OSD_THEME") {
        return t == "dark";
    }
    let mut v = 1u32;
    let mut len = 4u32;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut len)
    };
    r == 0 && v == 0
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// Size in device-independent pixels (scaled by DPI when drawn).
const PAD: f32 = 18.0;
const LINE: f32 = 30.0;
/// Gap between the hint and the bottom of the work area (the taskbar).
const MARGIN: f32 = 40.0;

fn content_size(p: &Hint) -> (i32, i32) {
    let w = (p.text.chars().count() as f32 * 8.6 + 2.0 * PAD).max(200.0);
    ((w * p.scale) as i32, ((LINE + 2.0 * PAD) * p.scale) as i32)
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    if let Some(p) = (*std::ptr::addr_of!(HINT_STATE)).clone() {
        let (back, text) = if p.dark { (rgb(43, 43, 43), rgb(240, 240, 240)) } else { (rgb(249, 249, 249), rgb(26, 26, 26)) };
        let (w, h) = content_size(&p);
        let b = CreateSolidBrush(back);
        FillRect(dc, &RECT { left: 0, top: 0, right: w, bottom: h }, b);
        DeleteObject(b);
        SetBkMode(dc, TRANSPARENT as i32);
        let font = CreateFontW(
            -((14.0 * p.scale) as i32), 0, 0, 0, FW_SEMIBOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32, DEFAULT_PITCH as u32, wide("Segoe UI").as_ptr(),
        );
        let old = SelectObject(dc, font);
        SetTextColor(dc, text);
        let s = wide(&p.text);
        let mut r = RECT { left: 0, top: 0, right: w, bottom: h };
        DrawTextW(dc, s.as_ptr(), s.len() as i32 - 1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
        DeleteObject(font);
    }
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn hint_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        // Clicks go through to the window below.
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn hint_window() -> HWND {
    let h = HINT.load(Relaxed) as HWND;
    if !h.is_null() {
        return h;
    }
    let hinst = GetModuleHandleW(std::ptr::null());
    let class = wide(HINT_CLASS);
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(hint_proc);
    wc.hInstance = hinst;
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc); // fails harmlessly when already registered
    // Tool window of our own process: no taskbar / Alt+Tab, never activated, transparent for the
    // mouse, never a candidate (the watcher skips our process).
    let h = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinst,
        std::ptr::null(),
    );
    if !h.is_null() {
        // Opaque: layered only so that WS_EX_TRANSPARENT lets clicks through reliably.
        SetLayeredWindowAttributes(h, 0, 255, LWA_ALPHA);
    }
    HINT.store(h as usize, Relaxed);
    h
}

/// Shows `text` at the bottom of the work area of the monitor of the foreground window (or the
/// primary one), above the taskbar.
pub fn hint_show(text: &str) {
    unsafe {
        let near = GetForegroundWindow();
        let mon = if near.is_null() {
            MonitorFromWindow(std::ptr::null_mut(), MONITOR_DEFAULTTOPRIMARY)
        } else {
            MonitorFromWindow(near, MONITOR_DEFAULTTONEAREST)
        };
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) == 0 {
            return;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 || dx == 0 {
            dx = 96;
        }
        let p = Hint { scale: dx as f32 / 96.0, dark: system_dark(), text: text.to_string() };
        let (w, h) = content_size(&p);
        *std::ptr::addr_of_mut!(HINT_STATE) = Some(p);
        let hwnd = hint_window();
        if hwnd.is_null() {
            return;
        }
        let wk = mi.rcWork;
        let work = crate::core::overlay::Rect::new(wk.left, wk.top, wk.right, wk.bottom);
        let (x, y) = crate::core::overlay::hint_pos(&work, w, h, (MARGIN * dx as f32 / 96.0) as i32);
        let radius = (16.0 * dx as f32 / 96.0) as i32;
        SetWindowRgn(hwnd, CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius), 1);
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

/// Removes the hint (destroyed, so nothing of it stays in memory while idle).
pub fn hint_hide() {
    let h = HINT.swap(0, Relaxed) as HWND;
    if !h.is_null() {
        unsafe { DestroyWindow(h) };
    }
    unsafe { *std::ptr::addr_of_mut!(HINT_STATE) = None };
}