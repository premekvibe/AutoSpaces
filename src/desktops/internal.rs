//! Level 1 backend: undocumented `IVirtualDesktopManagerInternal` in explorer.exe,
//! called through raw vtable slots described by `variants.json` (plan 5.1).
//!
//! All calls go through COM proxies (the objects live in explorer.exe) and must be
//! made from the thread that initialised the STA (the single UI thread).

use super::backend::{check, DesktopBackend, Error, Result};
use super::guid::Guid;
use super::registry;
use super::select::{self, Candidate, Guard, Probe, Skip};
use super::variants::{self, Variant};
use std::ffi::c_void;
use std::fmt;
use std::ptr::{null_mut, NonNull};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_IMAGE, PAGE_EXECUTE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS,
};
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
    /// Explorer's COM services were not reachable (restarting / not up yet): worth retrying.
    pub transient: bool,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported Windows build {}: {}", self.build, self.details)
    }
}

/// IIDs from public sources that no variant uses (one source only / never released). Only asked
/// for with QueryService when nothing passed, so a report says which interface Explorer offers.
const EXTRA_PROBE_IIDS: [&str; 3] = [
    "4970BA3D-FD4E-4647-BEA3-D89076EF4B9C", // pyvda: 22631 (other sources: 53F5CA0B)
    "094AFE11-44F2-4BA0-976F-29A97E263EE0", // pyvda: Insider 20231+
    "0F3A72B0-4566-487E-9A33-4ED302F6D6CE", // IVirtualDesktopManagerInternal2, Windows 10 2004+
];

// ---------------------------------------------------------------- self-test plumbing
//
// During the self-test the layout is not trusted yet. A wrong slot may write more (or less) than
// the expected type, or leave garbage where an interface pointer should be. So the read-only calls
// write into oversized zeroed buffers (anything written past the expected size = wrong layout) and
// a returned pointer is used only when it points to committed readable memory whose first entry
// (the vtable) leads to executable code of a loaded image, as every COM proxy does. An implausible
// pointer is leaked rather than released.

fn readable(addr: usize, len: usize) -> Option<MEMORY_BASIC_INFORMATION> {
    if addr < 0x10000 {
        return None;
    }
    let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let got = unsafe { VirtualQuery(addr as *const c_void, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
    let end = (m.BaseAddress as usize).saturating_add(m.RegionSize);
    let ok = got != 0
        && m.State == MEM_COMMIT
        && m.Protect != 0
        && m.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
        && addr.checked_add(len).is_some_and(|e| e <= end);
    ok.then_some(m)
}

fn plausible_object(p: Raw) -> bool {
    const EXEC: u32 = PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
    let a = p as usize;
    if a % 8 != 0 || readable(a, 8).is_none() {
        return false;
    }
    let vtbl = unsafe { *(a as *const usize) };
    if vtbl % 8 != 0 || readable(vtbl, 3 * 8).is_none() {
        return false;
    }
    let qi = unsafe { *(vtbl as *const usize) };
    readable(qi, 1).is_some_and(|m| m.Type == MEM_IMAGE && m.Protect & EXEC != 0)
}

/// `[out] IUnknown**` call for the self-test (see above).
fn guarded_out(f: impl FnOnce(*mut Raw) -> Hr) -> std::result::Result<ComPtr, String> {
    let mut buf = [0usize; 4];
    let hr = f(buf.as_mut_ptr() as *mut Raw);
    let p = buf[0] as Raw;
    if buf[1..].iter().any(|&x| x != 0) {
        return Err("wrote past the interface pointer".into());
    }
    if hr < 0 {
        // A failing call may still have left an object; release it only when it is plausible.
        if !p.is_null() && plausible_object(p) {
            drop(ComPtr(unsafe { NonNull::new_unchecked(p) }));
        }
        return Err(format!("0x{:08X}", hr as u32));
    }
    if !plausible_object(p) {
        return Err(format!("implausible object pointer {:#x}", p as usize));
    }
    Ok(ComPtr(unsafe { NonNull::new_unchecked(p) }))
}

/// The self-test's view of a freshly opened (not yet trusted) variant.
struct Untrusted<'a>(&'a InternalBackend);

impl Untrusted<'_> {
    fn id_of(&self, d: &ComPtr) -> std::result::Result<Guid, String> {
        // The object must really implement this variant's IVirtualDesktop.
        let d = d.query(&self.0.v.iid_desktop).map_err(|e| format!("QueryInterface(IVirtualDesktop) {e}"))?;
        let mut buf = [0u8; 48];
        let hr = unsafe { d.slot::<FnGuidOut>(self.0.v.desktop_get_id)(d.raw(), buf.as_mut_ptr() as *mut Guid) };
        if hr < 0 {
            return Err(format!("GetId 0x{:08X}", hr as u32));
        }
        if buf[16..].iter().any(|&x| x != 0) {
            return Err("GetId wrote more than a GUID".into());
        }
        Guid::from_bytes(&buf[..16]).ok_or_else(|| "GetId".to_string())
    }

    fn out_call(&self, slot: usize) -> std::result::Result<ComPtr, String> {
        let (m, mon) = (&self.0.mgr, self.0.v.monitor_param);
        guarded_out(|p| unsafe {
            if mon {
                m.slot::<FnMonOut>(slot)(m.raw(), null_mut(), p)
            } else {
                m.slot::<FnOut>(slot)(m.raw(), p)
            }
        })
    }
}

