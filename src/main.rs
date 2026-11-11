#![windows_subsystem = "windows"]

mod core;
mod desktops;
mod json;
mod selftest;
mod tray;
mod windowing;

use crate::core::{Event, Orchestrator, Schedule, Settings};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetTimer, TranslateMessage, MSG,
    WM_DESTROY, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
};
use windowing::system::Win32Windows;
use windowing::watcher;

type Orch = Orchestrator<desktops::InternalBackend, Win32Windows>;

static LOGGING: AtomicBool = AtomicBool::new(false);
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
// Touched only from the single UI thread.
static mut WATCHER: Option<watcher::Watcher> = None;
static mut ORCH: Option<Orch> = None;
/// One-shot timers requested by the orchestrator: (timer id, hwnd).
static mut TIMERS: Vec<(usize, HWND)> = Vec::new();

const TIMER_BASE: usize = 100;
const TIMER_REINIT: usize = 1;
const REINIT_TRIES: u32 = 20;
static REINIT_LEFT: AtomicU32 = AtomicU32::new(0);

/// Selects the desktop backend; on failure the app stays inactive and only logs (plan 5.3).
fn init_backend() -> Option<desktops::InternalBackend> {
    use desktops::DesktopBackend;
    let mut diag = Vec::new();
    let res = desktops::InternalBackend::select(&mut diag);
    for d in &diag {
        log_line(d);
    }
    match res {
        Ok(b) => {
            log_line(&format!("backend: {}", b.name()));
            Some(b)
        }
        Err(u) => {
            log_line(&format!("backend: {u}"));
            None
        }
    }
}

fn app_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let dir = std::path::Path::new(&base).join("AutoSpaces");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

fn log_line(line: &str) {
    if !LOGGING.load(Relaxed) {
        return;
    }
    let Some(dir) = app_dir() else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("log.txt"))
    {
        let _ = writeln!(f, "{line}");
    }
}

fn orch() -> Option<&'static mut Orch> {
    unsafe { (*std::ptr::addr_of_mut!(ORCH)).as_mut() }
}

fn new_orchestrator(b: desktops::InternalBackend) -> Orch {
    let settings = Settings::default();
    let learned: Box<dyn core::learned::Learned> = match app_dir() {
        Some(dir) => {
            use desktops::DesktopBackend;
            Box::new(core::learned::LearnedStore::load(
                dir.join("learned.json"),
                b.name(),
                settings.learned_retry_days,
            ))
        }
        None => Box::new(core::learned::NoLearned),
    };
    let mut o = Orchestrator::new(b, Win32Windows::new(), settings);
    o.set_learned(learned);
    o.settings.enabled = tray::ENABLED.load(Relaxed);
    o
}

/// Writes `state.json` (managed spaces) when it changed; logs the orchestrator's notes.
fn after_orchestrator(hwnd: HWND, o: &mut Orch, sch: Option<Schedule>) {
    for l in o.drain_log() {
        log_line(&l);
    }
    if o.take_dirty() {
        if let Some(dir) = app_dir() {
            let tmp = dir.join("state.json.tmp");
            if std::fs::write(&tmp, core::state::to_json(&o.snapshot())).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("state.json"));
            }
        }
    }
    if let Some(s) = sch {
        schedule(hwnd, s);
    }
}

fn schedule(main: HWND, s: Schedule) {
    unsafe {
        let timers = &mut *std::ptr::addr_of_mut!(TIMERS);
        // Same window: reuse the id, which restarts the timer.
        let id = match timers.iter().find(|t| t.1 == s.hwnd) {
            Some(t) => t.0,
            None => {
                let id = (TIMER_BASE..).find(|i| !timers.iter().any(|t| t.0 == *i)).unwrap_or(TIMER_BASE);
                timers.push((id, s.hwnd));
                id
            }
        };
        SetTimer(main, id, s.delay_ms.max(10), None);
    }
}

fn dispatch(main: HWND, target: HWND, ev: Event) {
    if let Some(o) = orch() {
        let sch = o.on_event(target, ev);
        after_orchestrator(main, o, sch);
    }
}

fn on_timer(main: HWND, id: usize) {
    unsafe {
        if id == TIMER_REINIT {
            KillTimer(main, id);
            reinit_backend(main);
            return;
        }
        KillTimer(main, id);
        let timers = &mut *std::ptr::addr_of_mut!(TIMERS);
        if let Some(pos) = timers.iter().position(|t| t.0 == id) {
            let (_, target) = timers.swap_remove(pos);
            dispatch(main, target, Event::Recheck);
        }
    }
}

