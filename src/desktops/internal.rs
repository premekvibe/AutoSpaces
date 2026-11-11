//! Level 1 backend: undocumented `IVirtualDesktopManagerInternal` in explorer.exe,
//! called through raw vtable slots described by `variants.json` (plan 5.1).
//!
//! All calls go through COM proxies (the objects live in explorer.exe) and must be
//! made from the thread that initialised the STA (the single UI thread).

use super::backend::{check, DesktopBackend, Error, Result};
use super::guid::Guid;
use super::registry;
use super::variants::{self, Variant};
use std::ffi::c_void;
use std::fmt;
use std::ptr::{null_mut, NonNull};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, CLSCTX_LOCAL_SERVER,
    COINIT_APARTMENTTHREADED,
};

type Raw = *mut c_void;
type Hr = i32;

const E_POINTER: Hr = 0x80004003u32 as i32;

const CLSID_IMMERSIVE_SHELL: &str = "C2F03A33-21F5-47FA-B4BB-156362A2F239";
const IID_SERVICE_PROVIDER: &str = "6D5140C1-7436-11CE-8034-00AA006009FA";
const SID_VDM_INTERNAL: &str = "C5E0CDCA-7B6E-41B2-9FC4-D93975CC467B";
const CLSID_VDM_PUBLIC: &str = "AA509086-5CA9-4C25-8F95-589D3C07B48A";
const IID_VDM_PUBLIC: &str = "A5CD92FF-29BE-454C-8D04-D82879FB3F1B";
const IID_OBJECT_ARRAY: &str = "92CA9DCD-5622-4BBA-A805-5E9F541BD8C9";

// Fixed slots of documented interfaces.
const SLOT_QI: usize = 0;
const SLOT_RELEASE: usize = 2;
const SLOT_QUERY_SERVICE: usize = 3; // IServiceProvider
const SLOT_ARRAY_COUNT: usize = 3; // IObjectArray::GetCount
const SLOT_ARRAY_GET_AT: usize = 4; // IObjectArray::GetAt
const SLOT_GET_WINDOW_DESKTOP_ID: usize = 4; // IVirtualDesktopManager

fn guid(s: &str) -> Guid {
    Guid::parse(s).unwrap_or_default()
}

// ---------------------------------------------------------------- COM plumbing

/// STA initialisation guard for the current thread.
pub struct Apartment(bool);