impl Probe for Untrusted<'_> {
    fn count(&mut self) -> std::result::Result<i32, String> {
        let (b, mut buf) = (self.0, [0u32; 8]);
        let m = &b.mgr;
        let out = buf.as_mut_ptr() as *mut i32;
        let hr = unsafe {
            if b.v.monitor_param {
                m.slot::<FnMonCount>(b.v.slots.get_count)(m.raw(), null_mut(), out)
            } else {
                m.slot::<FnCount>(b.v.slots.get_count)(m.raw(), out)
            }
        };
        if hr < 0 {
            return Err(format!("0x{:08X}", hr as u32));
        }
        if buf[1..].iter().any(|&x| x != 0) {
            return Err("wrote more than an int".into());
        }
        Ok(buf[0] as i32)
    }

    fn current(&mut self) -> std::result::Result<Guid, String> {
        let d = self.out_call(self.0.v.slots.get_current)?;
        self.id_of(&d)
    }

    fn desktops(&mut self) -> std::result::Result<Vec<Guid>, String> {
        let arr = self.out_call(self.0.v.slots.get_desktops)?;
        let arr = arr.query(&guid(IID_OBJECT_ARRAY)).map_err(|e| format!("QueryInterface(IObjectArray) {e}"))?;
        let mut n = 0u32;
        check(unsafe { arr.slot::<FnArrCount>(SLOT_ARRAY_COUNT)(arr.raw(), &mut n) }).map_err(|e| e.to_string())?;
        if n > 1000 {
            return Err(format!("{n} desktops"));
        }
        let iid = self.0.v.iid_desktop;
        (0..n)
            .map(|i| {
                let d = out_ptr(|p| unsafe { arr.slot::<FnArrGetAt>(SLOT_ARRAY_GET_AT)(arr.raw(), i, &iid, p) })
                    .map_err(|e| format!("GetAt({i}) {e}"))?;
                self.id_of(&d)
            })
            .collect()
    }

    fn find(&mut self, id: Guid) -> std::result::Result<Guid, String> {
        let m = &self.0.mgr;
        let d = guarded_out(|p| unsafe { m.slot::<FnFind>(self.0.v.slots.find)(m.raw(), &id, p) })?;
        self.id_of(&d)
    }
}

/// [`Candidate`] backed by Explorer: opens a variant with QueryService, self-tests it read-only.
struct System<'a> {
    shell: &'a ComPtr,
    reg_ids: Option<Vec<Guid>>,
    reg_cur: Option<Guid>,
    opened: Option<InternalBackend>,
}

