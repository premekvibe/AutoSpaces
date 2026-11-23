//! Carrying a window (plan 12.13), the Win32 side: the shortcut (RegisterHotKey), the low-level
//! keyboard hook that exists only while a window is carried, and the indicator (OSD).
//!
//! The hook records and logs nothing: it only looks at the three modifiers (released = drop) and
//! Esc (swallowed, = take the window back); every other key passes untouched.

use crate::core::carry::{Edge, Osd};
use crate::tray::wide;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW, Ellipse, EndPaint,
    FillRect, GetMonitorInfoW, GetStockObject, InvalidateRect, MonitorFromWindow, SelectObject, SetBkMode,
    SetTextColor, SetWindowRgn, CLEARTYPE_QUALITY, DEFAULT_CHARSET, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT, CLIP_DEFAULT_PRECIS, DEFAULT_PITCH, CreatePen, PS_SOLID,
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

// ---------------------------------------------------------------- indicator

const OSD_CLASS: &str = "AutoSpacesCarryOsd";
static OSD: AtomicUsize = AtomicUsize::new(0);
static mut OSD_STATE: Option<Paint> = None;

#[derive(Clone)]
struct Paint {
    osd: Osd,
    scale: f32,
    dark: bool,
    text: String,
}

/// Windows uses the dark theme for the system (taskbar, Start, flyouts).
fn system_dark() -> bool {
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

struct Colors {
    back: COLORREF,
    text: COLORREF,
    dim: COLORREF,
    accent: COLORREF,
    on_accent: COLORREF,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors { back: rgb(43, 43, 43), text: rgb(240, 240, 240), dim: rgb(110, 110, 110), accent: rgb(76, 194, 255), on_accent: rgb(0, 0, 0) }
    } else {
        Colors { back: rgb(249, 249, 249), text: rgb(26, 26, 26), dim: rgb(160, 160, 160), accent: rgb(0, 95, 184), on_accent: rgb(255, 255, 255) }
    }
}

/// Size in device-independent pixels (scaled by DPI when drawn).
const DOT: f32 = 30.0;
const GAP: f32 = 10.0;
const PAD: f32 = 18.0;
const MARK_W: f32 = 22.0;
/// More desktops than this are shown as "k / n".
const MAX_DOTS: usize = 16;

fn content_size(p: &Paint) -> (i32, i32) {
    let s = p.scale;
    if p.osd.cannot_move {
        let w = (p.text.chars().count() as f32 * 8.6 + 2.0 * PAD).max(200.0);
        return ((w * s) as i32, ((DOT + 2.0 * PAD) * s) as i32);
    }
    let n = if p.osd.count > MAX_DOTS { 3 } else { p.osd.count.max(1) };
    let marks = (p.osd.left != Edge::None) as usize + (p.osd.right != Edge::None) as usize;
    let w = 2.0 * PAD + n as f32 * DOT + (n.saturating_sub(1) + marks) as f32 * GAP + marks as f32 * MARK_W;
    ((w * s) as i32, ((DOT + 2.0 * PAD) * s) as i32)
}

unsafe fn font(px: i32, weight: u32, face: &str) -> HFONT {
    CreateFontW(
        -px, 0, 0, 0, weight as i32, 0, 0, 0, DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32, DEFAULT_PITCH as u32, wide(face).as_ptr(),
    )
}

