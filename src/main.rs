#![windows_subsystem = "windows"]

mod about_dlg;
mod carry_ui;
mod core;
mod desktops;
mod glass;
mod i18n;
mod overlay_ui;
mod json;
mod report;
mod report_dlg;
mod selftest;
mod settings_dlg;
mod sysopts;
mod tray;
mod update;
mod update_dlg;
mod wallpaper;
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
/// While a window is carried: a safety check that the modifiers are still held (in case the
/// keyboard hook was removed by Windows), plan 12.13.
const TIMER_CARRY_WATCH: usize = 3;
/// Hides the hint "This window cannot be moved".
const TIMER_HINT_HIDE: usize = 4;
const CARRY_WATCH_MS: u32 = 250;
/// "This window cannot be moved" stays this long.
const HINT_MS: u32 = 1200;
/// Win+Ctrl+Shift held alone: the preview's delay (only while the three are held).
const TIMER_PREVIEW: usize = 5;
/// One-shot timer of the update check (plan 12.15), re-armed after every check (at most an hour).
const TIMER_UPDATE: usize = 6;
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

/// Why there is no desktop backend.
enum NoBackend {
    /// Switched off by the hidden `"backend"` key (plan 5.2: the fallback is not built).
    Forced(core::settings::BackendPref),
    /// No variant passed (plan 5.3).
    Unsupported { build: u32 },
}

/// Diagnostics of the last variant selection, for the report (plan 12.12).
static mut LAST_DIAG: Vec<String> = Vec::new();
/// The "not supported" notification is shown once per run.
static NOTICE_SHOWN: AtomicBool = AtomicBool::new(false);

/// Selects the desktop backend; on failure the app stays inactive and only logs (plan 5.3).
fn init_backend() -> Result<desktops::InternalBackend, NoBackend> {
    use core::settings::BackendPref;
    use desktops::DesktopBackend;
    let pref = settings_path().map_or(BackendPref::Auto, |p| Settings::load(&p).backend);
    if pref != BackendPref::Auto {
        let why = match pref {
            BackendPref::Fallback => "the public-API fallback (plan 5.2) is not built",
            _ => "no backend requested",
        };
        log_line(&format!("backend: switched off by settings.json \"backend\": \"{}\" ({why})", pref.code()));
        return Err(NoBackend::Forced(pref));
    }
    let mut diag = Vec::new();
    let res = desktops::InternalBackend::select(&mut diag);
    for d in &diag {
        log_line(d);
    }
    match res {
        Ok(b) => {
            let tag = if b.verified() { "" } else { " (UNVERIFIED variant, self-test passed)" };
            log_line(&format!("backend: {}{tag}", b.name()));
            diag.push(format!("result: {}{tag}", b.name()));
            unsafe { *std::ptr::addr_of_mut!(LAST_DIAG) = diag };
            Ok(b)
        }
        Err(u) => {
            log_line(&format!("backend: {u}"));
            diag.push(format!("result: {u}"));
            diag.push(
                if u.transient {
                    "cause: Explorer's virtual desktop service could not be reached (it may not run on this system)"
                } else {
                    "cause: no known variant of the virtual desktop interface passed the read-only self-test"
                }
                .to_string(),
            );
            unsafe { *std::ptr::addr_of_mut!(LAST_DIAG) = diag };
            Err(NoBackend::Unsupported { build: u.build })
        }
    }
}

/// The app runs without a backend: nothing is moved. The icon says so and a notification explains it
/// once per run; "Report a problem" in the menu creates what is needed to add a variant (plan 5.3).
fn enter_no_backend(main: HWND, why: &NoBackend) {
    let (mode, text) = match why {
        NoBackend::Forced(p) => (tray::MODE_FORCED_OFF, i18n::tf(i18n::Key::NoticeForcedOff, &[p.code()])),
        NoBackend::Unsupported { build, .. } => {
            (tray::MODE_UNSUPPORTED, i18n::tf(i18n::Key::NoticeUnsupported, &[&build.to_string()]))
        }
    };
    tray::set_mode(main, mode);
    // Debug builds: `AUTOSPACES_NO_NOTICE=1` keeps a test copy quiet.
    #[cfg(debug_assertions)]
    if std::env::var_os("AUTOSPACES_NO_NOTICE").is_some() {
        NOTICE_SHOWN.store(true, Relaxed);
    }
    if !NOTICE_SHOWN.swap(true, Relaxed) {
        BALLOON_UPDATE.store(false, Relaxed);
        tray::notify(main, &text);
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
        || report_dlg::is_open()
        || update_dlg::is_open()
}

// ---------------------------------------------------------------- updates (plan 12.15)

/// When the next automatic check is due, the versions announced / skipped, the release found last.
static mut UPDATE_TIMING: core::update::Timing = core::update::Timing::new();
static mut UPDATE_NOTICES: Option<core::update::Notices> = None;
static mut UPDATE_FOUND: Option<core::update::Release> = None;
/// The last notification of the icon was about a new version (a click on it opens the dialog).
static BALLOON_UPDATE: AtomicBool = AtomicBool::new(false);
/// "Check for updates" was chosen while an automatic check was running: its result counts as manual.
static UPDATE_MANUAL_PENDING: AtomicBool = AtomicBool::new(false);

fn update_timing() -> &'static mut core::update::Timing {
    unsafe { &mut *std::ptr::addr_of_mut!(UPDATE_TIMING) }
}

fn update_notices() -> &'static mut core::update::Notices {
    unsafe { (*std::ptr::addr_of_mut!(UPDATE_NOTICES)).get_or_insert_with(|| update::load_notices(app_dir().as_deref())) }
}

fn update_found() -> &'static mut Option<core::update::Release> {
    unsafe { &mut *std::ptr::addr_of_mut!(UPDATE_FOUND) }
}

