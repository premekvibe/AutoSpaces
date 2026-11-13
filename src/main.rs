#![windows_subsystem = "windows"]

mod about_dlg;
mod core;
mod desktops;
mod i18n;
mod json;
mod selftest;
mod settings_dlg;
mod sysopts;
mod tray;
mod windowing;

use crate::core::{Event, Orchestrator, Schedule, Settings};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
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
    WM_CLOSE, WM_DESTROY, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
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
/// One-shot timer of the support reminder (plan 12.9).
const TIMER_SUPPORT: usize = 2;
/// Retry delay when the user is busy (full-screen app, layout profile, another dialog).
const SUPPORT_RETRY_SECS: u64 = 5 * 60;
/// Tick count at start (uptime of this run) and whether the reminder was shown in this run.
static START_TICK_MS: AtomicU64 = AtomicU64::new(0);
static SUPPORT_SHOWN: AtomicBool = AtomicBool::new(false);
static mut SUPPORT: Option<core::support::Support> = None;
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

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn uptime_secs() -> u64 {
    let start = START_TICK_MS.load(Relaxed);
    (unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }.saturating_sub(start)) / 1000
}

fn support() -> &'static mut core::support::Support {
    unsafe { (*std::ptr::addr_of_mut!(SUPPORT)).get_or_insert_with(Default::default) }
}

fn save_support() {
    if let Some(dir) = app_dir() {
        let tmp = dir.join("support.json.tmp");
        if std::fs::write(&tmp, support().to_json()).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join("support.json"));
        }
    }
}

/// Reads support.json (tolerantly), records the first run and arms the reminder timer.
fn support_init(main: HWND) {
    START_TICK_MS.store(unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }, Relaxed);
    let text = app_dir().and_then(|d| std::fs::read_to_string(d.join("support.json")).ok());
    let missing = text.is_none();
    *support() = text.map_or_else(Default::default, |t| core::support::Support::from_json(&t));
    if support().normalize(now_secs()) || missing {
        save_support();
    }
    support_arm(main, None);
}

/// Arms the one-shot timer for the earliest moment the reminder may appear (`retry`: a fixed delay).
fn support_arm(main: HWND, retry: Option<u64>) {
    use core::support::Verdict;
    let secs = match retry {
        Some(r) => r,
        None => match support().verdict(now_secs(), uptime_secs(), SUPPORT_SHOWN.load(Relaxed)) {
            Verdict::Done => return,
            Verdict::Show => 1,
            Verdict::WaitSecs(w) => w + 1,
        },
    };
    // Longest possible timer is about 24 days; the check simply re-arms itself when it fires early.
    let ms = (secs.saturating_mul(1000)).clamp(1000, 0x7FFF_FFFF) as u32;
    unsafe { SetTimer(main, TIMER_SUPPORT, ms, None) };
}

/// True when the user should not be interrupted right now.
fn support_busy() -> bool {
    windowing::classifier::foreground_is_fullscreen()
        || orch().is_some_and(|o| o.layout_running())
        || unsafe { !(*std::ptr::addr_of!(TIMERS)).is_empty() }
        || about_dlg::is_open()
        || settings_dlg::is_open()
}

/// The support timer fired: show the reminder when due and the moment is calm, else try again.
fn on_support_timer(main: HWND) {
    use core::support::Verdict;
    unsafe { KillTimer(main, TIMER_SUPPORT) };
    let now = now_secs();
    if support().normalize(now) {
        save_support();
    }
    match support().verdict(now, uptime_secs(), SUPPORT_SHOWN.load(Relaxed)) {
        Verdict::Done => {}
        Verdict::WaitSecs(_) => support_arm(main, None),
        Verdict::Show => show_support(main, false),
    }
}

fn show_support(main: HWND, forced: bool) {
    if !forced && support_busy() {
        support_arm(main, Some(SUPPORT_RETRY_SECS));
        return;
    }
    if about_dlg::show_support(main) {
        SUPPORT_SHOWN.store(true, Relaxed);
        log_line("support reminder shown");
    } else {
        support_arm(main, Some(SUPPORT_RETRY_SECS));
    }
}

/// The log is kept below this size: a full `log.txt` becomes `log.old.txt` (replacing it).
const LOG_MAX_BYTES: u64 = 1024 * 1024;

fn log_line(line: &str) {
    if !LOGGING.load(Relaxed) {
        return;
    }
    let Some(dir) = app_dir() else {
        return;
    };
    let path = dir.join("log.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() >= LOG_MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join("log.old.txt"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
        let _ = writeln!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {line}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
        );
    }
}

/// Log on when built for debugging, `AUTOSPACES_LOG=1`, or `"log": true` in settings.json.
fn logging_wanted(settings_log: bool) -> bool {
    cfg!(debug_assertions) || settings_log || std::env::var("AUTOSPACES_LOG").is_ok_and(|v| v == "1")
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
    let Some(p) = settings_path() else { return Settings::default() };
    let mut s = Settings::load(&p);
    // Plan 12.7: older versions stored packaged apps by their versioned WindowsApps path.
    if s.layout.migrate_packaged(read_manifest) {
        match s.save(&p) {
            Ok(()) => log_line("settings.json: packaged apps of the layout profile now start by AUMID"),
            Err(e) => log_line(&format!("settings.json not saved: {e}")),
        }
    }
    s
}

/// `AppxManifest.xml` of a package folder.
fn read_manifest(folder: &str) -> Option<String> {
    std::fs::read_to_string(std::path::Path::new(folder).join("AppxManifest.xml")).ok()
}

