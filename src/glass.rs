//! Acrylic glass for the carry overlay (plan 12.13): the system's host backdrop brush of
//! Windows.UI.Composition (documented; `DWMWA_USE_HOSTBACKDROPBRUSH`, Windows 11) blurs live
//! whatever DWM composes below the window – also windows that are never active – plus a tint.
//!
//! Only windows-sys: the WinRT functions are loaded at run time (combase.dll, CoreMessaging.dll)
//! and the interfaces are called through their vtables. Slot numbers come from the Windows SDK
//! metadata (`C:\Windows\System32\WinMetadata\Windows.UI.winmd`, read 2026-11-24): the methods of
//! a WinRT interface follow the six of IInspectable in metadata order.

use crate::desktops::Guid;
use crate::tray::wide;
use std::ffi::c_void;
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

type Raw = *mut c_void;

/// Owned COM / WinRT interface pointer.
struct Com(Raw);

impl Com {
    unsafe fn f<F: Copy>(&self, slot: usize) -> F {
        let vtbl = *(self.0 as *const *const *const c_void);
        std::mem::transmute_copy(&*vtbl.add(slot))
    }

    fn qi(&self, iid: &str) -> Result<Com, String> {
        type Qi = unsafe extern "system" fn(Raw, *const Guid, *mut Raw) -> i32;
        let g = Guid::parse(iid).unwrap_or_default();
        let mut p: Raw = std::ptr::null_mut();
        let hr = unsafe { self.f::<Qi>(0)(self.0, &g, &mut p) };
        if hr < 0 || p.is_null() {
            return Err(format!("QueryInterface {iid}: {hr:#x}"));
        }
        Ok(Com(p))
    }

    /// A method whose only argument is an [out] interface.
    fn out(&self, slot: usize, what: &str) -> Result<Com, String> {
        type F = unsafe extern "system" fn(Raw, *mut Raw) -> i32;
        let mut p: Raw = std::ptr::null_mut();
        let hr = unsafe { self.f::<F>(slot)(self.0, &mut p) };
        if hr < 0 || p.is_null() {
            return Err(format!("{what}: {hr:#x}"));
        }
        Ok(Com(p))
    }