/// `"checkUpdates"` of the running settings (or of settings.json when there is no orchestrator).
fn check_updates_on() -> bool {
    orch().map(|o| o.settings.check_updates).or_else(|| settings_path().map(|p| Settings::load(&p).check_updates)).unwrap_or(true)
}

/// Saves `"checkUpdates"` (the box in the update dialog).
fn set_check_updates(on: bool) {
    let settings = match orch() {
        Some(o) => {
            o.settings.check_updates = on;
            o.settings.clone()
        }
        None => {
            let mut s = settings_path().map_or_else(Settings::default, |p| Settings::load(&p));
            s.check_updates = on;
            s
        }
    };
    if let Some(p) = settings_path() {
        if let Err(e) = settings.save(&p) {
            log_line(&format!("settings.json not saved: {e}"));
        }
    }
    log_line(&format!("update: automatic checks {}", if on { "on" } else { "off" }));
}

/// Uptime as the update timer sees it (debug builds: `AUTOSPACES_UPDATE_NOW=1` skips the 10 minutes).
fn update_uptime() -> u64 {
    #[cfg(debug_assertions)]
    if std::env::var_os("AUTOSPACES_UPDATE_NOW").is_some() {
        return uptime_secs() + core::update::FIRST_CHECK_SECS;
    }
    uptime_secs()
}

/// Arms the update timer for the next due check (at most an hour ahead, so sleep is noticed).
fn update_arm(main: HWND) {
    let wait = update_timing().wait_secs(update_uptime(), now_secs());
    let wait = if wait == 0 { core::update::MAX_TIMER_SECS } else { wait };
    unsafe { SetTimer(main, TIMER_UPDATE, core::update::Timing::timer_ms(wait), None) };
}

fn on_update_timer(main: HWND) {
    unsafe { KillTimer(main, TIMER_UPDATE) };
    if update_timing().wait_secs(update_uptime(), now_secs()) == 0 && check_updates_on() {
        // What an earlier update left in %TEMP% (the installer has long finished by now).
        update::cleanup();
        if update::start_check(main, false) {
            log_line("update: checking GitHub for a new version");
            return; // re-armed when the result arrives
        }
    }
    update_arm(main);
}

/// "Check for updates" in the tray menu.
fn check_updates_now(main: HWND) {
    if update_dlg::bring_forward() {
        return;
    }
    if update::start_check(main, true) {
        log_line("update: checking GitHub for a new version (on request)");
    } else {
        UPDATE_MANUAL_PENDING.store(true, Relaxed);
    }
}

fn info_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND};
    let title = tray::wide(i18n::t(i18n::Key::AppName));
    unsafe { MessageBoxW(std::ptr::null_mut(), tray::wide(text).as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND) };
}

/// The check thread is done.
fn on_update_checked(main: HWND, lp: LPARAM) {
    use core::update::{InstallKind, Verdict, Version};
    let checked = unsafe { update::take(lp) };
    let manual = checked.manual || UPDATE_MANUAL_PENDING.swap(false, Relaxed);
    let now = now_secs();
    match checked.result {
        Err(e) => {
            log_line(&format!("update: check failed: {e}"));
            update_timing().done(now, false);
            if manual {
                info_box(i18n::t(i18n::Key::UpdateCheckFailed));
            }
        }
        Ok(found) => {
            update_timing().done(now, true);
            let current = Version::current();
            let latest = found.as_ref().map(|r| r.version);
            log_line(&format!(
                "update: latest release {}, this is {current}",
                latest.map_or_else(|| "none usable".to_string(), |v| v.to_string())
            ));
            if let Some(r) = found.filter(|r| r.version > current) {
                *update_found() = Some(r);
            }
            match update_notices().verdict(current, latest, manual) {
                Verdict::Nothing => {}
                Verdict::UpToDate => info_box(&i18n::tf(i18n::Key::UpdateUpToDate, &[&current.to_string()])),
                Verdict::Notify => {
                    if let Some(r) = update_found().as_ref() {
                        let key = if r.installable() && update::install_kind() != InstallKind::Portable {
                            i18n::Key::NoticeUpdate
                        } else {
                            i18n::Key::NoticeUpdatePage
                        };
                        BALLOON_UPDATE.store(true, Relaxed);
                        tray::notify_info(main, &i18n::tf(key, &[&r.version.to_string()]));
                        log_line(&format!("update: {} announced", r.version));
                    }
                }
                Verdict::Dialog => open_update(main),
            }
        }
    }
    update_arm(main);
}