impl Candidate for System<'_> {
    fn open(&mut self, v: &Variant) -> std::result::Result<(), Skip> {
        self.opened = None;
        let mgr = query_service(self.shell, &guid(SID_VDM_INTERNAL), &v.iid_manager).map_err(|e| Skip::Iid(e.hr()))?;
        let vc = v.iid_view_collection;
        let views = query_service(self.shell, &vc, &vc).map_err(|e| Skip::Views(e.hr()))?;
        let public = create_instance(CLSID_VDM_PUBLIC, CLSCTX_ALL, IID_VDM_PUBLIC).map_err(|e| Skip::Views(e.hr()))?;
        let name = format!("internal:{}", v.name);
        self.opened = Some(InternalBackend { name, v: v.clone(), mgr, views, public });
        Ok(())
    }

    fn test(&mut self, _v: &Variant) -> std::result::Result<String, String> {
        let b = self.opened.as_ref().ok_or("not opened")?;
        select::self_test(&mut Untrusted(b), self.reg_ids.as_deref(), self.reg_cur)
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

    /// The variant in use was checked on a real system (`"verified": true` in variants.json).
    pub fn verified(&self) -> bool {
        self.v.verified
    }

    /// Picks the first variant whose build range matches, whose IID Explorer accepts and which
    /// passes the read-only self-test, in the order described in `select.rs`. `diag` collects
    /// human-readable diagnostics.
    pub fn select(diag: &mut Vec<String>) -> std::result::Result<InternalBackend, Unsupported> {
        let build = os_build();
        let ubr = registry::ubr().map_or(String::new(), |u| format!(".{u}"));
        let (list, src) = variants::load();
        diag.push(format!("build {build}{ubr}, {src}, {} variant(s)", list.len()));
        let fail = |details: String, transient: bool| Unsupported { build, details, transient };

        let shell = create_instance(CLSID_IMMERSIVE_SHELL, CLSCTX_LOCAL_SERVER, IID_SERVICE_PROVIDER)
            .map_err(|e| fail(format!("ImmersiveShell: {e}"), true))?;
        let reg_ids = registry::desktop_ids();
        let reg_cur = registry::current_desktop();
        diag.push(format!(
            "registry: {} desktop(s), current {}",
            reg_ids.as_ref().map_or("? (no VirtualDesktopIDs)".into(), |l| l.len().to_string()),
            reg_cur.map_or("?".into(), |g| g.to_string())
        ));

        let mut guard = match std::env::var_os("LOCALAPPDATA") {
            Some(base) => Guard::open(std::path::Path::new(&base).join("AutoSpaces").join("selftest-guard.txt")),
            None => Guard::default(),
        };
        let mut sys = System { shell: &shell, reg_ids, reg_cur, opened: None };
        let (chosen, outcome) = select::choose(&list, build, &mut guard, &mut sys);
        for (name, r) in &outcome {
            let v = list.iter().find(|v| &v.name == name);
            let range = v.map_or(String::new(), |v| {
                format!("{}..{}", v.min_build, v.max_build.map_or(String::new(), |m| m.to_string()))
            });
            let tag = if v.is_some_and(|v| v.verified) { "" } else { " [unverified]" };
            let iid = v.map_or(String::new(), |v| v.iid_manager.to_string());
            match r {
                Ok(s) => diag.push(format!("  {name}{tag}: QueryService({iid}) OK, self-test PASS ({s})")),
                Err(Skip::Range) => diag.push(format!("  {name}{tag}: build range {range} does not match")),
                Err(e) => diag.push(format!("  {name}{tag}: {iid}: {e}")),
            }
        }
        if chosen.is_some() {
            if let Some(b) = sys.opened.take() {
                if !b.v.verified {
                    diag.push(format!("variant {} is UNVERIFIED (taken from public sources, passed the self-test)", b.v.name));
                }
                return Ok(b);
            }
        }
        drop(sys);
        // Which interfaces Explorer offers at all (QueryService only, no method calls): the
        // information needed to add a variant for this build.
        let sid = guid(SID_VDM_INTERNAL);
        let mut iids: Vec<Guid> = Vec::new();
        for g in list.iter().map(|v| v.iid_manager).chain(EXTRA_PROBE_IIDS.iter().map(|s| guid(s))) {
            if !iids.contains(&g) {
                iids.push(g);
            }
        }
        let accepted: Vec<String> =
            iids.iter().filter(|g| query_service(&shell, &sid, g).is_ok()).map(|g| g.to_string()).collect();
        diag.push(format!(
            "IVirtualDesktopManagerInternal IIDs accepted by Explorer: {}",
            if accepted.is_empty() { "none of the known ones".into() } else { accepted.join(", ") }
        ));
        let summary: Vec<String> = outcome
            .iter()
            .filter(|(_, r)| *r != Err(Skip::Range))
            .map(|(n, r)| format!("{n}: {}", r.as_ref().map_or_else(|e| e.to_string(), |s| s.clone())))
            .collect();
        Err(fail(
            if summary.is_empty() { "no variant for this build".into() } else { summary.join("; ") },
            false,
        ))
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
        // No view yet (TYPE_E_ELEMENTNOTFOUND) is reported as such: right after SHOW Explorer has
        // not created it yet (measured 7-200 ms), so it must not count as "cannot move".
        let view = self.view(hwnd)?;
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
