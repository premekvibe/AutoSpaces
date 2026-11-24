//! The "overlay" way of carrying a window (plan 12.13), the Win32 side. In the style of Task View:
//!
//! * a layer over the work area of every monitor shows the current desktop blurred and dimmed;
//!   the carried window is shown on top of it as a 1:1 DWM thumbnail (sharp, in place, live), so
//!   the real window is never touched (no topmost, no z-order change, nothing to restore);
//! * a row of live desktop previews at the bottom of the carried window's monitor, the target
//!   highlighted by a frame that glides from one preview to the next.
//!
//! Every desktop has a small, cloaked copy ("mini", about 320 px wide): its wallpaper (WIC, small,
//! decoded on a worker thread) plus DWM thumbnails of its windows. The previews are thumbnails of
//! the minis; the blur is a mini enlarged 16 times with sub-pixel shifts and blended (the system
//! backdrop does not blur for windows that are never active). Nothing exists while idle: the
//! layers are created when a carry starts and destroyed after it.

use crate::core::overlay::{self as ov, Motion, Placement, Rect};
use crate::desktops::Guid;
use crate::tray::wide;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmFlush, DwmGetWindowAttribute, DwmIsCompositionEnabled, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail,
    DwmSetWindowAttribute, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties, DWMWA_CLOAK, DWMWA_CLOAKED,
    DWMWA_EXTENDED_FRAME_BOUNDS, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreatePen,
    CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, EnumDisplayMonitors, FillRect,
    GetMonitorInfoW, GetStockObject, InvalidateRect, RoundRect, SelectObject, SetBkMode,
    SetTextColor, SetWindowRgn, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, HBITMAP, HDC, HMONITOR, MONITORINFO, NULL_BRUSH,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_DOT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GetSystemMetrics, GetWindowLongPtrW, GetWindowLongW,
    GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, ShowWindow, SystemParametersInfoW, GWLP_USERDATA, GWL_EXSTYLE,
    HTTRANSPARENT, LWA_ALPHA, MA_NOACTIVATE, SM_REMOTESESSION, SPI_GETCLIENTAREAANIMATION, SPI_GETDESKWALLPAPER,
    SW_SHOWNOACTIVATE, WM_APP, WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use crate::glass;

/// Posted to the row's window when the wallpapers are decoded (`wParam` = generation).
const WM_WALLPAPERS: u32 = WM_APP + 41;
const CLASS: &str = "AutoSpacesCarryLayer";

static mut STATE: Option<Overlay> = None;
static mut GENERATION: usize = 0;

fn state() -> Option<&'static mut Overlay> {
    unsafe { (*std::ptr::addr_of_mut!(STATE)).as_mut() }
}

/// The overlay can be drawn here: DWM composition is on (always since Windows 8) and this is not
/// a remote session (where every frame would travel over the network).
pub fn available() -> bool {
    let mut on: BOOL = 0;
    let comp = unsafe { DwmIsCompositionEnabled(&mut on) } >= 0 && on != 0;
    comp && unsafe { GetSystemMetrics(SM_REMOTESESSION) } == 0
}

/// The window the layers show 1:1 (the carried one, or the one of a preview).
pub fn carried() -> Option<HWND> {
    state().map(|s| s.carried)
}

/// Layers exist (a carry runs or the layers are fading away).
pub fn active() -> bool {
    state().is_some()
}

/// Frames are needed now (something moves or fades); otherwise the message loop sleeps.
pub fn animating() -> bool {
    state().is_some_and(|s| s.busy(s.now()))
}

// ---------------------------------------------------------------- small Win32 helpers

fn rc(r: &Rect) -> RECT {
    RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

fn from_rc(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

/// Registers a thumbnail of `src` in `dest`; 0 on failure.
fn thumb(dest: HWND, src: HWND) -> isize {
    let mut t = 0isize;
    if unsafe { DwmRegisterThumbnail(dest, src, &mut t) } < 0 {
        return 0;
    }
    t
}

fn thumb_size(t: isize) -> (i32, i32) {
    let mut s = SIZE { cx: 0, cy: 0 };
    unsafe { DwmQueryThumbnailSourceSize(t, &mut s) };
    (s.cx, s.cy)
}

fn place_thumb(t: isize, dest: &Rect, src: &Rect, opacity: u8, visible: bool) {
    if t == 0 {
        return;
    }
    let p = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE | DWM_TNP_OPACITY | DWM_TNP_VISIBLE | DWM_TNP_SOURCECLIENTAREAONLY,
        rcDestination: rc(dest),
        rcSource: rc(src),
        opacity,
        fVisible: visible as BOOL,
        fSourceClientAreaOnly: 0,
    };
    unsafe { DwmUpdateThumbnailProperties(t, &p) };
}

/// place_thumb with the destination cut to the window (ounds); hidden when nothing is left.
fn place_clipped(t: isize, dest: Rect, src: Rect, bounds: &Rect, opacity: u8) {
    match ov::clip_thumb(dest, src, bounds) {
        Some((d, s)) => place_thumb(t, &d, &s, opacity, true),
        None => place_thumb(t, &dest, &src, 0, false),
    }
}

fn unthumb(t: &mut isize) {
    if *t != 0 {
        unsafe { DwmUnregisterThumbnail(*t) };
        *t = 0;
    }
}

fn frame_bounds(h: HWND) -> Option<(Rect, Rect)> {
    let mut w: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(h, &mut w) } == 0 {
        return None;
    }
    let mut f: RECT = unsafe { std::mem::zeroed() };
    let ok = unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS as u32, (&mut f as *mut RECT).cast(), 16) } >= 0;
    let w = from_rc(&w);
    Some((w, if ok { from_rc(&f) } else { w }))
}

fn reg_string(key: &str, name: &str) -> Option<String> {
    let (k, n) = (wide(key), wide(name));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len)
    };
    if r != 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
    Some(s.trim_end_matches('\0').to_string())
}

fn desktop_key(g: &Guid) -> String {
    format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops\Desktops\{{{g}}}")
}

unsafe fn layer(ex: u32, r: &Rect, kind: isize) -> HWND {
    let hinst = GetModuleHandleW(std::ptr::null());
    let class = wide(CLASS);
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(layer_proc);
    wc.hInstance = hinst;
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc); // fails harmlessly when already registered
    // Tool windows of our own process: never on the taskbar or in Alt+Tab, never activated, not
    // cloaked by a desktop switch, skipped by the window watcher, never a candidate.
    let h = CreateWindowExW(
        ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        r.left,
        r.top,
        r.w().max(1),
        r.h().max(1),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinst,
        std::ptr::null(),
    );
    if !h.is_null() {
        SetWindowLongPtrW(h, GWLP_USERDATA, kind);
    }
    h
}

const KIND_BLUR: isize = 1;
const KIND_STRIP: isize = 2;
// Minis store their wallpaper bitmap (HBITMAP) in GWLP_USERDATA (never 1 or 2).