/// The update dialog for the release found last; then skip, or download + start the installer.
fn open_update(main: HWND) {
    use core::update::InstallKind;
    use update_dlg::Choice;
    let Some(release) = update_found().clone() else { return };
    let kind = update::install_kind();
    let check = check_updates_on();
    let Some(out) = update_dlg::show(&release, kind, check) else { return };
    for l in &out.log {
        log_line(l);
    }
    if out.check_updates != check {
        set_check_updates(out.check_updates);
    }
    match out.choice {
        Choice::Later => {}
        Choice::Skip => {
            update_notices().skip(release.version);
            update::save_notices(app_dir().as_deref(), update_notices());
            log_line(&format!("update: version {} skipped", release.version));
        }
        Choice::Install(verified) => match update::launch_installer(&verified, kind) {
            Ok(()) => {
                drop(verified);
                if kind == InstallKind::PerUser {
                    log_line(&format!("update: installer of {} started (current user); exiting", release.version));
                    // The same clean exit as "Exit": windows and desktops are given back first.
                    unsafe { exit_clean(main) };
                } else {
                    // All users: the installer asks for elevation itself and, once allowed, closes
                    // AutoSpaces with WM_CLOSE (the same clean exit). Refused = nothing happens.
                    log_line(&format!("update: installer of {} started (all users); it closes AutoSpaces after elevation", release.version));
                }
            }
            Err(e) => {
                log_line(&format!("update: installer not started: {e}"));
                info_box(i18n::t(i18n::Key::UpdateLaunchFailed));
            }
        },
    }
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

/// Is the activity log switched on by the user (`"log"` in settings.json or `AUTOSPACES_LOG=1`)?
/// Debug builds always log, which does not count here.
fn log_switched_on() -> bool {
    orch().map(|o| o.settings.log).or_else(|| settings_path().map(|p| Settings::load(&p).log)).unwrap_or(false)
        || std::env::var("AUTOSPACES_LOG").is_ok_and(|v| v == "1")
}

/// "Turn on logging" in the report dialog: the log starts at once and stays on in settings.json.
fn enable_logging() {
    LOGGING.store(true, Relaxed);
    let settings = match orch() {
        Some(o) => {
            o.settings.log = true;
            o.settings.clone()
        }
        None => {
            let mut s = settings_path().map_or_else(Settings::default, |p| Settings::load(&p));
            s.log = true;
            s
        }
    };
    if let Some(p) = settings_path() {
        if let Err(e) = settings.save(&p) {
            log_line(&format!("settings.json not saved: {e}"));
        }
    }
    log_line("activity logging switched on from the report dialog");
}

/// Writes `%LOCALAPPDATA%\AutoSpaces\report.txt` (plan 12.12); its path.
fn create_report() -> Option<std::path::PathBuf> {
    use core::settings::BackendPref;
    use desktops::DesktopBackend;
    let dir = app_dir()?;
    let env = report::Env::from_process();
    let (monitors, primary_size, primary_dpi) = report::display_info();
    let build = desktops::internal::os_build();
    let (variant, variant_verified) = match orch() {
        Some(o) => (Some(o.backend().name().to_string()), o.backend().verified()),
        None => (None, false),
    };
    let file_settings = settings_path().map(|p| Settings::load(&p));
    let pref = orch().map(|o| o.settings.language).or(file_settings.as_ref().map(|s| s.language)).unwrap_or_default();
    let mut selection = unsafe { (*std::ptr::addr_of!(LAST_DIAG)).clone() };
    let mode = match tray::mode() {
        tray::MODE_UNSUPPORTED => format!("unsupported Windows build {build}; nothing is moved"),
        tray::MODE_FORCED_OFF => {
            let code = file_settings.as_ref().map_or(BackendPref::None, |s| s.backend).code();
            selection.push(format!("switched off by \"backend\": \"{code}\" in settings.json"));
            format!("switched off by \"backend\": \"{code}\" in settings.json; nothing is moved")
        }
        _ if variant.is_some() => "normal".to_string(),
        _ => "starting (no desktop interface yet)".to_string(),
    };
    let read = |name: &str| std::fs::read(dir.join(name)).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    // A log that was just rotated leaves too few lines: the older file completes the tail.
    let log = read("log.txt").map(|cur| match read("log.old.txt") {
        Some(old) if cur.lines().count() < report::LOG_TAIL_LINES => format!("{}\n{cur}", old.trim_end()),
        _ => cur,
    });
    let data = report::ReportData {
        version: env!("CARGO_PKG_VERSION").to_string(),
        windows_build: build,
        ubr: desktops::registry::ubr(),
        ui_language: format!("{} (setting: {})", i18n::current().code(), pref.code()),
        monitors,
        primary_size,
        primary_dpi,
        mode,
        variant,
        variant_verified,
        selection,
        // From the registry: Explorer's COM interface refuses calls made from inside a dialog's button handler.
        desktops: Some(
            desktops::registry::desktop_ids().map_or("1 (no desktop list in the registry)".to_string(), |l| l.len().to_string()),
        ),
        logging: log_switched_on(),
        settings_json: settings_path().and_then(|p| std::fs::read(p).ok()).map(|b| String::from_utf8_lossy(&b).into_owned()),
        learned_json: read("learned.json"),
        log,
    };
    let path = dir.join("report.txt");
    std::fs::write(&path, report::build_report(&data, &env)).ok()?;
    Some(path)
}

fn open_report() {
    report_dlg::show(report_dlg::Hooks { create: create_report, logging_on: log_switched_on, enable_logging });
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
    let wall_note = wallpaper::status(wallpaper::caps(Some(o.backend())));
    let old_wall = current.wallpapers.clone();
    let Some(out) = settings_dlg::show(
        &current,
        learned,
        settings_dlg::running_apps,
        capture_layout,
        carry_status(),
        wall_note,
        wallpaper::request_shuffle,
    ) else {
        return;
    };
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
    // Wallpapers (plan 12.17): applied at once when the choice changed; "do not change" touches nothing.
    if out.settings.wallpapers.changed_from(&old_wall) {
        wallpaper::apply(Some(o.backend()), &out.settings.wallpapers, &mut |l| log_line(&l));
    }
    o.apply_settings(out.settings);
    // "Apply now": the profile is applied at once (the timer, if any, only runs while apps are awaited).
    let sch = if out.apply_now { o.start_layout() } else { None };
    after_orchestrator(hwnd, o, sch);
    update_hotkey(hwnd);
}

/// Opens the About dialog (modal; shows the backend variant the app runs on, if any).
fn open_about() {
    use about_dlg::Backend;
    use desktops::DesktopBackend;
    let name = orch().map(|o| (o.backend().name().to_string(), o.backend().verified()));
    let info = match (&name, tray::mode()) {
        (Some((n, v)), tray::MODE_NORMAL) => Backend::Variant { name: n, verified: *v },
        (_, tray::MODE_FORCED_OFF) => Backend::ForcedOff,
        (_, tray::MODE_UNSUPPORTED) => Backend::Unsupported { build: desktops::internal::os_build() },
        _ => Backend::Pending,
    };
    about_dlg::show(&info);
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
    // Windows pinned by a carry are recorded in state.json before they are pinned (plan 12.13).
    o.set_persist(Box::new(write_state));
    o.settings.enabled = tray::ENABLED.load(Relaxed);
    // Debug builds only: live tests move nothing but their own test processes.
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("AUTOSPACES_ONLY") {
        let names: Vec<String> = v.split(',').map(core::settings::norm_process).filter(|p| !p.is_empty()).collect();
        log_line(&format!("debug: only these processes are moved: {}", names.join(", ")));
        o.set_only(names);
    }
    o.set_overlay_available(overlay_ui::available());
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
        write_state(o.state_json());
    }
}