    fn put(&self, slot: usize, arg: &Com, what: &str) -> Result<(), String> {
        type F = unsafe extern "system" fn(Raw, Raw) -> i32;
        let hr = unsafe { self.f::<F>(slot)(self.0, arg.0) };
        if hr < 0 {
            return Err(format!("{what}: {hr:#x}"));
        }
        Ok(())
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

// Interfaces (Windows.UI.Composition unless said otherwise).
const IID_COMPOSITOR: &str = "B403CA50-7F8C-4E83-985F-CC45060036D8";
const IID_COMPOSITOR3: &str = "C9DD8EF0-6EB1-4E3C-A658-675D9C64D4AB";
/// ICompositorDesktopInterop (windows.ui.composition.interop.h).
const IID_DESKTOP_INTEROP: &str = "29E691FA-4567-4DCA-B319-D0F207EB6807";
const IID_TARGET: &str = "A1BEA8BA-D726-4663-8129-6B5E7927FFA6";
const IID_VISUAL: &str = "117E202D-A859-4C89-873B-C2AA566788E3";
const IID_BRUSH: &str = "AB0D7608-30C0-40E9-B568-B60A6BD1FB46";
// Slots.
const COMPOSITOR_CREATE_COLOR_BRUSH_WITH_COLOR: usize = 8;
const COMPOSITOR_CREATE_CONTAINER_VISUAL: usize = 9;
const COMPOSITOR_CREATE_SPRITE_VISUAL: usize = 22;
const COMPOSITOR3_CREATE_HOST_BACKDROP_BRUSH: usize = 6;
const INTEROP_CREATE_DESKTOP_WINDOW_TARGET: usize = 3;
const TARGET_PUT_ROOT: usize = 7;
const CONTAINER_GET_CHILDREN: usize = 6;
const COLLECTION_INSERT_AT_TOP: usize = 9;
const SPRITE_PUT_BRUSH: usize = 7;
const VISUAL_PUT_OPACITY: usize = 23;
const VISUAL_PUT_SIZE: usize = 36;
/// DWMWINDOWATTRIBUTE DWMWA_USE_HOSTBACKDROPBRUSH (Windows 11, build 22000+).
const DWMWA_USE_HOSTBACKDROPBRUSH: u32 = 17;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vector2 {
    x: f32,
    y: f32,
}

/// Windows.UI.Color.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The thread's compositor (with the DispatcherQueue it needs). Made at the first carry and kept
/// for the life of the process: a thread can have only one DispatcherQueueController.
struct Compositor {
    _queue: Com,
    main: Com,
    c3: Com,
    interop: Com,
}

static mut COMPOSITOR: Option<Compositor> = None;
static mut FAILED: Option<String> = None;

unsafe fn proc_addr(dll: &str, name: &str) -> Option<unsafe extern "system" fn() -> isize> {
    let m = LoadLibraryW(wide(dll).as_ptr());
    if m.is_null() {
        return None;
    }
    let n: Vec<u8> = name.bytes().chain(Some(0)).collect();
    GetProcAddress(m, n.as_ptr())
}

unsafe fn make_compositor() -> Result<Compositor, String> {
    #[repr(C)]
    struct DispatcherQueueOptions {
        size: u32,
        thread_type: i32,   // DQTYPE_THREAD_CURRENT = 2
        apartment_type: i32, // DQTAT_COM_NONE = 0 (the UI thread is an STA already)
    }
    type CreateQueue = unsafe extern "system" fn(DispatcherQueueOptions, *mut Raw) -> i32;
    type CreateString = unsafe extern "system" fn(*const u16, u32, *mut Raw) -> i32;
    type DeleteString = unsafe extern "system" fn(Raw) -> i32;
    type Activate = unsafe extern "system" fn(Raw, *mut Raw) -> i32;
    let cq: CreateQueue = std::mem::transmute(proc_addr("CoreMessaging.dll", "CreateDispatcherQueueController").ok_or("CreateDispatcherQueueController missing")?);
    let cs: CreateString = std::mem::transmute(proc_addr("combase.dll", "WindowsCreateString").ok_or("WindowsCreateString missing")?);
    let ds: DeleteString = std::mem::transmute(proc_addr("combase.dll", "WindowsDeleteString").ok_or("WindowsDeleteString missing")?);
    let act: Activate = std::mem::transmute(proc_addr("combase.dll", "RoActivateInstance").ok_or("RoActivateInstance missing")?);
    let mut q: Raw = std::ptr::null_mut();
    let hr = cq(DispatcherQueueOptions { size: 12, thread_type: 2, apartment_type: 0 }, &mut q);
    if hr < 0 || q.is_null() {
        return Err(format!("CreateDispatcherQueueController: {hr:#x}"));
    }
    let queue = Com(q);
    let name: Vec<u16> = "Windows.UI.Composition.Compositor".encode_utf16().collect();
    let mut hs: Raw = std::ptr::null_mut();
    if cs(name.as_ptr(), name.len() as u32, &mut hs) < 0 {
        return Err("WindowsCreateString failed".into());
    }
    let mut obj: Raw = std::ptr::null_mut();
    let hr = act(hs, &mut obj);
    ds(hs);
    if hr < 0 || obj.is_null() {
        return Err(format!("RoActivateInstance(Compositor): {hr:#x}"));
    }
    let obj = Com(obj);
    Ok(Compositor { main: obj.qi(IID_COMPOSITOR)?, c3: obj.qi(IID_COMPOSITOR3)?, interop: obj.qi(IID_DESKTOP_INTEROP)?, _queue: queue })
}

fn compositor() -> Result<&'static Compositor, String> {
    unsafe {
        if let Some(e) = (*std::ptr::addr_of!(FAILED)).as_ref() {
            return Err(e.clone());
        }
        if (*std::ptr::addr_of!(COMPOSITOR)).is_none() {
            match make_compositor() {
                Ok(c) => *std::ptr::addr_of_mut!(COMPOSITOR) = Some(c),
                Err(e) => {
                    *std::ptr::addr_of_mut!(FAILED) = Some(e.clone());
                    return Err(e);
                }
            }
        }
        (*std::ptr::addr_of!(COMPOSITOR)).as_ref().ok_or_else(|| "no compositor".into())
    }
}

/// Glass can be used: Windows 11 (the host backdrop brush for desktop windows is there from build
/// 22000) and "Transparency effects" are on (off = the brush shows only a flat colour).
pub fn available() -> bool {
    // Debug builds: `AUTOSPACES_NO_GLASS=1` shows the dark layer used without acrylic.
    #[cfg(debug_assertions)]
    if std::env::var_os("AUTOSPACES_NO_GLASS").is_some() {
        return false;
    }
    crate::desktops::internal::os_build() >= 22000 && transparency_on()
}

fn transparency_on() -> bool {
    let mut v = 1u32;
    let mut len = 4u32;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("EnableTransparency");
    let r = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut len)
    };
    r != 0 || v != 0
}