unsafe extern "system" fn layer_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            match GetWindowLongPtrW(hwnd, GWLP_USERDATA) {
                KIND_STRIP => {
                    if let Some(s) = state() {
                        s.paint_strip(dc);
                    }
                }
                0 | KIND_BLUR => {
                    FillRect(dc, &ps.rcPaint, GetStockObject(BLACK_BRUSH));
                }
                bmp => {
                    let mem = CreateCompatibleDC(dc);
                    let old = SelectObject(mem, bmp as HBITMAP);
                    let r = ps.rcPaint;
                    BitBlt(dc, r.left, r.top, r.right - r.left, r.bottom - r.top, mem, r.left, r.top, SRCCOPY);
                    SelectObject(mem, old);
                    DeleteDC(mem);
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_WALLPAPERS => {
            if let Some(s) = state() {
                if s.generation == wp {
                    s.wallpapers_ready();
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------------------------------------------------------------- what is on the screen

struct Monitor {
    rect: Rect,
    work: Rect,
    dpi_scale: f64,
}

unsafe extern "system" fn monitor_cb(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    let out = &mut *(lp as *mut Vec<Monitor>);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(m, &mut mi) != 0 {
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 || dx == 0 {
            dx = 96;
        }
        out.push(Monitor { rect: from_rc(&mi.rcMonitor), work: from_rc(&mi.rcWork), dpi_scale: dx as f64 / 96.0 });
    }
    1
}

fn monitors() -> Vec<Monitor> {
    let mut v: Vec<Monitor> = Vec::new();
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(monitor_cb), &mut v as *mut _ as LPARAM) };
    v
}

/// A window shown in the previews.
struct Win {
    hwnd: HWND,
    window: Rect,
    frame: Rect,
    /// Its desktop; zero = on all desktops.
    desk: Guid,
}

unsafe extern "system" fn window_cb(h: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp as *mut Vec<HWND>);
    out.push(h);
    1
}

/// Top-level windows from the top of the z-order down that have a desktop (an application view):
/// visible, not minimized, not cloaked by their app, not ours.
fn windows(desk_of: &dyn Fn(HWND) -> Option<Guid>, skip: HWND) -> Vec<Win> {
    let mut all: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(window_cb), &mut all as *mut _ as LPARAM) };
    let me = unsafe { GetCurrentProcessId() };
    let mut out = Vec::new();
    for h in all {
        if h == skip || unsafe { IsWindowVisible(h) == 0 || IsIconic(h) != 0 } {
            continue;
        }
        let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
        if ex & WS_EX_TOOLWINDOW != 0 {
            continue;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, &mut pid) };
        if pid == me {
            continue;
        }
        let mut cloaked = 0u32;
        unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, (&mut cloaked as *mut u32).cast(), 4) };
        if cloaked & 1 != 0 {
            continue; // DWM_CLOAKED_APP: hidden by the app itself
        }
        let Some(desk) = desk_of(h) else { continue };
        let Some((window, frame)) = frame_bounds(h) else { continue };
        if frame.w() <= 0 || frame.h() <= 0 {
            continue;
        }
        out.push(Win { hwnd: h, window, frame, desk });
    }
    out
}

// ---------------------------------------------------------------- wallpapers (WIC)

/// Wallpaper of one desktop: file (None = solid colour).
fn wallpaper_of(g: &Guid) -> Option<String> {
    if let Some(p) = reg_string(&desktop_key(g), "Wallpaper").filter(|p| !p.is_empty()) {
        return Some(p);
    }
    let mut buf = vec![0u16; 520];
    let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr().cast(), 0) } != 0;
    let n = buf.iter().position(|c| *c == 0).unwrap_or(0);
    let p = String::from_utf16_lossy(&buf[..n]);
    (ok && !p.is_empty()).then_some(p)
}

fn placement() -> Placement {
    let k = r"Control Panel\Desktop";
    Placement::from_registry(&reg_string(k, "WallpaperStyle").unwrap_or_default(), &reg_string(k, "TileWallpaper").unwrap_or_default())
}

fn background_color() -> u32 {
    reg_string(r"Control Panel\Colors", "Background").and_then(|s| ov::parse_rgb(&s)).unwrap_or(0)
}

type Raw = *mut c_void;

/// Minimal raw COM pointer for WIC (documented interfaces, fixed vtable order).
struct Com(Raw);

impl Com {
    unsafe fn f<F: Copy>(&self, slot: usize) -> F {
        let vtbl = *(self.0 as *const *const *const c_void);
        std::mem::transmute_copy(&*vtbl.add(slot))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        type Release = unsafe extern "system" fn(Raw) -> u32;
        if !self.0.is_null() {
            unsafe { self.f::<Release>(2)(self.0) };
        }
    }
}

fn guid(s: &str) -> Guid {
    Guid::parse(s).unwrap_or_default()
}

/// Decodes an image file scaled by WIC; `size` gets the image's size and says the size wanted.
/// Pixels are 32-bit BGR, top-down.
fn decode(path: &str, size: impl FnOnce(i32, i32) -> (i32, i32)) -> Option<(Vec<u32>, i32, i32, i32, i32)> {
    type Create0 = unsafe extern "system" fn(Raw, *mut Raw) -> i32;
    type FromFile = unsafe extern "system" fn(Raw, *const u16, *const Guid, u32, i32, *mut Raw) -> i32;
    type GetFrame = unsafe extern "system" fn(Raw, u32, *mut Raw) -> i32;
    type GetSize = unsafe extern "system" fn(Raw, *mut u32, *mut u32) -> i32;
    type ScalerInit = unsafe extern "system" fn(Raw, Raw, u32, u32, i32) -> i32;
    type ConvInit = unsafe extern "system" fn(Raw, Raw, *const Guid, i32, Raw, f64, i32) -> i32;
    type CopyPixels = unsafe extern "system" fn(Raw, *const c_void, u32, u32, *mut u8) -> i32;
    let clsid = guid("CACAF262-9370-4615-A13B-9F5539DA4C0A"); // CLSID_WICImagingFactory
    let iid = guid("EC5EC8A9-C395-4314-9C77-54D7A935FF70"); // IWICImagingFactory
    let bgr = guid("6FDDC324-4E03-4BFE-B185-3D77768DC90E"); // GUID_WICPixelFormat32bppBGR
    unsafe {
        let mut p: Raw = std::ptr::null_mut();
        if CoCreateInstance(clsid.as_ptr(), std::ptr::null_mut(), CLSCTX_INPROC_SERVER, iid.as_ptr(), &mut p) < 0 || p.is_null() {
            return None;
        }
        let factory = Com(p);
        let name = wide(path);
        let mut d: Raw = std::ptr::null_mut();
        // CreateDecoderFromFilename(name, vendor, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
        if factory.f::<FromFile>(3)(factory.0, name.as_ptr(), std::ptr::null(), 0x8000_0000, 0, &mut d) < 0 || d.is_null() {
            return None;
        }
        let decoder = Com(d);
        let mut fr: Raw = std::ptr::null_mut();
        if decoder.f::<GetFrame>(13)(decoder.0, 0, &mut fr) < 0 || fr.is_null() {
            return None;
        }
        let frame = Com(fr);
        let (mut iw, mut ih) = (0u32, 0u32);
        if frame.f::<GetSize>(3)(frame.0, &mut iw, &mut ih) < 0 || iw == 0 || ih == 0 {
            return None;
        }
        let (w, h) = size(iw as i32, ih as i32);
        let (w, h) = (w.clamp(1, 4096), h.clamp(1, 4096));
        // JPEG and others that scale while decoding (IWICBitmapSourceTransform, e.g. 1/8 in the
        // DCT): the full-size picture is never held in memory.
        type Qi = unsafe extern "system" fn(Raw, *const Guid, *mut Raw) -> i32;
        type Closest = unsafe extern "system" fn(Raw, *mut u32, *mut u32) -> i32;
        type TCopy = unsafe extern "system" fn(Raw, *const c_void, u32, u32, *mut Guid, i32, u32, u32, *mut u8) -> i32;
        let iid_transform = guid("3B16811B-6A43-4EC9-B713-3D5A0C13B940");
        let mut t: Raw = std::ptr::null_mut();
        if frame.f::<Qi>(0)(frame.0, &iid_transform, &mut t) >= 0 && !t.is_null() {
            let tr = Com(t);
            let (mut cw, mut ch) = (w as u32, h as u32);
            if tr.f::<Closest>(4)(tr.0, &mut cw, &mut ch) >= 0 && cw > 0 && ch > 0 && cw <= 8192 && ch <= 8192 {
                let mut fmt = bgr;
                let stride = cw * 4;
                let mut px = vec![0u32; (cw * ch) as usize];
                let r = tr.f::<TCopy>(3)(tr.0, std::ptr::null(), cw, ch, &mut fmt, 0, stride, stride * ch, px.as_mut_ptr().cast());
                if r >= 0 && fmt == bgr {
                    let out = ov::resize(&px, cw as usize, ch as usize, w as usize, h as usize);
                    return Some((out, w, h, iw as i32, ih as i32));
                }
            }
        }
        let mut s: Raw = std::ptr::null_mut();
        if factory.f::<Create0>(11)(factory.0, &mut s) < 0 || s.is_null() {
            return None;
        }
        let scaler = Com(s);
        // WICBitmapInterpolationModeFant = 3
        if scaler.f::<ScalerInit>(8)(scaler.0, frame.0, w as u32, h as u32, 3) < 0 {
            return None;
        }
        let mut c: Raw = std::ptr::null_mut();
        if factory.f::<Create0>(10)(factory.0, &mut c) < 0 || c.is_null() {
            return None;
        }
        let conv = Com(c);
        if conv.f::<ConvInit>(8)(conv.0, scaler.0, &bgr, 0, std::ptr::null_mut(), 0.0, 0) < 0 {
            return None;
        }
        let mut px = vec![0u32; (w * h) as usize];
        let stride = (w * 4) as u32;
        if conv.f::<CopyPixels>(7)(conv.0, std::ptr::null(), stride, stride * h as u32, px.as_mut_ptr().cast()) < 0 {
            return None;
        }
        Some((px, w, h, iw as i32, ih as i32))
    }
}

