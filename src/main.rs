#![windows_subsystem = "windows"]

mod core;
mod desktops;
mod json;
mod selftest;
mod settings_dlg;
mod sysopts;
mod tray;
mod windowing;

use crate::core::{Event, Orchestrator, Schedule, Settings};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY,
    REG_NOTIFY_CHANGE_LAST_SET,
};
use windows_sys::Win32::System::Threading::CreateEventW;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, KillTimer,
    MsgWaitForMultipleObjectsEx, PeekMessageW, PostQuitMessage, RegisterClassW, MWMO_INPUTAVAILABLE,
    PM_REMOVE, QS_ALLINPUT, WM_QUIT, RegisterWindowMessageW, SetTimer, TranslateMessage, MSG,
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
/// Signalled by RegNotifyChangeKeyValue when the virtual-desktop registry key changes (plan 12.1).
static mut REG_EVENT: HANDLE = std::ptr::null_mut();
static mut REG_KEY: HKEY = std::ptr::null_mut();

const VD_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";

/// (Re-)arms the asynchronous registry notification.
unsafe fn arm_reg_notify() {
    let key = *std::ptr::addr_of!(REG_KEY);
    let ev = *std::ptr::addr_of!(REG_EVENT);
    if !key.is_null() && !ev.is_null() {
        RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, ev, 1);
    }
}

unsafe fn init_reg_notify() {
    let mut key: HKEY = std::ptr::null_mut();
    if RegOpenKeyExW(HKEY_CURRENT_USER, tray::wide(VD_KEY).as_ptr(), 0, KEY_NOTIFY, &mut key) != 0 {
        return;
    }
    let ev = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());
    if ev.is_null() {
        RegCloseKey(key);
        return;
    }
    *std::ptr::addr_of_mut!(REG_KEY) = key;
    *std::ptr::addr_of_mut!(REG_EVENT) = ev;
    arm_reg_notify();
}
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

/// `%APPDATA%\AutoSpaces\settings.json` (plan 12.2).
fn settings_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::Path::new(&base).join("AutoSpaces").join("settings.json"))
}

fn load_settings() -> Settings {
    match settings_path() {
        Some(p) => Settings::load(&p),
        None => Settings::default(),
    }
}

/// Opens the settings dialog and applies the result at once.
fn open_settings(hwnd: HWND) {
    let Some(o) = orch() else { return };
    // Names of apps that run right now fill gaps in old records (version info is read here,
    // when the dialog opens, never on the event path).
    let running = settings_dlg::running_apps();
    let mut current = o.settings.clone();
    for (p, n) in &running {
        if current.is_excluded(p) && !current.names.contains_key(p) {
            current.set_name(p, n);
        }
        o.learned_mut().set_name(p, n);
    }
    let learned = o.learned_mut().list_named();
    // Modal; events keep flowing meanwhile, so the orchestrator is looked up again afterwards.
    let Some(out) = settings_dlg::show(&current, learned, settings_dlg::running_apps) else { return };
    let Some(o) = orch() else { return };
    for p in &out.forgotten {
        o.forget_learned(Some(p));
        log_line(&format!("learned: forgot {p}"));
    }
    if sysopts::autostart_enabled() != out.settings.autostart && !sysopts::set_autostart(out.settings.autostart) {
        log_line("autostart: registry write failed");
    }
    if let Some(p) = settings_path() {
        if let Err(e) = out.settings.save(&p) {
            log_line(&format!("settings.json not saved: {e}"));
        }
    }
    o.apply_settings(out.settings);
    after_orchestrator(hwnd, o, None);
}

fn new_orchestrator(b: desktops::InternalBackend) -> Orch {
    let settings = load_settings();
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

fn flush_state(o: &mut Orch) {
    if o.take_dirty() {
        if let Some(dir) = app_dir() {
            let tmp = dir.join("state.json.tmp");
            if std::fs::write(&tmp, core::state::to_json(&o.snapshot(), o.baseline())).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("state.json"));
            }
        }
    }
}