/// Writes `state.json` (via a temporary file, so a crash never leaves half a file).
fn write_state(text: String) {
    if let Some(dir) = app_dir() {
        let tmp = dir.join("state.json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join("state.json"));
        }
    }
}

/// `--restore`: gives every managed window back without starting the tray.
fn restore_cli() -> i32 {
    use desktops::DesktopBackend;
    let mut out = Vec::new();
    let _apartment = desktops::Apartment::init();
    let code = match init_backend() {
        Err(_) => {
            out.push("no desktop backend; nothing done".to_string());
            1
        }
        Ok(b) => {
            let mut o = new_orchestrator(b);
            let count = |o: &Orch| o.backend().desktops().map_or(0, |l| l.len());
            out.push(format!("desktops before: {}", count(&o)));
            let st = load_state();
            out.push(format!("state.json: {} managed window(s), baseline {:?}", st.spaces.len(), st.baseline));
            o.adopt(st.spaces, st.baseline);
            o.release_pins(st.pinned);
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
    for s in o.drain_schedules() {
        schedule(hwnd, s);
    }
    carry_sync(hwnd, o);
    // A desktop that appeared gets its wallpaper (plan 12.17); cheap when nothing changed.
    wallpaper::sync(o.backend(), o.known_desktops(), o.carrying(), &o.settings.wallpapers, &mut |l| log_line(&l));
}

// ---------------------------------------------------------------- carrying a window (plan 12.13)

/// Registers or releases Win+Ctrl+Shift+Left/Right as the settings say (only with a backend).
fn update_hotkey(main: HWND) {
    let want = orch().is_some_and(|o| o.settings.carry_enabled);
    // Debug builds: `AUTOSPACES_NO_HOTKEY=1` leaves the shortcut to the installed instance.
    #[cfg(debug_assertions)]
    let want = want && std::env::var_os("AUTOSPACES_NO_HOTKEY").is_none();
    if want {
        if carry_ui::registered() {
            return;
        }
        if carry_ui::register(main) {
            log_line("carry: Win+Ctrl+Shift+Left/Right registered");
            if !carry_ui::raw_on(main) {
                log_line("carry: Raw Input refused; no preview while Win+Ctrl+Shift is held");
            }
        } else {
            log_line("carry: Win+Ctrl+Shift+Left/Right is taken by another app; carrying is off");
        }
    } else if !want {
        carry_ui::unregister(main);
        carry_ui::raw_off();
        preview_hide(true);
    }
}

/// Text for the settings dialog when carrying cannot work.
fn carry_status() -> Option<String> {
    if orch().is_none() {
        Some(i18n::t(i18n::Key::CarryUnavailable).to_string())
    } else if carry_ui::register_failed() {
        Some(i18n::t(i18n::Key::CarryHotkeyBusy).to_string())
    } else {
        None
    }
}

/// Win+Ctrl+Shift held alone (plan 12.13): the state machine fed by Raw Input, and whether the
/// layers on screen are a preview (no carry yet).
static mut CHORD: core::carry::Chord = core::carry::Chord::NEW;
static PREVIEW: AtomicBool = AtomicBool::new(false);

fn chord() -> &'static mut core::carry::Chord {
    unsafe { &mut *std::ptr::addr_of_mut!(CHORD) }
}

/// Debug builds: number of key events seen (AUTOSPACES_RAW_PROBE; only the count, no keys).
#[cfg(debug_assertions)]
static RAW_EVENTS: AtomicU64 = AtomicU64::new(0);
/// Debug builds: all WM_INPUT messages (the probe also registers the mouse, to show delivery).
#[cfg(debug_assertions)]
static RAW_MESSAGES: AtomicU64 = AtomicU64::new(0);

/// Milliseconds on a fine clock (the preview's delay is short).
fn fine_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// WM_INPUT: a key went up or down somewhere (only its kind is looked at).
fn on_raw_input(main: HWND, lp: LPARAM) {
    use core::carry::ChordAction;
    #[cfg(debug_assertions)]
    RAW_MESSAGES.fetch_add(1, Relaxed);
    let Some((key, down)) = carry_ui::raw_key(lp) else { return };
    #[cfg(debug_assertions)]
    RAW_EVENTS.fetch_add(1, Relaxed);
    let act = chord().key(key, down, fine_ms());
    if orch().is_some_and(|o| o.carrying()) {
        return; // the carry (and its keyboard hook) is in charge
    }
    match act {
        ChordAction::Arm => {
            // Built now, invisible: when the delay is over it only fades in.
            unsafe { SetTimer(main, TIMER_PREVIEW, core::carry::PREVIEW_MS, None) };
            preview_prepare();
        }
        ChordAction::Hide => preview_hide(false),
        ChordAction::HideNow => preview_hide(true),
        ChordAction::Show | ChordAction::None => {}
    }
    // Released (or another key) before the delay: the prepared layers go, unseen.
    if PREVIEW.load(Relaxed) && !chord().armed() && !chord().shown() {
        unsafe { KillTimer(main, TIMER_PREVIEW) };
        preview_hide(true);
    }
}

/// The preview's timer: Win+Ctrl+Shift has been held alone long enough.
fn on_preview_timer(main: HWND) {
    use core::carry::ChordAction;
    unsafe { KillTimer(main, TIMER_PREVIEW) };
    match chord().tick(fine_ms()) {
        ChordAction::Show => {
            if PREVIEW.load(Relaxed) {
                overlay_ui::reveal();
            } else {
                chord().reset_preview(); // could not be prepared (window cannot be moved, ...)
            }
        }
        // A timer may come a little early: wait for the rest.
        _ if chord().armed() => unsafe {
            SetTimer(main, TIMER_PREVIEW, 15, None);
        },
        _ => {}
    }
}

/// Builds the overlay for the foreground window, invisible, without carrying anything (false =
/// not here: the window cannot be moved, a full-screen app, the overlay is not used, ...).
fn preview_prepare() -> bool {
    use desktops::DesktopBackend;
    let Some(o) = orch() else { return false };
    let Some(w) = o.carry_preview() else { return false };
    let Ok(list) = o.backend().desktops() else { return false };
    let Some(cur) = o.backend().current().ok().and_then(|c| list.iter().position(|d| *d == c)) else { return false };
    let b = o.backend();
    let desk_of = |h: HWND| b.window_desktop(h);
    let res = overlay_ui::begin(overlay_ui::Begin {
        carried: w,
        desktops: list,
        start: cur,
        target: cur,
        create_new: o.settings.carry_create_new,
        full: true,
        hidden: true,
        desk_of: &desk_of,
        word: i18n::t(i18n::Key::DesktopWord),
    });
    match res {
        Ok(()) => {
            PREVIEW.store(true, Relaxed);
            true
        }
        Err(e) => {
            log_line(&format!("carry: preview cannot be drawn: {e}"));
            overlay_ui::destroy();
            false
        }
    }
}

/// Ends a preview: fading (`now` = false) or at once (another key: nothing may linger).
fn preview_hide(now: bool) {
    if PREVIEW.swap(false, Relaxed) {
        if now {
            overlay_ui::destroy();
        } else {
            overlay_ui::finish(None);
        }
    }
}

/// Time of the last carry key (GetTickCount64), for the safety timeout.
static CARRY_LAST_KEY: AtomicU64 = AtomicU64::new(0);
const CARRY_TIMEOUT_MS: u64 = 30_000;

fn on_hotkey(main: HWND, id: i32) {
    let Some(o) = orch() else { return };
    CARRY_LAST_KEY.store(unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }, Relaxed);
    let first = !o.carrying();
    let (outcome, sch) = o.carry_step(id == carry_ui::HK_RIGHT);
    match outcome {
        core::orchestrator::CarryOutcome::Carrying if first && o.carrying() => {
            // Win was used with a key: releasing it must not open the Start menu.
            carry_ui::mask_win_key();
            carry_ui::hook_on(main);
            unsafe { SetTimer(main, TIMER_CARRY_WATCH, CARRY_WATCH_MS, None) };
        }
        core::orchestrator::CarryOutcome::CannotMove => {
            carry_ui::mask_win_key();
            carry_ui::hint_show(i18n::t(i18n::Key::CarryCannotMove));
            unsafe { SetTimer(main, TIMER_HINT_HIDE, HINT_MS, None) };
        }
        _ => {}
    }
    after_orchestrator(main, o, sch);
}