/// Small wallpaper pictures kept between carries (key: file, its time, size, placement, colour);
/// the oldest go beyond `CACHE_MAX` (each is about 230 KB at 2560×1440).
static mut WALL_CACHE: Vec<(String, Vec<u32>)> = Vec::new();
const CACHE_MAX: usize = 6;

fn cache_get(key: &str) -> Option<Vec<u32>> {
    unsafe { (*std::ptr::addr_of!(WALL_CACHE)).iter().find(|e| e.0 == key).map(|e| e.1.clone()) }
}

fn cache_put(key: String, px: Vec<u32>) {
    unsafe {
        let c = &mut *std::ptr::addr_of_mut!(WALL_CACHE);
        c.retain(|e| e.0 != key);
        c.push((key, px));
        while c.len() > CACHE_MAX {
            c.remove(0);
        }
    }
}

/// A desktop's wallpaper composed at a mini's size (or None: solid colour only).
struct WallJob {
    path: String,
    w: i32,
    h: i32,
    scale: i32,
}

/// Composes the wallpaper picture of a mini: background colour plus the image placed as the
/// user set it. Returns BGR pixels w×h.
fn compose(job: &WallJob, placement: Placement, color: u32) -> Option<Vec<u32>> {
    let (w, h, s) = (job.w, job.h, job.scale);
    let img = decode(&job.path, |iw, ih| {
        let first = ov::place(placement, iw, ih, w, h, s).first().copied().unwrap_or_default();
        (first.w(), first.h())
    })?;
    let (px, iw, ih, ow, oh) = img;
    let rects = ov::place(placement, ow, oh, w, h, s);
    let bg = (color & 0xFF) << 16 | (color & 0xFF00) | (color >> 16 & 0xFF);
    let mut out = vec![bg; (w * h) as usize];
    for r in rects {
        for y in 0..ih {
            let ty = r.top + y;
            if ty < 0 || ty >= h {
                continue;
            }
            for x in 0..iw {
                let tx = r.left + x;
                if tx >= 0 && tx < w {
                    out[(ty * w + tx) as usize] = px[(y * iw + x) as usize];
                }
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- the layers

/// A desktop's wallpaper on one monitor, small (cloaked window painting the picture; only seen
/// through thumbnails, enlarged under the glass).
struct Wall {
    mon: usize,
    desk: usize,
    hwnd: HWND,
    size: (i32, i32),
    scale: i32,
    bitmap: HBITMAP,
    /// Its wallpaper was handed to the decoding thread.
    queued: bool,
}

/// A desktop on one monitor at full resolution: cloaked window (no surface of its own) holding
/// the wallpaper enlarged and DWM thumbnails of the desktop's windows 1:1. The background and the
/// previews in the row are thumbnails of it.
struct Panel {
    mon: usize,
    desk: usize,
    hwnd: HWND,
    thumbs: Vec<isize>,
}

/// A monitor's background (carry with the overlay only): the target desktop, sliding.
struct Band {
    hwnd: HWND,
    /// Each desktop's wallpaper and windows, in desktop order.
    sets: Vec<BandSet>,
}

struct BandSet {
    /// Thumbnail, its place on the monitor (monitor coordinates) and its source part.
    thumbs: Vec<(isize, Rect, Rect)>,
    shown: bool,
}

/// A monitor's glass over the background: acrylic, or a plain dark layer where acrylic is not
/// available (Windows 10, "Transparency effects" off).
struct GlassLayer {
    hwnd: HWND,
    glass: Option<glass::Glass>,
}

/// Opacity of the dark layer used instead of acrylic.
const DIM_ALPHA: f64 = 0.55;

struct Desk {
    id: Guid,
    label: String,
    wallpaper: Option<String>,
}

struct Overlay {
    generation: usize,
    t0: Instant,
    animate: bool,
    /// Background, glass and the carried window's stand-in (the overlay method); false = only
    /// the row of previews (the other methods).
    full: bool,
    monitors: Vec<Monitor>,
    strip_mon: usize,
    carried: HWND,
    carried_rects: Option<(Rect, Rect)>,
    wins: Vec<Win>,
    desks: Vec<Desk>,
    target: usize,
    plus: bool,
    walls: Vec<Wall>,
    panels: Vec<Panel>,
    bands: Vec<Band>,
    glasses: Vec<GlassLayer>,
    /// The carried window 1:1 over the glass.
    standin: HWND,
    standin_thumb: isize,
    strip: HWND,
    layout: ov::Strip,
    tiles: Vec<isize>,
    /// The carried window in the target's preview.
    strip_carried: isize,
    motion: Motion,
    /// The final position of the last motion has been drawn.
    settled: bool,
    /// A jump over the edge in progress (from, to): the background cross-fades.
    jump: Option<(usize, usize)>,
    /// Fading in (true) or out (false) since then (ms).
    fade: Option<(bool, f64)>,
    opacity: f64,
    placement: Placement,
    color: u32,
    decoded: Arc<Mutex<Vec<(usize, usize, String, Vec<u32>)>>>,
    dark: bool,
    last_frame: Option<f64>,
    gaps: Vec<f32>,
    /// Time spent per frame before waiting for vsync (ms).
    work: Vec<f32>,
}

/// What the host knows when a carry starts.
pub struct Begin<'a> {
    pub carried: HWND,
    pub desktops: Vec<Guid>,
    pub start: usize,
    pub target: usize,
    pub create_new: bool,
    /// The overlay method (background, glass, stand-in); false = only the row.
    pub full: bool,
    /// Built invisible (a preview being prepared); `reveal` fades it in.
    pub hidden: bool,
    pub desk_of: &'a dyn Fn(HWND) -> Option<Guid>,
    pub word: &'a str,
}

/// Creates the layers (fading in). Err = they cannot be drawn (the carry falls back).
pub fn begin(b: Begin) -> Result<(), String> {
    destroy();
    if b.desktops.is_empty() {
        return Err("no desktops".into());
    }
    let mons = monitors();
    if mons.is_empty() {
        return Err("no monitor".into());
    }
    let Some((cw, cf)) = frame_bounds(b.carried) else { return Err("carried window has no rectangle".into()) };
    let centre = ((cf.left + cf.right) / 2, (cf.top + cf.bottom) / 2);
    let strip_mon = mons
        .iter()
        .position(|m| centre.0 >= m.rect.left && centre.0 < m.rect.right && centre.1 >= m.rect.top && centre.1 < m.rect.bottom)
        .or_else(|| mons.iter().position(|m| m.rect.intersects(&cf)))
        .unwrap_or(0);
    let mut animate: BOOL = 1;
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut animate as *mut BOOL).cast(), 0) };
    let desks = b
        .desktops
        .iter()
        .enumerate()
        .map(|(i, g)| Desk {
            id: *g,
            label: ov::desktop_label(i, reg_string(&desktop_key(g), "Name").as_deref(), b.word),
            wallpaper: wallpaper_of(g),
        })
        .collect();
    let generation = unsafe {
        GENERATION += 1;
        GENERATION
    };
    let start = b.start.min(b.desktops.len() - 1);
    let ov = Overlay {
        generation,
        t0: Instant::now(),
        animate: animate != 0,
        full: b.full,
        strip_mon,
        carried: b.carried,
        carried_rects: Some((cw, cf)),
        wins: windows(b.desk_of, b.carried),
        desks,
        // The background starts at the current desktop; a target given already is moved to.
        target: start,
        plus: b.create_new,
        walls: Vec::new(),
        panels: Vec::new(),
        bands: Vec::new(),
        glasses: Vec::new(),
        standin: std::ptr::null_mut(),
        standin_thumb: 0,
        strip: std::ptr::null_mut(),
        layout: ov::strip(&mons[strip_mon].work, &mons[strip_mon].rect, b.desktops.len(), b.create_new, mons[strip_mon].dpi_scale),
        tiles: Vec::new(),
        strip_carried: 0,
        motion: Motion::at(start as f64),
        settled: true,
        jump: None,
        fade: if b.hidden { None } else { Some((true, 0.0)) },
        opacity: 0.0,
        placement: placement(),
        color: background_color(),
        decoded: Arc::new(Mutex::new(Vec::new())),
        dark: crate::carry_ui::system_dark(),
        last_frame: None,
        gaps: Vec::new(),
        work: Vec::new(),
        monitors: mons,
    };
    unsafe { *std::ptr::addr_of_mut!(STATE) = Some(ov) };
    let s = state().unwrap();
    if let Err(e) = s.build() {
        destroy();
        return Err(e);
    }
    s.load_wallpapers();
    s.go_to(b.target.min(s.desks.len() - 1));
    Ok(())
}