/// `--restore`: gives every managed window back without starting the tray.
fn restore_cli() -> i32 {
    use desktops::DesktopBackend;
    let mut out = Vec::new();
    let _apartment = desktops::Apartment::init();
    let code = match init_backend() {
        None => {
            out.push("no desktop backend; nothing done".to_string());
            1
        }
        Some(b) => {
            let mut o = new_orchestrator(b);
            let count = |o: &Orch| o.backend().desktops().map_or(0, |l| l.len());
            out.push(format!("desktops before: {}", count(&o)));
            let st = load_state();
            out.push(format!("state.json: {} managed window(s), baseline {:?}", st.spaces.len(), st.baseline));
            o.adopt(st.spaces, st.baseline);
            o.give_back_all();
            out.extend(o.drain_log());
            flush_state(&mut o);
            out.push(format!("desktops after: {} (baseline {})", count(&o), o.baseline()));
            0
        }
    };
    if let Some(dir) = app_dir() {
        let _ = std::fs::write(dir.join("restore.txt"), out.join("\n") + "\n");
    }
    code
}

/// Writes `state.json` (managed spaces) when it changed; logs the orchestrator's notes.
fn after_orchestrator(hwnd: HWND, o: &mut Orch, sch: Option<Schedule>) {
    for l in o.drain_log() {
        log_line(&l);
    }
    flush_state(o);
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
                    let st = load_state();
                    o.adopt(st.spaces, st.baseline);
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

fn load_state() -> core::state::Stored {
    let none = || core::state::Stored { spaces: Vec::new(), baseline: None };
    let Some(dir) = app_dir() else {
        return none();
    };
    match std::fs::read_to_string(dir.join("state.json")) {
        Ok(t) => core::state::from_json(&t).unwrap_or_else(|e| {
            log_line(&format!("state.json ignored: {e}"));
            none()
        }),
        Err(_) => none(),
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
        #[cfg(debug_assertions)]
        m if m == windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 9 => {
            open_settings(hwnd);
            0
        }
        WM_TIMER => {
            on_timer(hwnd, wp);
            0
        }
        tray::WM_TRAY => {
            if lp as u32 == WM_RBUTTONUP {
                let named = orch().map(|o| o.learned_mut().list_named()).unwrap_or_default();
                let names: Vec<String> = named.iter().map(|(p, _)| p.clone()).collect();
                let labels: Vec<String> =
                    named.iter().map(|(p, n)| core::settings::display_label(p, n)).collect();
                match tray::show_menu(hwnd, &labels) {
                    tray::CMD_TOGGLE => {
                        let on = !tray::ENABLED.fetch_xor(true, Relaxed);
                        if let Some(o) = orch() {
                            o.set_enabled(on);
                        }
                    }
                    tray::CMD_EXIT => {
                        if let Some(o) = orch() {
                            o.give_back_all();
                            after_orchestrator(hwnd, o, None);
                        }
                        DestroyWindow(hwnd);
                    }
                    tray::CMD_SETTINGS => open_settings(hwnd),
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
    if std::env::args().nth(1).as_deref() == Some("--restore") {
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, tray::wide("Local\\AutoSpaces_SingleInstance").as_ptr()) };
        if mutex.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            if let Some(dir) = app_dir() {
                let _ = std::fs::write(dir.join("restore.txt"), "autospaces is running; exit it from the tray first\n");
            }
            std::process::exit(2);
        }
        std::process::exit(restore_cli());
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
                let st = load_state();
                o.adopt(st.spaces, st.baseline);
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
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_OPEN_SETTINGS").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 9, 0, 0);
        }
        init_reg_notify();
        *std::ptr::addr_of_mut!(WATCHER) = Some(watcher::Watcher::start(hwnd));

        let mut msg: MSG = std::mem::zeroed();
        'run: loop {
            let ev = *std::ptr::addr_of!(REG_EVENT);
            let handles = [ev];
            let n = if ev.is_null() { 0 } else { 1 };
            let r = MsgWaitForMultipleObjectsEx(n, handles.as_ptr(), u32::MAX, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            if n == 1 && r == WAIT_OBJECT_0 {
                arm_reg_notify();
                if let Some(o) = orch() {
                    o.poll();
                    after_orchestrator(hwnd, o, None);
                }
                continue;
            }
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    break 'run;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
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