impl Apartment {
    pub fn init() -> Apartment {
        Apartment(unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Owned interface pointer (one reference). Released on drop.
struct ComPtr(NonNull<c_void>);

impl ComPtr {
    fn raw(&self) -> Raw {
        self.0.as_ptr()
    }

    /// Function pointer from vtable slot `idx`. `F` must be an `extern "system" fn`
    /// whose first argument is the interface pointer.
    unsafe fn slot<F: Copy>(&self, idx: usize) -> F {
        debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<usize>());
        let vtbl = *(self.raw() as *const *const *const c_void);
        let f = *vtbl.add(idx);
        std::mem::transmute_copy(&f)
    }

    fn query(&self, iid: &Guid) -> Result<ComPtr> {
        type Qi = unsafe extern "system" fn(Raw, *const Guid, *mut Raw) -> Hr;
        out_ptr(|p| unsafe { self.slot::<Qi>(SLOT_QI)(self.raw(), iid, p) })
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        type Release = unsafe extern "system" fn(Raw) -> u32;
        unsafe { self.slot::<Release>(SLOT_RELEASE)(self.raw()) };
    }
}

/// Runs a call with an `[out] IUnknown**` parameter and takes ownership of the result.
fn out_ptr(f: impl FnOnce(*mut Raw) -> Hr) -> Result<ComPtr> {
    let mut p: Raw = null_mut();
    let hr = f(&mut p);
    let obj = NonNull::new(p).map(ComPtr); // released on error too
    check(hr)?;
    obj.ok_or(Error::Permanent(E_POINTER))
}

fn create_instance(clsid: &str, ctx: u32, iid: &str) -> Result<ComPtr> {
    let (c, i) = (guid(clsid), guid(iid));
    out_ptr(|p| unsafe { CoCreateInstance(c.as_ptr(), null_mut(), ctx, i.as_ptr(), p) })
}

fn query_service(sp: &ComPtr, sid: &Guid, iid: &Guid) -> Result<ComPtr> {
    type Qs = unsafe extern "system" fn(Raw, *const Guid, *const Guid, *mut Raw) -> Hr;
    out_ptr(|p| unsafe { sp.slot::<Qs>(SLOT_QUERY_SERVICE)(sp.raw(), sid, iid, p) })
}

// ---------------------------------------------------------------- OS build

#[repr(C)]
struct OsVersionInfo {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd: [u16; 128],
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
}

pub fn os_build() -> u32 {
    let mut vi = OsVersionInfo { size: 0, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
    vi.size = std::mem::size_of::<OsVersionInfo>() as u32;
    if unsafe { RtlGetVersion(&mut vi) } >= 0 {
        vi.build
    } else {
        0
    }
}

// ---------------------------------------------------------------- backend

type FnOut = unsafe extern "system" fn(Raw, *mut Raw) -> Hr;
type FnMonOut = unsafe extern "system" fn(Raw, Raw, *mut Raw) -> Hr;
type FnCount = unsafe extern "system" fn(Raw, *mut i32) -> Hr;
type FnMonCount = unsafe extern "system" fn(Raw, Raw, *mut i32) -> Hr;
type FnIn1 = unsafe extern "system" fn(Raw, Raw) -> Hr;
type FnMonIn1 = unsafe extern "system" fn(Raw, Raw, Raw) -> Hr;
type FnIn2 = unsafe extern "system" fn(Raw, Raw, Raw) -> Hr;
type FnInOutBool = unsafe extern "system" fn(Raw, Raw, *mut i32) -> Hr;
type FnFind = unsafe extern "system" fn(Raw, *const Guid, *mut Raw) -> Hr;
type FnGuidOut = unsafe extern "system" fn(Raw, *mut Guid) -> Hr;
type FnHwndOut = unsafe extern "system" fn(Raw, HWND, *mut Raw) -> Hr;
type FnHwndGuidOut = unsafe extern "system" fn(Raw, HWND, *mut Guid) -> Hr;
type FnArrCount = unsafe extern "system" fn(Raw, *mut u32) -> Hr;
type FnArrGetAt = unsafe extern "system" fn(Raw, u32, *const Guid, *mut Raw) -> Hr;

pub struct InternalBackend {
    name: String,
    v: Variant,
    mgr: ComPtr,
    views: ComPtr,
    public: ComPtr,
}

/// No variant passed; carries diagnostics for extending `variants.json` (plan 5.3).
#[derive(Debug)]
pub struct Unsupported {
    pub build: u32,
    pub details: String,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported Windows build {}: {}", self.build, self.details)
    }
}

impl InternalBackend {
    /// Calls a "no-argument, [out] object" method, inserting a null HMONITOR when the variant needs it.
    fn call_out(&self, slot: usize) -> Result<ComPtr> {
        let m = &self.mgr;
        out_ptr(|p| unsafe {
            if self.v.monitor_param {
                m.slot::<FnMonOut>(slot)(m.raw(), null_mut(), p)
            } else {
                m.slot::<FnOut>(slot)(m.raw(), p)
            }
        })
    }

    fn count(&self) -> Result<i32> {
        let (m, mut n) = (&self.mgr, 0i32);
        check(unsafe {
            if self.v.monitor_param {
                m.slot::<FnMonCount>(self.v.slots.get_count)(m.raw(), null_mut(), &mut n)
            } else {
                m.slot::<FnCount>(self.v.slots.get_count)(m.raw(), &mut n)
            }
        })?;
        Ok(n)
    }

    fn desktop_id(&self, d: &ComPtr) -> Result<Guid> {
        let mut g = Guid::ZERO;
        check(unsafe { d.slot::<FnGuidOut>(self.v.desktop_get_id)(d.raw(), &mut g) })?;
        Ok(g)
    }

    fn find(&self, id: Guid) -> Result<ComPtr> {
        let m = &self.mgr;
        out_ptr(|p| unsafe { m.slot::<FnFind>(self.v.slots.find)(m.raw(), &id, p) })
    }

    /// `IApplicationView` for a window; only passed through, never called.
    fn view(&self, hwnd: HWND) -> Result<ComPtr> {
        let c = &self.views;
        out_ptr(|p| unsafe { c.slot::<FnHwndOut>(self.v.view_for_hwnd)(c.raw(), hwnd, p) })
    }

    fn call_in2(&self, slot: usize, a: &ComPtr, b: &ComPtr) -> Result<()> {
        let m = &self.mgr;
        check(unsafe { m.slot::<FnIn2>(slot)(m.raw(), a.raw(), b.raw()) })
    }

    /// Read-only verification against the registry (plan 5.1 step 4).
    fn self_test(&self, reg_ids: Option<&[Guid]>, reg_cur: Option<Guid>) -> std::result::Result<String, String> {
        let n = self.count().map_err(|e| format!("GetCount failed ({e})"))?;
        let expected = reg_ids.map_or(1, |l| l.len());
        if n < 1 || n as usize != expected {
            return Err(format!("GetCount={n}, registry has {expected}"));
        }
        let d = self.call_out(self.v.slots.get_current).map_err(|e| format!("GetCurrentDesktop failed ({e})"))?;
        // The returned object must really be an IVirtualDesktop of this variant.
        let d = d.query(&self.v.iid_desktop).map_err(|e| format!("current desktop QI failed ({e})"))?;
        let cur = self.desktop_id(&d).map_err(|e| format!("GetId failed ({e})"))?;
        match reg_cur {
            Some(rc) if rc != cur => Err(format!("current {cur} != registry {rc}")),
            None if reg_ids.is_some_and(|l| !l.contains(&cur)) => Err(format!("current {cur} not in registry list")),
            _ => Ok(format!("count {n}, current {cur}")),
        }
    }

    /// Picks the first variant whose build range matches, whose IID explorer accepts
    /// and which passes the read-only self-test. `diag` collects human-readable diagnostics.
    pub fn select(diag: &mut Vec<String>) -> std::result::Result<InternalBackend, Unsupported> {
        let build = os_build();
        let (mut list, src) = variants::load();
        diag.push(format!("build {build}, {src}, {} variant(s)", list.len()));
        let fail = |details: String| Unsupported { build, details };

        let shell = create_instance(CLSID_IMMERSIVE_SHELL, CLSCTX_LOCAL_SERVER, IID_SERVICE_PROVIDER)
            .map_err(|e| fail(format!("ImmersiveShell: {e}")))?;
        let reg_ids = registry::desktop_ids();
        let reg_cur = registry::current_desktop();
        diag.push(format!(
            "registry: {} desktop(s), current {}",
            reg_ids.as_ref().map_or("? (no VirtualDesktopIDs)".into(), |l| l.len().to_string()),
            reg_cur.map_or("?".into(), |g| g.to_string())
        ));

        // Most specific (newest) range first.
        list.sort_by_key(|v| std::cmp::Reverse(v.min_build));
        let sid = guid(SID_VDM_INTERNAL);
        let mut results = Vec::new();
        for v in list {
            let range = format!("{}..{}", v.min_build, v.max_build.map_or(String::new(), |m| m.to_string()));
            if !v.matches_build(build) {
                diag.push(format!("  {}: build range {range} does not match", v.name));
                results.push(format!("{}=range", v.name));
                continue;
            }
            let mgr = match query_service(&shell, &sid, &v.iid_manager) {
                Ok(m) => m,
                Err(e) => {
                    diag.push(format!("  {}: QueryService({}) -> 0x{:08X}", v.name, v.iid_manager, e.hr() as u32));
                    results.push(format!("{}=0x{:08X}", v.name, e.hr() as u32));
                    continue;
                }
            };
            let iid_vc = v.iid_view_collection;
            let views = match query_service(&shell, &iid_vc, &iid_vc) {
                Ok(c) => c,
                Err(e) => {
                    diag.push(format!("  {}: view collection -> 0x{:08X}", v.name, e.hr() as u32));
                    results.push(format!("{}=views 0x{:08X}", v.name, e.hr() as u32));
                    continue;
                }
            };
            let public = create_instance(CLSID_VDM_PUBLIC, CLSCTX_ALL, IID_VDM_PUBLIC)
                .map_err(|e| fail(format!("IVirtualDesktopManager: {e}")))?;
            let b = InternalBackend { name: format!("internal:{}", v.name), v, mgr, views, public };
            match b.self_test(reg_ids.as_deref(), reg_cur) {
                Ok(s) => {
                    diag.push(format!("  {}: QueryService OK, self-test PASS ({s})", b.v.name));
                    return Ok(b);
                }
                Err(s) => {
                    diag.push(format!("  {}: QueryService OK, self-test FAIL ({s})", b.v.name));
                    results.push(format!("{}=selftest", b.v.name));
                }
            }
        }
        Err(fail(if results.is_empty() { "no variants".into() } else { results.join(", ") }))
    }
}

impl DesktopBackend for InternalBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn desktops(&self) -> Result<Vec<Guid>> {
        let arr = self.call_out(self.v.slots.get_desktops)?;
        let arr = arr.query(&guid(IID_OBJECT_ARRAY))?;
        let mut n = 0u32;
        check(unsafe { arr.slot::<FnArrCount>(SLOT_ARRAY_COUNT)(arr.raw(), &mut n) })?;
        let iid = self.v.iid_desktop;
        (0..n)
            .map(|i| {
                let d = out_ptr(|p| unsafe { arr.slot::<FnArrGetAt>(SLOT_ARRAY_GET_AT)(arr.raw(), i, &iid, p) })?;
                self.desktop_id(&d)
            })
            .collect()
    }

    fn current(&self) -> Result<Guid> {
        let d = self.call_out(self.v.slots.get_current)?;
        self.desktop_id(&d)
    }

    fn can_move(&self, hwnd: HWND) -> Result<bool> {
        let view = match self.view(hwnd) {
            Ok(v) => v,
            Err(e) if e.is_transient() => return Err(e),
            Err(_) => return Ok(false), // no application view → not movable
        };
        let (m, mut ok) = (&self.mgr, 0i32);
        check(unsafe { m.slot::<FnInOutBool>(self.v.slots.can_view_move)(m.raw(), view.raw(), &mut ok) })?;
        Ok(ok != 0)
    }

    fn create(&self) -> Result<Guid> {
        let d = self.call_out(self.v.slots.create)?;
        self.desktop_id(&d)
    }

    fn move_window(&self, hwnd: HWND, desk: Guid) -> Result<()> {
        let view = self.view(hwnd)?;
        let d = self.find(desk)?;
        self.call_in2(self.v.slots.move_view, &view, &d)
    }

    fn switch(&self, desk: Guid, animated: bool) -> Result<()> {
        let d = self.find(desk)?;
        let m = &self.mgr;
        let anim = if animated { self.v.slots.opt("SwitchDesktopWithAnimation") } else { None };
        check(unsafe {
            match anim {
                Some(slot) => m.slot::<FnIn1>(slot)(m.raw(), d.raw()),
                None if self.v.monitor_param => m.slot::<FnMonIn1>(self.v.slots.switch)(m.raw(), null_mut(), d.raw()),
                None => m.slot::<FnIn1>(self.v.slots.switch)(m.raw(), d.raw()),
            }
        })
    }

    fn remove(&self, desk: Guid, fallback: Guid) -> Result<()> {
        let d = self.find(desk)?;
        let f = self.find(fallback)?;
        self.call_in2(self.v.slots.remove, &d, &f)
    }

    fn window_desktop(&self, hwnd: HWND) -> Option<Guid> {
        // Any failure (incl. TYPE_E_ELEMENTNOTFOUND 0x8002802B for hidden windows) → None.
        let (p, mut g) = (&self.public, Guid::ZERO);
        let hr = unsafe { p.slot::<FnHwndGuidOut>(SLOT_GET_WINDOW_DESKTOP_ID)(p.raw(), hwnd, &mut g) };
        (hr >= 0 && !g.is_zero()).then_some(g)
    }
}