/// A cloaked window: never visible itself, only through thumbnails.
unsafe fn cloaked(ex: u32, r: &Rect, kind: isize) -> HWND {
    let h = layer(ex, r, kind);
    if !h.is_null() {
        let on: BOOL = 1;
        DwmSetWindowAttribute(h, DWMWA_CLOAK as u32, (&on as *const BOOL).cast(), 4);
        ShowWindow(h, SW_SHOWNOACTIVATE);
    }
    h
}

impl Overlay {
    fn now(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }

    fn busy(&self, now: f64) -> bool {
        // Until a frame has drawn the final position (also when the thread was busy meanwhile).
        self.fade.is_some() || !self.motion.done(now) || !self.settled
    }

    /// Monitors with a background (all for the overlay method, none for the row only).
    fn background_monitors(&self) -> std::ops::Range<usize> {
        if self.full {
            0..self.monitors.len()
        } else {
            0..0
        }
    }

    /// Panels the layers need: every desktop on every monitor with a background, and on the
    /// row's monitor.
    fn ensure_panels(&mut self) -> Result<(), String> {
        for m in 0..self.monitors.len() {
            if self.full || m == self.strip_mon {
                for d in 0..self.desks.len() {
                    self.panel(m, d).ok_or("panel window failed")?;
                }
            }
        }
        Ok(())
    }

    fn build(&mut self) -> Result<(), String> {
        self.ensure_panels()?;
        let use_glass = glass::available();
        for m in self.background_monitors() {
            let work = self.monitors[m].work;
            // The background is opaque and only fades with the glass over it.
            let h = unsafe { layer(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT, &work, KIND_BLUR) };
            if h.is_null() {
                return Err("CreateWindowEx failed".into());
            }
            unsafe { SetLayeredWindowAttributes(h, 0, 0, LWA_ALPHA) };
            self.bands.push(Band { hwnd: h, sets: Vec::new() });
            self.build_band(m)?;
            unsafe { ShowWindow(h, SW_SHOWNOACTIVATE) };
        }
        for m in self.background_monitors() {
            let work = self.monitors[m].work;
            let mut g = None;
            let mut h = std::ptr::null_mut();
            if use_glass {
                h = unsafe { layer(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TRANSPARENT, &work, KIND_BLUR) };
                match glass::Glass::attach(h, work.w() as f32, work.h() as f32, glass::tint(self.dark)) {
                    Ok(x) => {
                        x.set_opacity(0.0);
                        g = Some(x);
                    }
                    Err(e) => {
                        crate::log_line(&format!("carry: acrylic not available ({e}); dark layer instead"));
                        unsafe { DestroyWindow(h) };
                        h = std::ptr::null_mut();
                    }
                }
            }
            if h.is_null() {
                h = unsafe { layer(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT, &work, KIND_BLUR) };
                if h.is_null() {
                    return Err("CreateWindowEx failed".into());
                }
                unsafe { SetLayeredWindowAttributes(h, 0, 0, LWA_ALPHA) };
            }
            self.glasses.push(GlassLayer { hwnd: h, glass: g });
            unsafe { ShowWindow(h, SW_SHOWNOACTIVATE) };
        }
        if self.full {
            self.build_standin()?;
        }
        self.build_strip()?;
        Ok(())
    }

    fn wall(&mut self, mon: usize, desk: usize) -> Option<usize> {
        if let Some(i) = self.walls.iter().position(|w| w.mon == mon && w.desk == desk) {
            return Some(i);
        }
        let mr = self.monitors[mon].rect;
        // Small: under acrylic nothing of it is sharp; the dark layer without blur gets more.
        let scale = if self.full && glass::available() { ov::mini_scale(mr.w()) } else { (ov::mini_scale(mr.w()) / 2).max(2) };
        let (w, h) = ((mr.w() + scale - 1) / scale, (mr.h() + scale - 1) / scale);
        let bitmap = unsafe { dib(w, h, None, self.color) };
        let hwnd = unsafe { cloaked(0, &Rect::new(mr.left, mr.top, mr.left + w, mr.top + h), bitmap as isize) };
        if hwnd.is_null() {
            unsafe { DeleteObject(bitmap) };
            return None;
        }
        self.walls.push(Wall { mon, desk, hwnd, size: (w, h), scale, bitmap, queued: false });
        Some(self.walls.len() - 1)
    }

    fn panel(&mut self, mon: usize, desk: usize) -> Option<usize> {
        if let Some(i) = self.panels.iter().position(|p| p.mon == mon && p.desk == desk) {
            return Some(i);
        }
        let k = self.wall(mon, desk)?;
        let (wall, wsize, wscale) = (self.walls[k].hwnd, self.walls[k].size, self.walls[k].scale);
        let mr = self.monitors[mon].rect;
        // No surface of its own: it holds only thumbnails (shown scaled down in the row).
        let hwnd = unsafe { cloaked(WS_EX_NOREDIRECTIONBITMAP, &mr, KIND_BLUR) };
        if hwnd.is_null() {
            return None;
        }
        let mut thumbs = Vec::new();
        // The wallpaper enlarged to the monitor.
        let full = Rect::new(0, 0, mr.w(), mr.h());
        let t = thumb(hwnd, wall);
        place_clipped(t, Rect::new(0, 0, wsize.0 * wscale, wsize.1 * wscale), Rect::new(0, 0, wsize.0, wsize.1), &full, 255);
        thumbs.push(t);
        let id = self.desks[desk].id;
        // Bottom first: later thumbnails are drawn on top. The carried window is not here.
        for win in self.wins.iter().rev() {
            if !(win.desk == id || win.desk.is_zero()) || !win.frame.intersects(&mr) {
                continue;
            }
            let t = thumb(hwnd, win.hwnd);
            if t != 0 {
                let src = ov::source_rect(&win.window, &win.frame, thumb_size(t));
                place_clipped(t, win.frame.offset(-mr.left, -mr.top), src, &full, 255);
                thumbs.push(t);
            }
        }
        self.panels.push(Panel { mon, desk, hwnd, thumbs });
        Some(self.panels.len() - 1)
    }