/// (Re)creates the backend, e.g. after an Explorer restart (plan 6.5). Retries by timer
/// while Explorer's COM services are not up yet.
fn reinit_backend(main: HWND) {
    match init_backend() {
        Some(b) => unsafe {
            match orch() {
                Some(o) => {
                    o.set_backend(b);
                    o.reconcile();
                }
                None => {
                    let mut o = new_orchestrator(b);
                    o.adopt(load_state());
                    *std::ptr::addr_of_mut!(ORCH) = Some(o);
                }
            }
            if let Some(o) = orch() {
                after_orchestrator(main, o, None);
            }
        },
        None => {
            let left = REINIT_LEFT.load(Relaxed);
            if left > 0 {
                REINIT_LEFT.store(left - 1, Relaxed);
                unsafe { SetTimer(main, TIMER_REINIT, 1000, None) };
            }
        }
    }
}

fn load_state() -> Vec<crate::core::ManagedSpace> {
    let Some(dir) = app_dir() else {
        return Vec::new();
    };
    match std::fs::read_to_string(dir.join("state.json")) {
        Ok(t) => core::state::from_json(&t).unwrap_or_else(|e| {
            log_line(&format!("state.json ignored: {e}"));
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        watcher::WM_WINEVENT => {
            if let Some(ev) = watcher::to_event(lp as u32) {
                dispatch(hwnd, wp as HWND, ev);
            }
            0
        }
        WM_TIMER => {
            on_timer(hwnd, wp);
            0
        }
        tray::WM_TRAY => {
            if lp as u32 == WM_RBUTTONUP {
                let names = orch().map(|o| o.learned_mut().list_blocked()).unwrap_or_default();
                match tray::show_menu(hwnd, &names) {
                    tray::CMD_TOGGLE => {
                        let on = !tray::ENABLED.fetch_xor(true, Relaxed);
                        if let Some(o) = orch() {
                            o.settings.enabled = on;
                        }
                    }
                    tray::CMD_EXIT => {
                        DestroyWindow(hwnd);
                    }
                    tray::CMD_FORGET_ALL => {
                        if let Some(o) = orch() {
                            o.forget_learned(None);
                        }
                        log_line("learned: forgot all");
                    }
                    c if c >= tray::CMD_FORGET_BASE => {
                        if let (Some(o), Some(name)) = (orch(), names.get(c - tray::CMD_FORGET_BASE)) {
                            o.forget_learned(Some(name));
                            log_line(&format!("learned: forgot {name}"));
                        }
                    }
                    _ => {}
                }
            }
            0
        }
        WM_DESTROY => {
            tray::remove(hwnd);
            if let Some(w) = (*std::ptr::addr_of_mut!(WATCHER)).as_mut() {
                w.stop();
            }
            PostQuitMessage(0);
            0
        }
        m if m != 0 && m == WM_TASKBAR_CREATED.load(Relaxed) => {
            tray::add(hwnd);
            if let Some(o) = orch() {
                o.suspend_learning();
            }
            REINIT_LEFT.store(REINIT_TRIES, Relaxed);
            reinit_backend(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    LOGGING.store(
        cfg!(debug_assertions) || std::env::var("AUTOSPACES_LOG").is_ok_and(|v| v == "1"),
        Relaxed,
    );
    if std::env::args().nth(1).as_deref() == Some("--selftest") {
        let apartment = desktops::Apartment::init();
        let code = selftest::run();
        drop(apartment);
        std::process::exit(code);
    }
    unsafe {
        let mutex = CreateMutexW(
            std::ptr::null(),
            0,
            tray::wide("Local\\AutoSpaces_SingleInstance").as_ptr(),
        );
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let hinst = GetModuleHandleW(std::ptr::null());
        let class_name = tray::wide("AutoSpacesHiddenWindow");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd_proc);
        wc.hInstance = hinst;
        wc.lpszClassName = class_name.as_ptr();
        RegisterClassW(&wc);

        WM_TASKBAR_CREATED.store(
            RegisterWindowMessageW(tray::wide("TaskbarCreated").as_ptr()),
            Relaxed,
        );

        // Hidden top-level window (not HWND_MESSAGE) so it gets broadcasts.
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }

        let _apartment = desktops::Apartment::init();
        match init_backend() {
            Some(b) => {
                let mut o = new_orchestrator(b);
                o.adopt(load_state());
                *std::ptr::addr_of_mut!(ORCH) = Some(o);
                if let Some(o) = orch() {
                    after_orchestrator(hwnd, o, None);
                }
            }
            None => {
                REINIT_LEFT.store(REINIT_TRIES, Relaxed);
                SetTimer(hwnd, TIMER_REINIT, 1000, None);
            }
        }

        tray::add(hwnd);
        *std::ptr::addr_of_mut!(WATCHER) = Some(watcher::Watcher::start(hwnd));

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // Release COM proxies before the apartment is torn down.
        *std::ptr::addr_of_mut!(ORCH) = None;
    }
    log_line(&format!(
        "exit: callbacks_total={} callbacks_passed={}",
        watcher::TOTAL.load(Relaxed),
        watcher::PASSED.load(Relaxed)
    ));
}
