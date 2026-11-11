#![windows_subsystem = "windows"]

mod desktops;
mod selftest;
mod tray;
mod windowing;

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, TranslateMessage, MSG, WM_DESTROY,
    WM_RBUTTONUP, WNDCLASSW,
};
use windowing::{classifier, watcher};

static LOGGING: AtomicBool = AtomicBool::new(false);
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
// Touched only from the single UI thread.
static mut WATCHER: Option<watcher::Watcher> = None;
// Initialised at startup; used by the orchestrator (phase 3). UI thread only.
static mut BACKEND: Option<desktops::InternalBackend> = None;

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

fn log_line(line: &str) {
    if !LOGGING.load(Relaxed) {
        return;
    }
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let dir = std::path::Path::new(&base).join("AutoSpaces");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("log.txt"))
    {
        let _ = writeln!(f, "{line}");
    }
}

fn handle_winevent(hwnd: HWND, event: u32) {
    if !tray::ENABLED.load(Relaxed) || !LOGGING.load(Relaxed) {
        return;
    }
    let state = classifier::window_state(hwnd);
    let class = classifier::class_name(hwnd);
    log_line(&format!(
        "{} {:#x} {:?} candidate={} class={}",
        watcher::event_name(event),
        hwnd as usize,
        state,
        classifier::is_candidate(hwnd),
        class
    ));
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        watcher::WM_WINEVENT => {
            handle_winevent(wp as HWND, lp as u32);
            0
        }
        tray::WM_TRAY => {
            if lp as u32 == WM_RBUTTONUP {
                match tray::show_menu(hwnd) {
                    tray::CMD_TOGGLE => {
                        tray::ENABLED.fetch_xor(true, Relaxed);
                    }
                    tray::CMD_EXIT => {
                        DestroyWindow(hwnd);
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
        *std::ptr::addr_of_mut!(BACKEND) = init_backend();

        tray::add(hwnd);
        *std::ptr::addr_of_mut!(WATCHER) = Some(watcher::Watcher::start(hwnd));

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // Release COM proxies before the apartment is torn down.
        *std::ptr::addr_of_mut!(BACKEND) = None;
    }
    log_line(&format!(
        "exit: callbacks_total={} callbacks_passed={}",
        watcher::TOTAL.load(Relaxed),
        watcher::PASSED.load(Relaxed)
    ));
}