    /// (Re)registers monitor `m`'s background: for every desktop its wallpaper (enlarged) and its
    /// windows 1:1, directly in the band (hidden until in view). Not through the panels: a
    /// full-size thumbnail of a cloaked panel came out empty or showed the screen behind it
    /// (measured), only scaled-down ones (the row's previews) are right.
    fn build_band(&mut self, m: usize) -> Result<(), String> {
        let mr = self.monitors[m].rect;
        let mut sources: Vec<Vec<(HWND, Rect, Option<(Rect, Rect)>)>> = Vec::new();
        for d in 0..self.desks.len() {
            let k = self.wall(m, d).ok_or("wall window failed")?;
            let w = &self.walls[k];
            let mut list = vec![(w.hwnd, Rect::new(0, 0, w.size.0 * w.scale, w.size.1 * w.scale), Some((Rect::new(0, 0, w.size.0, w.size.1), Rect::default())))];
            let id = self.desks[d].id;
            // Bottom first: later thumbnails are drawn on top. The carried window is not here.
            for win in self.wins.iter().rev() {
                if (win.desk == id || win.desk.is_zero()) && win.frame.intersects(&mr) {
                    list.push((win.hwnd, win.frame.offset(-mr.left, -mr.top), None));
                }
            }
            sources.push(list);
        }
        let b = &mut self.bands[m];
        for s in b.sets.iter_mut() {
            for t in s.thumbs.iter_mut() {
                unthumb(&mut t.0);
            }
        }
        b.sets.clear();
        for list in sources {
            let mut set = BandSet { thumbs: Vec::new(), shown: true };
            for (src_hwnd, dest, wall_src) in list {
                let t = thumb(b.hwnd, src_hwnd);
                if t == 0 {
                    continue;
                }
                let src = match wall_src {
                    Some((s, _)) => s,
                    None => {
                        let win = self.wins.iter().find(|w| w.hwnd == src_hwnd);
                        win.map_or(Rect::new(0, 0, dest.w(), dest.h()), |w| ov::source_rect(&w.window, &w.frame, thumb_size(t)))
                    }
                };
                set.thumbs.push((t, dest, src));
            }
            b.sets.push(set);
        }
        self.place_band(m, self.motion.pos(self.now()));
        Ok(())
    }

    /// Shows monitor `m`'s background for the highlight at `pos`: the desktops in view slide (or
    /// cross-fade over the edge).
    fn place_band(&mut self, m: usize, pos: f64) {
        let (mon, work) = (self.monitors[m].rect, self.monitors[m].work);
        let gap = (24.0 * self.monitors[m].dpi_scale).round() as i32;
        let view = ov::backdrop(pos, self.jump, self.desks.len(), mon.w() + gap);
        let bounds = Rect::new(0, 0, work.w(), work.h());
        let mut first = true;
        let b = &mut self.bands[m];
        for (d, set) in b.sets.iter_mut().enumerate() {
            let Some(&(_, dx, w)) = view.iter().find(|x| x.0 == d) else {
                if set.shown {
                    for t in &set.thumbs {
                        place_thumb(t.0, &t.1, &t.2, 0, false);
                    }
                    set.shown = false;
                }
                continue;
            };
            // The second one over the first with its weight (a cross-fade over the edge).
            let alpha = if first { 255 } else { (w * 255.0).round() as u8 };
            first = false;
            for t in &set.thumbs {
                place_clipped(t.0, t.1.offset(mon.left - work.left + dx, mon.top - work.top), t.2, &bounds, alpha);
            }
            set.shown = true;
        }
    }
    /// The carried window 1:1 in its place, over the glass (a window of its own at the frame).
    fn build_standin(&mut self) -> Result<(), String> {
        let Some((cw, cf)) = self.carried_rects else { return Err("no rectangle".into()) };
        let h = unsafe { layer(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TRANSPARENT, &cf, KIND_BLUR) };
        if h.is_null() {
            return Err("CreateWindowEx failed".into());
        }
        self.standin = h;
        self.standin_thumb = thumb(h, self.carried);
        if self.standin_thumb == 0 {
            // Without it the window would vanish under the glass.
            return Err("thumbnail of the carried window failed".into());
        }
        let src = ov::source_rect(&cw, &cf, thumb_size(self.standin_thumb));
        place_thumb(self.standin_thumb, &Rect::new(0, 0, cf.w(), cf.h()), &src, 255, true);
        unsafe { ShowWindow(h, SW_SHOWNOACTIVATE) };
        Ok(())
    }

    /// The target moves to desktop `target` (the highlight and the background follow).
    fn go_to(&mut self, target: usize) {
        let target = target.min(self.desks.len().saturating_sub(1));
        if target == self.target {
            return;
        }
        let now = self.now();
        let old = self.target;
        // A step during a jump: the jump ends at once and the step starts from its end.
        if let Some((_, to)) = self.jump.take() {
            self.motion = Motion::at(to as f64);
        }
        self.target = target;
        let (dur, jump) = ov::move_ms(old, target, self.animate);
        self.motion.retarget(target as f64, now, dur, jump);
        self.settled = false;
        if jump && dur > 0.0 {
            self.jump = Some((old, target));
        }
    }

    fn build_strip(&mut self) -> Result<(), String> {
        let m = self.strip_mon;
        let mon = &self.monitors[m];
        let mr = mon.rect;
        self.layout = ov::strip(&mon.work, &mon.rect, self.desks.len(), self.plus, mon.dpi_scale);
        let radius = (12.0 * mon.dpi_scale) as i32;
        let panel = self.layout.panel;
        let h = unsafe { layer(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT, &panel, KIND_STRIP) };
        if h.is_null() {
            return Err("CreateWindowEx failed".into());
        }
        unsafe {
            SetLayeredWindowAttributes(h, 0, (self.opacity * 255.0).round() as u8, LWA_ALPHA);
            SetWindowRgn(h, CreateRoundRectRgn(0, 0, panel.w() + 1, panel.h() + 1, radius, radius), 1);
        }
        let old = std::mem::replace(&mut self.strip, h);
        for t in self.tiles.iter_mut() {
            unthumb(t);
        }
        unthumb(&mut self.strip_carried);
        self.tiles.clear();
        for d in 0..self.desks.len() {
            let t = match self.panel(m, d) {
                Some(k) => thumb(h, self.panels[k].hwnd),
                None => 0,
            };
            self.tiles.push(t);
        }
        // The carried window in the target's preview, over the previews.
        if self.carried_rects.is_some_and(|(_, f)| f.intersects(&mr)) {
            self.strip_carried = thumb(h, self.carried);
        }
        self.place_tiles(self.motion.pos(self.now()));
        unsafe { ShowWindow(h, SW_SHOWNOACTIVATE) };
        if !old.is_null() {
            unsafe { DestroyWindow(old) };
        }
        Ok(())
    }

    fn place_tiles(&self, pos: f64) {
        let p = self.layout.panel;
        let mr = self.monitors[self.strip_mon].rect;
        for (d, t) in self.tiles.iter().enumerate() {
            let r = ov::zoomed(&self.layout.tiles[d], ov::tile_zoom(d, pos)).offset(-p.left, -p.top);
            place_thumb(*t, &r, &Rect::new(0, 0, mr.w(), mr.h()), 255, true);
        }
        if let (Some((cw, cf)), Some(tile)) = (self.carried_rects, self.layout.tiles.get(self.target)) {
            let r = ov::zoomed(tile, ov::tile_zoom(self.target, pos)).offset(-p.left, -p.top);
            let src = ov::source_rect(&cw, &cf, thumb_size(self.strip_carried));
            place_thumb(self.strip_carried, &ov::map_into(&r, &mr, &cf), &src, 255, true);
        }
    }