/// The tint over the blur, close to the acrylic of Task View in the dark / light theme.
pub fn tint(dark: bool) -> Color {
    if dark {
        Color { a: 150, r: 28, g: 28, b: 30 }
    } else {
        Color { a: 140, r: 238, g: 238, b: 240 }
    }
}

/// Acrylic glass filling a window (which must have `WS_EX_NOREDIRECTIONBITMAP`).
pub struct Glass {
    _target: Com,
    root: Com,
}

impl Glass {
    /// Blurred backdrop + `tint` over the whole `w`×`h` window.
    pub fn attach(hwnd: HWND, w: f32, h: f32, tint: Color) -> Result<Glass, String> {
        let on: BOOL = 1;
        let hr = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, (&on as *const BOOL).cast(), 4) };
        if hr < 0 {
            return Err(format!("DWMWA_USE_HOSTBACKDROPBRUSH: {hr:#x}"));
        }
        let c = compositor()?;
        type CreateTarget = unsafe extern "system" fn(Raw, HWND, BOOL, *mut Raw) -> i32;
        type CreateColor = unsafe extern "system" fn(Raw, Color, *mut Raw) -> i32;
        type PutSize = unsafe extern "system" fn(Raw, Vector2) -> i32;
        unsafe {
            let mut t: Raw = std::ptr::null_mut();
            let hr = c.interop.f::<CreateTarget>(INTEROP_CREATE_DESKTOP_WINDOW_TARGET)(c.interop.0, hwnd, 1, &mut t);
            if hr < 0 || t.is_null() {
                return Err(format!("CreateDesktopWindowTarget: {hr:#x}"));
            }
            let target = Com(t).qi(IID_TARGET)?;
            let size = Vector2 { x: w, y: h };
            let root = c.main.out(COMPOSITOR_CREATE_CONTAINER_VISUAL, "CreateContainerVisual")?;
            let root_v = root.qi(IID_VISUAL)?;
            root_v.f::<PutSize>(VISUAL_PUT_SIZE)(root_v.0, size);
            let children = root.out(CONTAINER_GET_CHILDREN, "Children")?;
            // The system's blurred backdrop (what DWM composes below this window).
            let backdrop = c.main.out(COMPOSITOR_CREATE_SPRITE_VISUAL, "CreateSpriteVisual")?;
            let brush = c.c3.out(COMPOSITOR3_CREATE_HOST_BACKDROP_BRUSH, "CreateHostBackdropBrush")?.qi(IID_BRUSH)?;
            backdrop.put(SPRITE_PUT_BRUSH, &brush, "put_Brush")?;
            let backdrop_v = backdrop.qi(IID_VISUAL)?;
            backdrop_v.f::<PutSize>(VISUAL_PUT_SIZE)(backdrop_v.0, size);
            children.put(COLLECTION_INSERT_AT_TOP, &backdrop_v, "InsertAtTop")?;
            // The tint.
            let tinted = c.main.out(COMPOSITOR_CREATE_SPRITE_VISUAL, "CreateSpriteVisual")?;
            let mut cb: Raw = std::ptr::null_mut();
            let hr = c.main.f::<CreateColor>(COMPOSITOR_CREATE_COLOR_BRUSH_WITH_COLOR)(c.main.0, tint, &mut cb);
            if hr < 0 || cb.is_null() {
                return Err(format!("CreateColorBrush: {hr:#x}"));
            }
            let cb = Com(cb).qi(IID_BRUSH)?;
            tinted.put(SPRITE_PUT_BRUSH, &cb, "put_Brush")?;
            let tinted_v = tinted.qi(IID_VISUAL)?;
            tinted_v.f::<PutSize>(VISUAL_PUT_SIZE)(tinted_v.0, size);
            children.put(COLLECTION_INSERT_AT_TOP, &tinted_v, "InsertAtTop")?;
            target.put(TARGET_PUT_ROOT, &root_v, "put_Root")?;
            Ok(Glass { _target: target, root: root_v })
        }
    }

    /// Opacity of the whole glass (fading in and out).
    pub fn set_opacity(&self, a: f32) {
        type PutOpacity = unsafe extern "system" fn(Raw, f32) -> i32;
        unsafe { self.root.f::<PutOpacity>(VISUAL_PUT_OPACITY)(self.root.0, a.clamp(0.0, 1.0)) };
    }
}