/// The hook saw the modifiers released (`cancel` = false) or Esc (`cancel`).
fn on_carry_drop(main: HWND, cancel: bool) {
    if cancel {
        carry_ui::mask_win_key(); // Esc was swallowed while Win is held
    }
    if let Some(o) = orch() {
        let sch = o.carry_drop(cancel);
        after_orchestrator(main, o, sch);
    }
}

/// Starts the layers for the running carry (plan 12.13): `full` = the overlay method (background,
/// acrylic, stand-in, row), else only the row of previews. False = they cannot be drawn.
fn overlay_begin(o: &Orch, full: bool) -> bool {
    use desktops::DesktopBackend;
    let Some((list, start)) = o.carry_desktops() else { return false };
    let target = o.carry_target().map_or(start, |t| t.1);
    let b = o.backend();
    let desk_of = |h: HWND| b.window_desktop(h);
    let t0 = std::time::Instant::now();
    let res = overlay_ui::begin(overlay_ui::Begin {
        carried: o.carried(),
        desktops: list,
        start,
        target,
        create_new: o.settings.carry_create_new,
        full,
        hidden: false,
        desk_of: &desk_of,
        word: i18n::t(i18n::Key::DesktopWord),
    });
    match res {
        Ok(()) => {
            log_line(&format!("carry: layers ready in {} ms ({})", t0.elapsed().as_millis(), if full { "overlay" } else { "row only" }));
            true
        }
        Err(e) => {
            log_line(&format!("carry: layers cannot be drawn ({}): {e}", if full { "overlay" } else { "row only" }));
            false
        }
    }
}