    /// Opacity of everything that fades (0..=1): background, glass, row.
    fn set_opacity(&mut self, a: f64) {
        if (a - self.opacity).abs() < 1e-4 {
            return;
        }
        self.opacity = a;
        let byte = (a * 255.0).round() as u8;
        for b in &self.bands {
            unsafe { SetLayeredWindowAttributes(b.hwnd, 0, byte, LWA_ALPHA) };
        }
        for g in &self.glasses {
            match &g.glass {
                Some(x) => x.set_opacity(a as f32),
                None => unsafe {
                    SetLayeredWindowAttributes(g.hwnd, 0, (a * DIM_ALPHA * 255.0).round() as u8, LWA_ALPHA);
                },
            }
        }
        if !self.strip.is_null() {
            unsafe { SetLayeredWindowAttributes(self.strip, 0, byte, LWA_ALPHA) };
        }
    }

    fn release(&mut self) {
        for b in self.bands.iter_mut() {
            for s in b.sets.iter_mut() {
                for t in s.thumbs.iter_mut() {
                    unthumb(&mut t.0);
                }
            }
            unsafe { DestroyWindow(b.hwnd) };
        }
        self.bands.clear();
        for g in self.glasses.drain(..) {
            drop(g.glass);
            unsafe { DestroyWindow(g.hwnd) };
        }
        unthumb(&mut self.standin_thumb);
        if !self.standin.is_null() {
            unsafe { DestroyWindow(self.standin) };
            self.standin = std::ptr::null_mut();
        }
        for t in self.tiles.iter_mut() {
            unthumb(t);
        }
        unthumb(&mut self.strip_carried);
        if !self.strip.is_null() {
            unsafe { DestroyWindow(self.strip) };
            self.strip = std::ptr::null_mut();
        }
        for p in self.panels.iter_mut() {
            for t in p.thumbs.iter_mut() {
                unthumb(t);
            }
            unsafe { DestroyWindow(p.hwnd) };
        }
        self.panels.clear();
        for w in self.walls.drain(..) {
            unsafe {
                DestroyWindow(w.hwnd);
                DeleteObject(w.bitmap);
            }
        }
    }

    /// Frame statistics: (frames, average ms, longest ms).
    fn stats(&self) -> Option<(usize, f64, f64)> {
        if self.gaps.is_empty() {
            return None;
        }
        let n = self.gaps.len();
        let avg = self.gaps.iter().map(|g| *g as f64).sum::<f64>() / n as f64;
        let max = self.gaps.iter().fold(0f32, |a, b| a.max(*b)) as f64;
        Some((n, avg, max))
    }
}