unsafe fn text_in(dc: windows_sys::Win32::Graphics::Gdi::HDC, s: &str, r: &RECT) {
    let w = wide(s);
    let mut r = *r;
    DrawTextW(dc, w.as_ptr(), w.len() as i32 - 1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let Some(p) = (*std::ptr::addr_of!(OSD_STATE)).clone() else {
        EndPaint(hwnd, &ps);
        return;
    };
    let c = colors(p.dark);
    let s = p.scale;
    let (w, h) = content_size(&p);
    let back = CreateSolidBrush(c.back);
    FillRect(dc, &RECT { left: 0, top: 0, right: w, bottom: h }, back);
    DeleteObject(back);
    SetBkMode(dc, TRANSPARENT as i32);
    let num_font = font((14.0 * s) as i32, FW_SEMIBOLD, "Segoe UI");
    let sym_font = font((18.0 * s) as i32, FW_NORMAL, "Segoe UI Symbol");
    let old_font = SelectObject(dc, num_font);
    if p.osd.cannot_move {
        SetTextColor(dc, c.text);
        text_in(dc, &p.text, &RECT { left: 0, top: 0, right: w, bottom: h });
    } else {
        let px = |v: f32| (v * s) as i32;
        let top = px(PAD);
        let bottom = top + px(DOT);
        let mut x = PAD;
        let mark = |dc: windows_sys::Win32::Graphics::Gdi::HDC, x: &mut f32, e: Edge, left: bool| {
            if e == Edge::None {
                return;
            }
            let glyph = match (e, left) {
                (Edge::New, _) => "+",
                (_, true) => "\u{21BA}",
                _ => "\u{21BB}",
            };
            SelectObject(dc, sym_font);
            SetTextColor(dc, c.dim);
            text_in(dc, glyph, &RECT { left: px(*x), top, right: px(*x + MARK_W), bottom });
            *x += MARK_W + GAP;
        };
        mark(dc, &mut x, p.osd.left, true);
        if p.osd.count > MAX_DOTS {
            SelectObject(dc, num_font);
            SetTextColor(dc, c.accent);
            let t = format!("{} / {}", p.osd.target + 1, p.osd.count);
            text_in(dc, &t, &RECT { left: px(x), top, right: px(x + 3.0 * DOT + 2.0 * GAP), bottom });
            x += 3.0 * DOT + 3.0 * GAP;
        } else {
            let pen_dim = CreatePen(PS_SOLID, ((1.5 * s) as i32).max(1), c.dim);
            for i in 0..p.osd.count.max(1) {
                let r = RECT { left: px(x), top, right: px(x + DOT), bottom };
                let current = i == p.osd.target;
                if current {
                    let b = CreateSolidBrush(c.accent);
                    let ob = SelectObject(dc, b);
                    let op = SelectObject(dc, GetStockObject(NULL_PEN));
                    Ellipse(dc, r.left, r.top, r.right + 1, r.bottom + 1);
                    SelectObject(dc, op);
                    SelectObject(dc, ob);
                    DeleteObject(b);
                } else {
                    let b = CreateSolidBrush(c.back);
                    let ob = SelectObject(dc, b);
                    let op = SelectObject(dc, pen_dim);
                    Ellipse(dc, r.left, r.top, r.right, r.bottom);
                    SelectObject(dc, op);
                    SelectObject(dc, ob);
                    DeleteObject(b);
                }
                SelectObject(dc, num_font);
                SetTextColor(dc, if current { c.on_accent } else { c.text });
                text_in(dc, &(i + 1).to_string(), &r);
                x += DOT + GAP;
            }
            DeleteObject(pen_dim);
        }
        mark(dc, &mut x, p.osd.right, false);
    }
    SelectObject(dc, old_font);
    DeleteObject(num_font);
    DeleteObject(sym_font);
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn osd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
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

unsafe fn osd_window() -> HWND {
    let h = OSD.load(Relaxed) as HWND;
    if !h.is_null() {
        return h;
    }
    let hinst = GetModuleHandleW(std::ptr::null());
    let class = wide(OSD_CLASS);
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(osd_proc);
    wc.hInstance = hinst;
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc); // fails harmlessly when already registered
    // A tool window has no application view: it is never cloaked by a desktop switch, so it stays
    // visible on every desktop while the desktops slide (no pinning needed); never in Alt+Tab or
    // on the taskbar, never activated, transparent for the mouse. Our own process: the window
    // watcher skips it, so it is never a candidate.
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
    OSD.store(h as usize, Relaxed);
    h
}

/// Shows (or updates) the indicator in the middle of the monitor of `near` (the carried window;
/// the foreground window or the primary monitor when null).
pub fn osd_show(osd: &Osd, near: HWND, cannot_move_text: &str) {
    unsafe {
        let near = if near.is_null() { GetForegroundWindow() } else { near };
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
        let p = Paint { osd: osd.clone(), scale: dx as f32 / 96.0, dark: system_dark(), text: cannot_move_text.to_string() };
        let (w, h) = content_size(&p);
        *std::ptr::addr_of_mut!(OSD_STATE) = Some(p);
        let hwnd = osd_window();
        if hwnd.is_null() {
            return;
        }
        let wk = mi.rcWork;
        let x = wk.left + ((wk.right - wk.left) - w) / 2;
        let y = wk.top + ((wk.bottom - wk.top) - h) / 2;
        let radius = (16.0 * dx as f32 / 96.0) as i32;
        SetWindowRgn(hwnd, CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius), 1);
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

/// Removes the indicator (destroyed, so nothing of it stays in memory while idle).
pub fn osd_hide() {
    let h = OSD.swap(0, Relaxed) as HWND;
    if !h.is_null() {
        unsafe { DestroyWindow(h) };
    }
    unsafe { *std::ptr::addr_of_mut!(OSD_STATE) = None };
}