/// Keeps the hook, the safety timer and the layers (the row of previews is the only indicator,
/// for every method) in line with the orchestrator.
fn carry_sync(main: HWND, o: &mut Orch) {
    use desktops::DesktopBackend;
    if o.carrying() {
        // From a preview: the carry goes on with its layers when they show the carried window.
        if PREVIEW.swap(false, Relaxed) {
            chord().reset_preview();
            unsafe { KillTimer(main, TIMER_PREVIEW) };
            let full = o.carry_style() == Some(core::settings::CarryStyle::Overlay);
            if !full || overlay_ui::carried() != Some(o.carried()) {
                overlay_ui::destroy();
            } else {
                overlay_ui::reveal(); // an arrow before the delay was over
            }
        }
        // A carry started while the last one's layers still fade: new layers.
        if overlay_ui::closing() {
            overlay_ui::destroy();
        }
        if overlay_ui::active() {
            if let (Some((_, target)), Some((list, _))) = (o.carry_target(), o.carry_desktops()) {
                overlay_ui::set_target(target, &list, i18n::t(i18n::Key::DesktopWord));
            }
            return;
        }
        let full = o.carry_style() == Some(core::settings::CarryStyle::Overlay);
        if overlay_begin(o, full) || !full {
            return;
        }
        // The overlay could not be drawn: this carry goes on with the next method (the next
        // carry tries the overlay again), with the row only.
        overlay_ui::destroy();
        let sch = o.carry_overlay_failed();
        for l in o.drain_log() {
            log_line(&l);
        }
        flush_state(o);
        if let Some(s) = sch {
            schedule(main, s);
        }
        if o.carrying() {
            overlay_begin(o, false);
        }
        return;
    }
    if overlay_ui::active() && !overlay_ui::closing() && !PREVIEW.load(Relaxed) {
        // The drop's switch (if any) is done: the layers show the desktop now in view and fade.
        let shown = o.backend().current().ok().and_then(|c| o.backend().desktops().ok()?.iter().position(|d| *d == c));
        overlay_ui::finish(shown);
    }
    if carry_ui::hook_installed() {
        carry_ui::hook_off();
        unsafe { KillTimer(main, TIMER_CARRY_WATCH) };
    }
}

/// Ends any carry (unpinning the window) and removes everything of the carry UI.
fn carry_shutdown(main: HWND) {
    if let Some(o) = orch() {
        o.carry_abort();
        for l in o.drain_log() {
            log_line(&l);
        }
        flush_state(o);
    }
    carry_ui::hook_off();
    carry_ui::unregister(main);
    carry_ui::hint_hide();
    carry_ui::raw_off();
    PREVIEW.store(false, Relaxed);
    unsafe { KillTimer(main, TIMER_PREVIEW) };
    overlay_ui::destroy();
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
        if id == TIMER_UPDATE {
            on_update_timer(main);
            return;
        }
        if id == TIMER_CARRY_WATCH {
            let idle = windows_sys::Win32::System::SystemInformation::GetTickCount64().saturating_sub(CARRY_LAST_KEY.load(Relaxed));
            if !orch().is_some_and(|o| o.carrying()) {
                KillTimer(main, id);
            } else if idle > CARRY_TIMEOUT_MS {
                // Safety: no key for this long (the hook may have missed the release): the
                // window goes back and the layers go away.
                log_line("carry: no key for 30 s; cancelled");
                on_carry_drop(main, true);
            } else if !carry_ui::modifiers_held() {
                on_carry_drop(main, false);
            }
            return;
        }
        #[cfg(debug_assertions)]
        if (90..=92).contains(&id) {
            debug_preview_step(main, id);
            return;
        }
        if id == TIMER_PREVIEW {
            on_preview_timer(main);
            return;
        }
        if id == TIMER_HINT_HIDE {
            KillTimer(main, id);
            carry_ui::hint_hide();
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
        Ok(b) => unsafe {
            if tray::mode() != tray::MODE_NORMAL {
                tray::set_mode(main, tray::MODE_NORMAL);
            }
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
                    o.release_pins(st.pinned);
                    *std::ptr::addr_of_mut!(ORCH) = Some(o);
                    if let Some(o) = orch() {
                        begin_startup(main, o);
                    }
                    update_hotkey(main);
                }
            }
            if let Some(o) = orch() {
                after_orchestrator(main, o, None);
            }
        },
        Err(why) => {
            let left = REINIT_LEFT.load(Relaxed);
            if left > 0 && !matches!(why, NoBackend::Forced(_)) {
                REINIT_LEFT.store(left - 1, Relaxed);
                unsafe { SetTimer(main, TIMER_REINIT, 1000, None) };
            } else if orch().is_none() {
                // Retries are over (or the backend is switched off): the app stays without one.
                enter_no_backend(main, &why);
            }
        }
    }
}