/// A DIB section w×h filled with `px` (BGR) or `color` (COLORREF).
unsafe fn dib(w: i32, h: i32, px: Option<&[u32]>, color: u32) -> HBITMAP {
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(std::ptr::null_mut(), &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
    if bmp.is_null() || bits.is_null() {
        return bmp;
    }
    let n = (w * h) as usize;
    let out = std::slice::from_raw_parts_mut(bits as *mut u32, n);
    match px {
        Some(p) if p.len() == n => out.copy_from_slice(p),
        _ => out.fill((color & 0xFF) << 16 | (color & 0xFF00) | (color >> 16 & 0xFF)),
    }
    bmp
}

/// The target moved (a step) to desktop `target` of `desktops`; a new desktop gets a preview.
pub fn set_target(target: usize, desktops: &[Guid], word: &str) {
    let Some(s) = state() else { return };
    if s.fade.is_some_and(|f| !f.0) {
        return; // fading away
    }
    if desktops.len() != s.desks.len() && !desktops.is_empty() {
        for (i, g) in desktops.iter().enumerate().skip(s.desks.len()) {
            s.desks.push(Desk { id: *g, label: ov::desktop_label(i, None, word), wallpaper: wallpaper_of(g) });
        }
        s.desks.truncate(desktops.len());
        if s.ensure_panels().is_err() {
            return;
        }
        for m in s.background_monitors() {
            if s.build_band(m).is_err() {
                return;
            }
        }
        if s.build_strip().is_err() {
            return;
        }
        s.load_wallpapers();
    }
    s.go_to(target);
}

/// Fades in layers built invisible (`Begin::hidden`); nothing when they are shown already.
pub fn reveal() {
    if let Some(s) = state() {
        if s.fade.is_none() && s.opacity <= 0.0 {
            s.fade = Some((true, s.now()));
        }
    }
}

/// The layers are fading away (a new carry must not reuse them).
pub fn closing() -> bool {
    state().is_some_and(|s| s.fade.is_some_and(|f| !f.0))
}

/// The carry ended: `shown` = the desktop now in view (after the drop's switch, or the start
/// after Esc). The background goes there while everything fades away.
pub fn finish(shown: Option<usize>) {
    let Some(s) = state() else { return };
    if s.fade.is_some_and(|f| !f.0) {
        return;
    }
    if s.fade.is_none() && s.opacity <= 0.0 {
        destroy(); // never shown (a preview being prepared)
        return;
    }
    if let Some(d) = shown.filter(|d| *d < s.desks.len()) {
        s.go_to(d);
    }
    let now = s.now();
    // Fading out from the opacity reached so far.
    let start = match s.fade {
        Some((true, t0)) => {
            let a = ov::fade(now - t0, true);
            now - ov::FADE_OUT_MS * (1.0 - a)
        }
        _ => now,
    };
    s.fade = Some((false, start));
}

/// Removes everything at once (exit, Explorer restart, a new carry).
pub fn destroy() {
    unsafe {
        if let Some(mut s) = (*std::ptr::addr_of_mut!(STATE)).take() {
            s.release();
        }
    }
}

/// One frame: moves the highlight and the background, fades, then waits for the next
/// composition (vsync). Returns a line for the log when the layers are gone.
pub fn frame() -> Option<String> {
    let s = state()?;
    let now = s.now();
    if let Some(last) = s.last_frame {
        if s.gaps.len() < 4000 {
            s.gaps.push((now - last) as f32);
        }
    }
    s.last_frame = Some(now);
    if !s.settled {
        let pos = s.motion.pos(now);
        if s.motion.done(now) {
            s.jump = None;
            s.settled = true; // this frame draws the final position
        }
        for m in s.background_monitors() {
            s.place_band(m, pos);
        }
        s.place_tiles(pos);
        unsafe {
            InvalidateRect(s.strip, std::ptr::null(), 0);
            UpdateWindow(s.strip);
        }
    }
    let mut gone = false;
    if let Some((inn, t0)) = s.fade {
        let a = ov::fade(now - t0, inn);
        s.set_opacity(a);
        if inn && a >= 1.0 {
            s.fade = None;
        } else if !inn && a <= 0.0 {
            gone = true;
        }
    }
    if gone {
        let line = s.stats().map(|(n, avg, max)| format!("carry: overlay {n} frames, average {avg:.1} ms, longest {max:.1} ms"));
        destroy();
        return line;
    }
    if s.work.len() < 4000 {
        s.work.push((s.now() - now) as f32);
    }
    if unsafe { DwmFlush() } < 0 {
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    if !s.busy(s.now()) {
        s.last_frame = None; // idle: the next motion starts a new series of frames
    }
    None
}

impl Overlay {
    unsafe fn paint_strip(&self, dc: HDC) {
        let p = self.layout.panel;
        let (w, h) = (p.w(), p.h());
        let mem = CreateCompatibleDC(dc);
        let bmp = CreateCompatibleBitmap(dc, w, h);
        let old = SelectObject(mem, bmp);
        let scale = self.monitors[self.strip_mon].dpi_scale;
        let px = |v: f64| (v * scale).round() as i32;
        let rgb = |r: u8, g: u8, b: u8| r as u32 | (g as u32) << 8 | (b as u32) << 16;
        let (back, text, dim, accent) = if self.dark {
            (rgb(32, 32, 32), rgb(240, 240, 240), rgb(90, 90, 90), rgb(76, 194, 255))
        } else {
            (rgb(243, 243, 243), rgb(26, 26, 26), rgb(170, 170, 170), rgb(0, 95, 184))
        };
        let b = CreateSolidBrush(back);
        FillRect(mem, &RECT { left: 0, top: 0, right: w, bottom: h }, b);
        DeleteObject(b);
        SetBkMode(mem, TRANSPARENT as i32);
        let pos = self.motion.pos(self.now());
        let font = CreateFontW(
            -px(13.0), 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32, DEFAULT_PITCH as u32, wide("Segoe UI").as_ptr(),
        );
        let bold = CreateFontW(
            -px(13.0), 0, 0, 0, FW_SEMIBOLD as i32, 0, 0, 0, DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32, DEFAULT_PITCH as u32, wide("Segoe UI").as_ptr(),
        );
        let of = SelectObject(mem, font);
        for (d, t) in self.layout.tiles.iter().enumerate() {
            let r = t.offset(-p.left, -p.top);
            let near = (d as f64 - pos).abs() < 0.5;
            SelectObject(mem, if near { bold } else { font });
            SetTextColor(mem, text);
            let label = wide(&self.desks[d].label);
            let mut lr = RECT { left: r.left - px(4.0), top: r.bottom + px(4.0), right: r.right + px(4.0), bottom: r.bottom + self.layout.label_h };
            DrawTextW(mem, label.as_ptr(), label.len() as i32 - 1, &mut lr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
        }
        if let Some(plus) = self.layout.plus {
            let r = plus.offset(-p.left, -p.top);
            let pen = CreatePen(PS_DOT, 1, dim);
            let op = SelectObject(mem, pen);
            let ob = SelectObject(mem, GetStockObject(NULL_BRUSH));
            RoundRect(mem, r.left, r.top, r.right, r.bottom, px(8.0), px(8.0));
            SelectObject(mem, ob);
            SelectObject(mem, op);
            DeleteObject(pen);
            SelectObject(mem, font);
            SetTextColor(mem, dim);
            let s = wide("+");
            let mut rr = rc(&r);
            DrawTextW(mem, s.as_ptr(), 1, &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }
        if let Some(hl) = ov::highlight(&self.layout.tiles, pos) {
            let g = px(5.0);
            let r = hl.offset(-p.left, -p.top);
            let pen = CreatePen(PS_SOLID, px(3.0).max(2), accent);
            let op = SelectObject(mem, pen);
            let ob = SelectObject(mem, GetStockObject(NULL_BRUSH));
            RoundRect(mem, r.left - g, r.top - g, r.right + g, r.bottom + g, px(10.0), px(10.0));
            SelectObject(mem, ob);
            SelectObject(mem, op);
            DeleteObject(pen);
        }
        SelectObject(mem, of);
        DeleteObject(font);
        DeleteObject(bold);
        BitBlt(dc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
    }

    /// The walls' wallpapers: from the cache at once, the others decoded on a worker thread (WIC,
    /// small) and repainted when done. The cache keeps the small pictures between carries (the
    /// key has the file's time, so a changed file is decoded again).
    fn load_wallpapers(&mut self) {
        let mut jobs: Vec<(usize, usize, String, WallJob)> = Vec::new();
        let mut ready: Vec<(usize, usize, Vec<u32>)> = Vec::new();
        for k in 0..self.walls.len() {
            let (mon, desk, size, scale) = (self.walls[k].mon, self.walls[k].desk, self.walls[k].size, self.walls[k].scale);
            if std::mem::replace(&mut self.walls[k].queued, true) {
                continue;
            }
            let Some(p) = self.desks[desk].wallpaper.clone() else { continue };
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            let key = format!("{p}|{mtime}|{}x{}|{:?}|{}", size.0, size.1, self.placement, self.color);
            match cache_get(&key) {
                Some(px) => ready.push((mon, desk, px)),
                None => jobs.push((mon, desk, key, WallJob { path: p, w: size.0, h: size.1, scale })),
            }
        }
        if !ready.is_empty() {
            if let Ok(mut v) = self.decoded.lock() {
                v.extend(ready.into_iter().map(|(m, d, px)| (m, d, String::new(), px)));
            }
            self.wallpapers_ready();
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_OVERLAY_DEMO_NOWALL").is_some() {
            return;
        }
        if jobs.is_empty() {
            return;
        }
        let out = self.decoded.clone();
        let (placement, color, generation) = (self.placement, self.color, self.generation);
        let notify = self.strip as usize;
        std::thread::spawn(move || {
            let com = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) } >= 0;
            let mut done: Vec<(String, Vec<u32>)> = Vec::new();
            for (mon, desk, key, job) in jobs {
                let px = match done.iter().find(|d| d.0 == key) {
                    Some(d) => Some(d.1.clone()),
                    None => compose(&job, placement, color),
                };
                if let Some(px) = px {
                    if !done.iter().any(|d| d.0 == key) {
                        done.push((key.clone(), px.clone()));
                    }
                    if let Ok(mut v) = out.lock() {
                        v.push((mon, desk, key, px));
                    }
                }
            }
            if com {
                unsafe { CoUninitialize() };
            }
            unsafe { PostMessageW(notify as HWND, WM_WALLPAPERS, generation, 0) };
        });
    }

    fn wallpapers_ready(&mut self) {
        let list = match self.decoded.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(_) => return,
        };
        for (mon, desk, key, px) in list {
            let Some(m) = self.walls.iter_mut().find(|m| m.mon == mon && m.desk == desk) else { continue };
            let bmp = unsafe { dib(m.size.0, m.size.1, Some(&px), self.color) };
            if !key.is_empty() {
                cache_put(key, px);
            }
            if bmp.is_null() {
                continue;
            }
            let old = std::mem::replace(&mut m.bitmap, bmp);
            unsafe {
                SetWindowLongPtrW(m.hwnd, GWLP_USERDATA, bmp as isize);
                DeleteObject(old);
                InvalidateRect(m.hwnd, std::ptr::null(), 0);
                UpdateWindow(m.hwnd);
            }
        }
    }
}

// ---------------------------------------------------------------- debug: demo without carrying

/// Debug builds only: draws the layers over the current desktop and moves the highlight without
/// carrying, switching or moving anything; saves screenshots into `shots` (BMP) and returns the
/// frame statistics.
#[cfg(debug_assertions)]
pub fn demo(carried: HWND, desktops: Vec<Guid>, current: usize, full: bool, desk_of: &dyn Fn(HWND) -> Option<Guid>, shots: Option<&std::path::Path>) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, MSG, PM_REMOVE};
    let n = desktops.len();
    // The window as it really is on the screen, to compare with its stand-in (sharpness).
    let pump = |ms: u64| {
        let t = Instant::now();
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while t.elapsed().as_millis() < ms as u128 {
            unsafe {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    DispatchMessageW(&msg);
                }
                DwmFlush();
            }
        }
    };
    pump(250);
    let geometry = frame_bounds(carried);
    let real = geometry.map(|(_, f)| grab(&f));
    let mut sharp = String::new();
    let started = Instant::now();
    let r = begin(Begin { carried, desktops: desktops.clone(), start: current, target: current, create_new: false, full, hidden: false, desk_of, word: "Plocha" });
    if let Err(e) = r {
        return format!("overlay demo: begin failed: {e}");
    }
    let setup_ms = started.elapsed().as_secs_f64() * 1000.0;
    // (time ms, action): 1 = right, -1 = left, 9 = capture, 0 = finish
    let mut target = current;
    let script: Vec<(u64, i32, &str)> = vec![
        (700, 9, "overlay-1-blur-and-row.bmp"),
        (900, 1, ""),
        (980, 9, "overlay-2-highlight-moving.bmp"),
        (1400, 9, "overlay-3-highlight-target.bmp"),
        (1700, 1, ""),
        (1760, 1, ""),
        (2300, 1, ""),
        (2330, 9, "overlay-4-wrap-jump.bmp"),
        (2800, -1, ""),
        (3400, 0, ""),
        (3470, 9, "overlay-5-fading-out.bmp"),
    ];
    let mut next = 0;
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    let mut gaps_all: Vec<f32> = Vec::new();
    let mut work_all: Vec<f32> = Vec::new();
    loop {
        unsafe {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                DispatchMessageW(&msg);
            }
        }
        let t = started.elapsed().as_millis() as u64;
        while next < script.len() && t >= script[next].0 {
            let (_, a, name) = script[next];
            match a {
                1 => target = (target + 1) % n,
                -1 => target = (target + n - 1) % n,
                9 => {
                    if let Some(dir) = shots {
                        capture(&dir.join(name));
                    }
                    if name.starts_with("overlay-3") {
                        sharp += &debug_state();
                        sharp += "\n";
                        if let (Some((w, f)), Some(real)) = (geometry, real.as_ref()) {
                            let now = grab(&f);
                            let (k, max) = diff(real, &now);
                            let src = state().filter(|s| s.standin_thumb != 0).map(|s| thumb_size(s.standin_thumb));
                            sharp += &format!(
                                "stand-in vs real window: {k} of {} pixels differ (largest channel difference {max}); window {w:?}, frame {f:?}, thumbnail source size {src:?}",
                                now.len()
                            );
                            if let Some(dir) = shots {
                                write_bmp(&dir.join("window-real.bmp"), f.w(), f.h(), real);
                                write_bmp(&dir.join("window-standin.bmp"), f.w(), f.h(), &now);
                            }
                        }
                    }
                }
                _ => finish(None),
            }
            if a == 1 || a == -1 {
                set_target(target, &desktops, "Plocha");
            }
            next += 1;
        }
        if !active() {
            break;
        }
        if t > 8000 {
            destroy();
            break;
        }
        if !animating() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }
        let line = frame();
        if let Some(s) = state() {
            gaps_all.append(&mut s.gaps);
            work_all.append(&mut s.work);
        }
        if line.is_some() || !active() {
            break;
        }
    }
    sharp += &format!("\nframes over 10 ms: {}", gaps_all.iter().filter(|g| **g > 10.0).count());
    let mut wi: Vec<(usize, f32)> = work_all.iter().cloned().enumerate().collect();
    wi.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sharp += &format!("\nlongest work frames (index, ms) of {}: {:?}", work_all.len(), &wi[..wi.len().min(6)]);
    let mut g: Vec<f32> = gaps_all.into_iter().filter(|x| *x < 100.0).collect();
    g.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if g.is_empty() {
        return format!("overlay demo: setup {setup_ms:.0} ms, no frames");
    }
    let avg = g.iter().map(|x| *x as f64).sum::<f64>() / g.len() as f64;
    format!(
        "{sharp}\nwork per frame: average {:.2} ms, longest {:.2} ms\noverlay demo: setup {setup_ms:.0} ms; {} frames while animating: average {avg:.2} ms, median {:.2}, p95 {:.2}, longest {:.2}",
        work_all.iter().map(|x| *x as f64).sum::<f64>() / work_all.len().max(1) as f64,
        work_all.iter().fold(0f32, |a, b| a.max(*b)),
        g.len(),
        g[g.len() / 2],
        g[g.len() * 95 / 100],
        g[g.len() - 1]
    )
}