/// Launch command for an exe the user picked: packaged apps by AUMID (plan 12.7).
pub fn launch_for_path(path: &str) -> String {
    core::settings::packaged_launch(path, read_manifest).unwrap_or_else(|| path.to_string())
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
    let Some(out) = settings_dlg::show(&current, learned, settings_dlg::running_apps, capture_layout) else { return };
    let Some(o) = orch() else { return };
    // The tray menu and every dialog opened from now on use the chosen language.
    i18n::set_pref(out.settings.language);
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
    LOGGING.store(logging_wanted(out.settings.log), Relaxed);
    o.apply_settings(out.settings);
    // "Apply now": the profile is applied at once (the timer, if any, only runs while apps are awaited).
    let sch = if out.apply_now { o.start_layout() } else { None };
    after_orchestrator(hwnd, o, sch);
}

/// Opens the About dialog (modal; shows the backend variant the app runs on, if any).
fn open_about() {
    use desktops::DesktopBackend;
    let backend = orch().map(|o| o.backend().name().to_string());
    about_dlg::show(backend.as_deref());
}

/// "Use current layout" button of the layout dialog.
fn capture_layout() -> (usize, Vec<core::settings::LayoutRule>) {
    orch().map(|o| o.capture_layout()).unwrap_or_default()
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
    // Debug builds only: live tests move nothing but their own test processes.
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("AUTOSPACES_ONLY") {
        let names: Vec<String> = v.split(',').map(core::settings::norm_process).filter(|p| !p.is_empty()).collect();
        log_line(&format!("debug: only these processes are moved: {}", names.join(", ")));
        o.set_only(names);
    }
    o
}

/// After `adopt` at startup: schedules the startup sequence (plan 12.11 + 12.7).
fn begin_startup(main: HWND, o: &mut Orch) {
    let at_logon = windowing::system::started_with_windows();
    let sch = o.startup_schedule(at_logon);
    after_orchestrator(main, o, sch);
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
        if id == TIMER_SUPPORT {
            on_support_timer(main);
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
                    // The backend was not ready at startup: this is the app's real start.
                    let mut o = new_orchestrator(b);
                    let st = load_state();
                    o.adopt(st.spaces, st.baseline);
                    *std::ptr::addr_of_mut!(ORCH) = Some(o);
                    if let Some(o) = orch() {
                        begin_startup(main, o);
                    }
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

/// Gives every managed window back, levels the desktops and closes the hidden window (ends the loop).
unsafe fn exit_clean(hwnd: HWND) {
    if let Some(o) = orch() {
        o.give_back_all();
        after_orchestrator(hwnd, o, None);
    }
    DestroyWindow(hwnd);
}

/// `--quit`: asks the running instance to exit cleanly and waits for it (0 = gone or not running, 1 = timeout).
fn quit_cli() -> i32 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::OpenMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
    let name = tray::wide(SINGLE_INSTANCE);
    unsafe {
        let hwnd = FindWindowW(tray::wide(WINDOW_CLASS).as_ptr(), std::ptr::null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        for _ in 0..100 {
            let h = OpenMutexW(SYNCHRONIZE, 0, name.as_ptr());
            if h.is_null() {
                return 0;
            }
            CloseHandle(h);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    1
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
        about_dlg::WM_SUPPORT_CLOSED => {
            support().closed(now_secs(), wp != 0);
            save_support();
            log_line(&format!("support reminder closed, never again: {}", wp != 0));
            if wp == 0 {
                support_arm(hwnd, None);
            }
            0
        }
        #[cfg(debug_assertions)]
        m if m == windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 11 => {
            show_support(hwnd, true);
            0
        }
        #[cfg(debug_assertions)]
        m if m == windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10 => {
            open_about();
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
                    tray::CMD_EXIT => exit_clean(hwnd),
                    tray::CMD_SETTINGS => open_settings(hwnd),
                    tray::CMD_ABOUT => {
                        open_about();
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
        // Asked from outside (installer, `--quit`, Restart Manager, ...): same clean path as the tray "Exit".
        WM_CLOSE => {
            exit_clean(hwnd);
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

/// Single-instance mutex (the installer's `AppMutex` is the same name) and the hidden window's class.
const SINGLE_INSTANCE: &str = r"Local\AutoSpaces_SingleInstance";
const WINDOW_CLASS: &str = "AutoSpacesHiddenWindow";

fn main() {
    LOGGING.store(logging_wanted(settings_path().is_some_and(|p| Settings::load(&p).log)), Relaxed);
    if std::env::args().nth(1).as_deref() == Some("--selftest") {
        let apartment = desktops::Apartment::init();
        let code = selftest::run();
        drop(apartment);
        std::process::exit(code);
    }
    if std::env::args().nth(1).as_deref() == Some("--quit") {
        std::process::exit(quit_cli());
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
        let class_name = tray::wide(WINDOW_CLASS);
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
                    // Arrangement + layout profile as one sequence, after the delay (plan 12.11).
                    begin_startup(hwnd, o);
                }
            }
            None => {
                REINIT_LEFT.store(REINIT_TRIES, Relaxed);
                SetTimer(hwnd, TIMER_REINIT, 1000, None);
            }
        }

        i18n::set_pref(settings_path().map_or(i18n::Pref::Auto, |p| Settings::load(&p).language));
        tray::add(hwnd);
        support_init(hwnd);
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_SHOW_SUPPORT").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 11, 0, 0);
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_OPEN_ABOUT").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10, 0, 0);
        }
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
                if about_dlg::handle_message(&msg) {
                    continue;
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