fn load_state() -> core::state::Stored {
    let none = core::state::Stored::empty;
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
    #[cfg(debug_assertions)]
    if let Ok(f) = std::env::var("AUTOSPACES_RAW_PROBE") {
        let _ = std::fs::write(f, format!("key events seen: {}, raw input messages (keyboard + mouse): {}\n", RAW_EVENTS.load(Relaxed), RAW_MESSAGES.load(Relaxed)));
    }
    carry_shutdown(hwnd);
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
        #[cfg(debug_assertions)]
        m if m == windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 12 => {
            open_report();
            0
        }
        WM_TIMER => {
            on_timer(hwnd, wp);
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_HOTKEY => {
            on_hotkey(hwnd, wp as i32);
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_INPUT => {
            on_raw_input(hwnd, lp);
            // The system cleans the raw input up.
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        wallpaper::WM_WALL_SHUFFLE => {
            if let (Some(w), Some(o)) = (wallpaper::take_pending(), orch().filter(|o| !o.carrying())) {
                log_line("wallpaper: shuffle now");
                wallpaper::apply(Some(o.backend()), &w, &mut |l| log_line(&l));
            }
            0
        }
        carry_ui::WM_CARRY_DROP => {
            on_carry_drop(hwnd, wp != 0);
            0
        }
        update::WM_UPDATE_CHECKED => {
            on_update_checked(hwnd, lp);
            0
        }
        tray::WM_TRAY => {
            if lp as u32 == windows_sys::Win32::UI::Shell::NIN_BALLOONUSERCLICK && BALLOON_UPDATE.load(Relaxed) {
                open_update(hwnd);
            }
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
                    tray::CMD_REPORT => open_report(),
                    tray::CMD_CHECK_UPDATES => check_updates_now(hwnd),
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
            carry_shutdown(hwnd);
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

/// Debug builds only: `AUTOSPACES_OVERLAY_DEMO=<folder>` draws the carry overlay over the current
/// desktop for about 4 s with the foreground window as the "carried" one, moves the highlight
/// (right, right, right, wrap, left) and fades out (AUTOSPACES_OVERLAY_DEMO_ROW=1: only the row,
/// as with the other methods); then shows the hint "cannot be moved" for a moment. Nothing is carried, switched or moved, no hotkey or hook; no other instance is
/// touched. Screenshots (BMP) and `demo.txt` (timing) go into the folder.
#[cfg(debug_assertions)]
fn overlay_demo(dir: &std::path::Path) -> i32 {
    use desktops::DesktopBackend;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetForegroundWindow, PeekMessageW, MSG, PM_REMOVE};
    let _ = std::fs::create_dir_all(dir);
    let apartment = desktops::Apartment::init();
    let mut out = Vec::new();
    let code = match init_backend() {
        Err(_) => {
            out.push("no backend".to_string());
            1
        }
        Ok(b) => {
            let list = b.desktops().unwrap_or_default();
            let cur = b.current().ok().and_then(|c| list.iter().position(|d| *d == c)).unwrap_or(0);
            // `AUTOSPACES_OVERLAY_DEMO_SAMPLE=1`: a sample window of the demo itself is "carried"
            // (so that the blur around it is seen even when the foreground window is maximized).
            let sample = std::env::var_os("AUTOSPACES_OVERLAY_DEMO_SAMPLE").map(|_| unsafe { demo_sample_window() });
            let carried = sample.unwrap_or_else(|| unsafe { GetForegroundWindow() });
            out.push(format!("desktops {}, current {}, overlay available {}", list.len(), cur + 1, overlay_ui::available()));
            let desk_of = |h: HWND| b.window_desktop(h);
            let shots = std::env::var_os("AUTOSPACES_OVERLAY_DEMO_NOSHOTS").is_none().then_some(dir);
            let full = std::env::var_os("AUTOSPACES_OVERLAY_DEMO_ROW").is_none();
            out.push(overlay_ui::demo(carried, list.clone(), cur, full, &desk_of, shots));
            carry_ui::hint_show(i18n::t(i18n::Key::CarryCannotMove));
            let t = std::time::Instant::now();
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while t.elapsed().as_millis() < 500 {
                unsafe {
                    while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if shots.is_some() {
                overlay_ui::capture(&dir.join("overlay-6-hint-cannot-move.bmp"));
            }
            carry_ui::hint_hide();
            // A quiet moment after the layers are gone (memory can be read from outside).
            std::thread::sleep(std::time::Duration::from_millis(1500));
            if let Some(h) = sample {
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(h) };
            }
            0
        }
    };
    drop(apartment);
    let _ = std::fs::write(dir.join("demo.txt"), out.join("\n") + "\n");
    code
}

/// Debug builds: a plain captioned window of the demo (not activated, not on the taskbar).
#[cfg(debug_assertions)]
unsafe fn demo_sample_window() -> HWND {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ShowWindow, SW_SHOWNOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
    };
    use windows_sys::Win32::Graphics::Gdi::{UpdateWindow, COLOR_WINDOW};
    let hinst = GetModuleHandleW(std::ptr::null());
    let class = tray::wide("AutoSpacesDemoSample");
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(demo_sample_proc);
    wc.hInstance = hinst;
    wc.hbrBackground = (COLOR_WINDOW + 1) as usize as _;
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc);
    let title = tray::wide("AutoSpaces – ukázkové okno (demo)");
    let h = CreateWindowExW(WS_EX_TOOLWINDOW, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW, 700, 300, 1160, 620, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
    ShowWindow(h, SW_SHOWNOACTIVATE);
    UpdateWindow(h);
    h
}

/// Debug builds: the sample window shows fine detail (1 px lines, small text) for the sharpness check.
#[cfg(debug_assertions)]
unsafe extern "system" fn demo_sample_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, LineTo, MoveToEx, SetBkMode, TextOutW, PAINTSTRUCT, TRANSPARENT};
    if msg == windows_sys::Win32::UI::WindowsAndMessaging::WM_PAINT {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(h, &mut ps);
        for i in 0..120 {
            let x = 20 + i * 3;
            MoveToEx(dc, x, 20, std::ptr::null_mut());
            LineTo(dc, x, 200);
        }
        SetBkMode(dc, TRANSPARENT as i32);
        for row in 0..14 {
            let t = tray::wide("Ostrost 1:1 – AutoSpaces přenášení okna, malé písmo 0123456789 ilI1| ~~~");
            TextOutW(dc, 400, 20 + row * 18, t.as_ptr(), t.len() as i32 - 1);
        }
        EndPaint(h, &ps);
        return 0;
    }
    DefWindowProcW(h, msg, wp, lp)
}
/// Debug builds only. `AUTOSPACES_RAW_PROBE=<file>`: keyboard Raw Input is registered at start (also
/// with `AUTOSPACES_NO_HOTKEY`); at exit the number of key events seen goes to the file (only
/// the count). `AUTOSPACES_PREVIEW_DEMO=<folder>`: 3 s after start the Win+Ctrl+Shift preview is
/// shown for the foreground window exactly as when the keys are held (no keys are sent), a
/// screenshot is taken, it fades away and the app exits.
#[cfg(debug_assertions)]
fn debug_preview_start(main: HWND) {
    if std::env::var_os("AUTOSPACES_RAW_PROBE").is_some() {
        log_line(&format!("debug: raw input {}", carry_ui::raw_on(main)));
        unsafe {
            use windows_sys::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK};
            let devs = [
                RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget: main },
                RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: main },
            ];
            RegisterRawInputDevices(devs.as_ptr(), 2, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
        }
        // Exits after AUTOSPACES_RAW_PROBE_SECS (20) s; the count is written on the way out.
        let secs = std::env::var("AUTOSPACES_RAW_PROBE_SECS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(20);
        unsafe { SetTimer(main, 92, secs * 1000, None) };
    }
    if std::env::var_os("AUTOSPACES_PREVIEW_DEMO").is_some() {
        unsafe { SetTimer(main, 90, 3000, None) };
    }
}

#[cfg(debug_assertions)]
fn debug_preview_step(main: HWND, id: usize) {
    unsafe { KillTimer(main, id) };
    let dir = std::env::var("AUTOSPACES_PREVIEW_DEMO").unwrap_or_default();
    let dir = std::path::Path::new(&dir);
    match id {
        90 => {
            let t = std::time::Instant::now();
            let shown = preview_prepare();
            overlay_ui::reveal();
            let _ = std::fs::write(dir.join("preview.txt"), format!("preview shown {shown} in {} ms\n", t.elapsed().as_millis()));
            unsafe { SetTimer(main, 91, 700, None) };
        }
        91 => {
            overlay_ui::capture(&dir.join("preview.bmp"));
            preview_hide(false);
            unsafe { SetTimer(main, 92, 600, None) };
        }
        _ => unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(main, WM_CLOSE, 0, 0);
        },
    }
}