/// Debug builds: what the layers look like now (for the demo's report).
#[cfg(debug_assertions)]
pub fn debug_state() -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetLayeredWindowAttributes;
    let Some(s) = state() else { return "no overlay".into() };
    let info = |h: HWND| unsafe {
        let mut a = 0u8;
        let mut k = 0u32;
        let mut f = 0u32;
        GetLayeredWindowAttributes(h, &mut k, &mut a, &mut f);
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(h, &mut r);
        let mut cl = 0u32;
        DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, (&mut cl as *mut u32).cast(), 4);
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let top = windows_sys::Win32::UI::WindowsAndMessaging::WindowFromPoint(windows_sys::Win32::Foundation::POINT { x: r.left + 200, y: r.top + 200 });
        format!("{:?} visible {} alpha {a} cloaked {cl} ex {ex:#x} rect {:?} window at point {:?}", h, IsWindowVisible(h), from_rc(&r), top)
    };
    let mut out = format!("opacity {:.2} target {} pos {:.3} jump {:?} busy {} panels {} walls {}", s.opacity, s.target, s.motion.pos(s.now()), s.jump, s.busy(s.now()), s.panels.len(), s.walls.len());
    for b in &s.bands {
        out += &format!("\n band {} sets {}", info(b.hwnd), b.sets.len());
    }
    for g in &s.glasses {
        out += &format!("\n glass {} acrylic {}", info(g.hwnd), g.glass.is_some());
    }
    out
}

/// Debug builds: pixels of a screen rectangle (32-bit BGR, top-down).
#[cfg(debug_assertions)]
pub fn grab(r: &Rect) -> Vec<u32> {
    use windows_sys::Win32::Graphics::Gdi::{GdiFlush, GetDC, ReleaseDC, CAPTUREBLT};
    let (w, h) = (r.w().max(1), r.h().max(1));
    unsafe {
        let sdc = GetDC(std::ptr::null_mut());
        let mdc = CreateCompatibleDC(sdc);
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = 40;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(mdc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old = SelectObject(mdc, bmp);
        BitBlt(mdc, 0, 0, w, h, sdc, r.left, r.top, SRCCOPY | CAPTUREBLT);
        GdiFlush();
        let out = if bits.is_null() { vec![0; (w * h) as usize] } else { std::slice::from_raw_parts(bits as *const u32, (w * h) as usize).to_vec() };
        SelectObject(mdc, old);
        DeleteObject(bmp);
        DeleteDC(mdc);
        ReleaseDC(std::ptr::null_mut(), sdc);
        out
    }
}

/// Debug builds: writes pixels as a BMP file.
#[cfg(debug_assertions)]
pub fn write_bmp(path: &std::path::Path, w: i32, h: i32, px: &[u32]) {
    let mut f = Vec::with_capacity(px.len() * 4 + 54);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(54 + px.len() as u32 * 4).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&(-h).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&[0u8; 24]);
    for p in px {
        f.extend_from_slice(&(p | 0xFF00_0000).to_le_bytes());
    }
    let _ = std::fs::write(path, f);
}

/// Debug builds: the whole (virtual) screen into a BMP file.
#[cfg(debug_assertions)]
pub fn capture(path: &std::path::Path) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};
    let (x, y, w, h) = unsafe {
        (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN))
    };
    let px = grab(&Rect::new(x, y, x + w, y + h));
    write_bmp(path, w, h, &px);
}

/// Debug builds: how many pixels of two grabs differ (ignoring the alpha byte) and the largest
/// difference of one channel.
#[cfg(debug_assertions)]
pub fn diff(a: &[u32], b: &[u32]) -> (usize, u32) {
    let mut n = 0;
    let mut max = 0;
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x & 0xFF_FFFF, y & 0xFF_FFFF);
        if x != y {
            n += 1;
            for s in [0, 8, 16] {
                max = max.max(((x >> s) & 0xFF).abs_diff((y >> s) & 0xFF));
            }
        }
    }
    (n, max)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// WIC decoding of a system wallpaper (skipped where the file does not exist): the JPEG is
    /// scaled while decoding, the result has the asked size and is not one flat colour.
    #[test]
    fn decodes_a_jpeg_wallpaper_small() {
        let path = r"C:\Windows\Web\Wallpaper\ThemeA\img20.jpg";
        if !std::path::Path::new(path).exists() {
            return;
        }
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
        let (px, w, h, iw, ih) = decode(path, |_, _| (320, 180)).expect("decoded");
        assert_eq!((w, h, px.len()), (320, 180, 320 * 180));
        assert!(iw >= 320 && ih >= 180);
        assert!(px.iter().any(|p| *p != px[0]));
        let job = WallJob { path: path.into(), w: 320, h: 180, scale: 8 };
        assert_eq!(compose(&job, Placement::Fit, 0x00FF00).map(|v| v.len()), Some(320 * 180));
        assert!(decode(r"C:\nonexistent\x.jpg", |_, _| (10, 10)).is_none());
    }
}