/// Single-instance mutex (the installer's `AppMutex` is the same name) and the hidden window's class.
const SINGLE_INSTANCE: &str = r"Local\AutoSpaces_SingleInstance";
const WINDOW_CLASS: &str = "AutoSpacesHiddenWindow";

/// Debug builds only: `AUTOSPACES_INSTANCE=x` lets a test copy run beside an installed instance.
fn instance_mutex_name() -> String {
    #[cfg(debug_assertions)]
    if let Ok(x) = std::env::var("AUTOSPACES_INSTANCE") {
        return format!("{SINGLE_INSTANCE}_{x}");
    }
    SINGLE_INSTANCE.to_string()
}

fn main() {
    LOGGING.store(logging_wanted(settings_path().is_some_and(|p| Settings::load(&p).log)), Relaxed);
    if std::env::args().nth(1).as_deref() == Some("--selftest") {
        let apartment = desktops::Apartment::init();
        let code = selftest::run(std::env::args().nth(2).as_deref() == Some("--select-only"));
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
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("AUTOSPACES_OVERLAY_DEMO") {
        std::process::exit(overlay_demo(std::path::Path::new(&dir)));
    }
    unsafe {
        let mutex = CreateMutexW(std::ptr::null(), 0, tray::wide(&instance_mutex_name()).as_ptr());
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
        wallpaper::set_main(hwnd);

        let _apartment = desktops::Apartment::init();
        i18n::set_pref(settings_path().map_or(i18n::Pref::Auto, |p| Settings::load(&p).language));
        let mut no_backend = None;
        match init_backend() {
            Ok(b) => {
                let mut o = new_orchestrator(b);
                let st = load_state();
                o.adopt(st.spaces, st.baseline);
                o.release_pins(st.pinned);
                *std::ptr::addr_of_mut!(ORCH) = Some(o);
                if let Some(o) = orch() {
                    // Arrangement + layout profile as one sequence, after the delay (plan 12.11).
                    begin_startup(hwnd, o);
                }
            }
            // Switched off on purpose: no retries, say so at once (after the icon exists).
            Err(why @ NoBackend::Forced(_)) => no_backend = Some(why),
            Err(_) => {
                // Explorer may still be starting: retry; the "not supported" mode follows only
                // when every retry failed.
                REINIT_LEFT.store(REINIT_TRIES, Relaxed);
                SetTimer(hwnd, TIMER_REINIT, 1000, None);
            }
        }

        tray::add(hwnd);
        update_hotkey(hwnd);
        if let Some(why) = no_backend {
            enter_no_backend(hwnd, &why);
        }
        support_init(hwnd);
        // First update check 10 minutes after start (plan 12.15), then daily.
        update_arm(hwnd);
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_SHOW_SUPPORT").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 11, 0, 0);
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_OPEN_ABOUT").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10, 0, 0);
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_OPEN_REPORT").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 12, 0, 0);
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("AUTOSPACES_OPEN_SETTINGS").is_some() {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 9, 0, 0);
        }
        #[cfg(debug_assertions)]
        debug_preview_start(hwnd);
        init_reg_notify();
        *std::ptr::addr_of_mut!(WATCHER) = Some(watcher::Watcher::start(hwnd));

        let mut msg: MSG = std::mem::zeroed();
        'run: loop {
            let ev = *std::ptr::addr_of!(REG_EVENT);
            let handles = [ev];
            let n = if ev.is_null() { 0 } else { 1 };
            // While the carry overlay moves or fades, frames are paced by DwmFlush (vsync) in
            // `overlay_ui::frame`; otherwise the loop sleeps until something happens.
            let wait = if overlay_ui::animating() { 0 } else { u32::MAX };
            let r = MsgWaitForMultipleObjectsEx(n, handles.as_ptr(), wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
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
            if overlay_ui::animating() {
                if let Some(line) = overlay_ui::frame() {
                    log_line(&line);
                }
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
