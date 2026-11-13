//! Win32 implementation of `core::WindowSystem`.

use super::classifier;
use crate::core::{State, WindowSystem};
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};
use windows_sys::Win32::System::Threading::AttachThreadInput;

pub struct Win32Windows {
    start: Instant,
}

impl Win32Windows {
    pub fn new() -> Self {
        Win32Windows { start: Instant::now() }
    }
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

/// Creation time of process `pid` (FILETIME, 100 ns units); 0 when unknown.
pub fn process_start_time(pid: u32) -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(h);
        if ok == 0 {
            return 0;
        }
        (c.dwHighDateTime as u64) << 32 | c.dwLowDateTime as u64
    }
}

/// True when this process started within two minutes of the shell (Explorer) of the session:
/// a start with Windows (Run key) rather than by hand. No shell yet = logon still under way.
pub fn started_with_windows() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetShellWindow;
    let shell = unsafe { GetShellWindow() };
    if shell.is_null() {
        return true;
    }
    let (me, sh) = (process_start_time(unsafe { GetCurrentProcessId() }), process_start_time(pid_of(shell)));
    if me == 0 || sh == 0 {
        return false;
    }
    me.saturating_sub(sh) < 120 * 10_000_000
}

fn process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Starts an exe minimized and inactive (STARTF_USESHOWWINDOW + SW_SHOWMINNOACTIVE: classic apps
/// apply it to their first window, so it does not cover the user's work before it is moved).
fn create_process(command: &str, args: &str) -> bool {
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE;
    let exe = command.trim().trim_matches('"');
    if exe.is_empty() || !exe.to_ascii_lowercase().ends_with(".exe") {
        return false;
    }
    let mut line: Vec<u16> = format!("\"{exe}\" {args}").trim_end().encode_utf16().chain(std::iter::once(0)).collect();
    // Absolute paths start in their own folder (as a shortcut would); bare names are searched.
    let dir: Option<Vec<u16>> = std::path::Path::new(exe)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect());
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.dwFlags = STARTF_USESHOWWINDOW;
        si.wShowWindow = SW_SHOWMINNOACTIVE as u16;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            std::ptr::null(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(),
            dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            &si,
            &mut pi,
        );
        if ok == 0 {
            return false;
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        true
    }
}

/// AppUserModelID of a packaged process; None for classic apps.
fn aumid(pid: u32) -> Option<String> {
    use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
    if pid == 0 {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 256]; // APPLICATION_USER_MODEL_ID_MAX_LENGTH = 130
        let mut len = buf.len() as u32;
        let r = GetApplicationUserModelId(h, &mut len, buf.as_mut_ptr());
        CloseHandle(h);
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        (r == 0 && n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
    }
}

fn process_name(pid: u32) -> String {
    process_path(pid)
        .map(|p| p.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase())
        .unwrap_or_default()
}

/// FileDescription (else ProductName) of an exe; empty when it has no version info.
pub fn version_name(path: &str) -> String {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let wpath: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(wpath.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return String::new();
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(wpath.as_ptr(), 0, size, buf.as_mut_ptr() as *mut _) == 0 {
            return String::new();
        }
        // Language/codepage pairs first, then the usual English fallbacks.
        let mut langs: Vec<String> = Vec::new();
        let tw: Vec<u16> = r"\VarFileInfo\Translation".encode_utf16().chain(std::iter::once(0)).collect();
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(buf.as_ptr() as *const _, tw.as_ptr(), &mut p, &mut len) != 0 && !p.is_null() {
            let pairs = std::slice::from_raw_parts(p as *const u16, (len / 2) as usize);
            for c in pairs.chunks_exact(2) {
                langs.push(format!("{:04x}{:04x}", c[0], c[1]));
            }
        }
        langs.push("040904b0".into());
        langs.push("040904e4".into());
        for field in ["FileDescription", "ProductName"] {
            for l in &langs {
                let sub: Vec<u16> =
                    format!(r"\StringFileInfo\{l}\{field}").encode_utf16().chain(std::iter::once(0)).collect();
                let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
                let mut len = 0u32;
                if VerQueryValueW(buf.as_ptr() as *const _, sub.as_ptr(), &mut p, &mut len) != 0 && !p.is_null() && len > 1 {
                    let w = std::slice::from_raw_parts(p as *const u16, len as usize);
                    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
                    let name = String::from_utf16_lossy(&w[..end]).trim().to_string();
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
        }
        String::new()
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> i32 {
    let v = &mut *(lp as *mut Vec<HWND>);
    if classifier::is_candidate(hwnd) {
        v.push(hwnd);
    }
    1
}

impl WindowSystem for Win32Windows {
    fn state(&self, hwnd: HWND) -> State {
        classifier::window_state(hwnd)
    }
    fn is_candidate(&self, hwnd: HWND) -> bool {
        classifier::is_candidate(hwnd)
    }
    fn process(&self, hwnd: HWND) -> (String, u32) {
        let pid = pid_of(hwnd);
        (process_name(pid), pid)
    }
    fn class(&self, hwnd: HWND) -> String {
        classifier::class_name(hwnd)
    }
    fn app_windows(&self) -> Vec<HWND> {
        let mut v: Vec<HWND> = Vec::new();
        unsafe { EnumWindows(Some(collect), &mut v as *mut Vec<HWND> as LPARAM) };
        v
    }
    fn activate(&self, hwnd: HWND) {
        unsafe {
            if SetForegroundWindow(hwnd) != 0 {
                return;
            }
            // Windows refuses focus changes from background processes; borrow the
            // foreground thread's input state for a moment.
            let fg = GetForegroundWindow();
            let fg_tid = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
            let me = GetCurrentThreadId();
            if fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, 1) != 0 {
                SetForegroundWindow(hwnd);
                AttachThreadInput(me, fg_tid, 0);
            }
        }
    }
    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
    fn is_alive(&self, hwnd: HWND, pid: u32) -> bool {
        unsafe { IsWindow(hwnd) != 0 && pid_of(hwnd) == pid }
    }
    fn exe_stamp(&self, pid: u32) -> String {
        let Some(m) = process_path(pid).and_then(|p| std::fs::metadata(p).ok()) else {
            return String::new();
        };
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        format!("{}:{mtime}", m.len())
    }
    fn is_plain_toplevel(&self, hwnd: HWND) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindow, GetWindowLongW, IsWindowVisible, GWL_STYLE, GW_OWNER, WS_CHILD,
        };
        unsafe {
            IsWindow(hwnd) != 0
                && IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
                && GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CHILD == 0
        }
    }
    fn process_started(&self, pid: u32) -> u64 {
        process_start_time(pid)
    }
    fn app_name(&self, pid: u32) -> String {
        process_path(pid).map(|p| version_name(&p)).unwrap_or_default()
    }
    fn exe_path(&self, pid: u32) -> String {
        process_path(pid).unwrap_or_default()
    }
    fn launch_command(&self, pid: u32) -> String {
        match aumid(pid) {
            Some(id) => crate::core::settings::apps_folder(&id),
            None => self.exe_path(pid),
        }
    }
    fn launch(&self, command: &str, args: &str) -> bool {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE;
        // Classic apps: CreateProcess directly. ShellExecute of an exe loads the shell's
        // association machinery and extensions into this process (measured: +95 modules,
        // +108 MB private), and CreateProcess passes the show command exactly.
        let packaged = command.get(..6).is_some_and(|s| s.eq_ignore_ascii_case("shell:"));
        if !packaged && create_process(command, args) {
            return true;
        }
        // Packaged apps (shell:AppsFolder\<AUMID>; light, measured +0.5 MB) and whatever
        // CreateProcess cannot start (App Paths names, documents, elevation).
        let w = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let (op, file, params) = (w("open"), w(command), w(args));
        let p = if args.is_empty() { std::ptr::null() } else { params.as_ptr() };
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), p, std::ptr::null(), SW_SHOWNOACTIVATE) };
        r as usize > 32
    }
    fn hold_foreground(&self, on: bool) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{LockSetForegroundWindow, LSFW_LOCK, LSFW_UNLOCK};
        // Allowed only when this process may set the foreground itself - which is exactly
        // when a launched app could take it (no foreground window, input lock timed out).
        // Any user input (Alt, a click) lifts the lock at once.
        unsafe { LockSetForegroundWindow(if on { LSFW_LOCK } else { LSFW_UNLOCK }) };
    }
    fn is_cloaked(&self, hwnd: HWND) -> bool {
        use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        let mut v = 0u32;
        let hr = unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut v as *mut u32 as *mut _, std::mem::size_of::<u32>() as u32)
        };
        hr >= 0 && v != 0
    }
    fn foreground(&self) -> HWND {
        unsafe { GetForegroundWindow() }
    }
    fn stop_flash(&self, hwnd: HWND) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{FlashWindowEx, FLASHWINFO, FLASHW_STOP};
        let fi = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd, dwFlags: FLASHW_STOP, uCount: 0, dwTimeout: 0 };
        unsafe { FlashWindowEx(&fi) };
    }
    fn maximize_inactive(&self, hwnd: HWND) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, GetWindowPlacement, IsIconic, IsZoomed, SetWindowPlacement, ShowWindowAsync, GWL_STYLE,
            SW_MAXIMIZE, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WINDOWPLACEMENT, WPF_ASYNCWINDOWPLACEMENT,
            WPF_RESTORETOMAXIMIZED, WS_MAXIMIZEBOX,
        };
        unsafe {
            if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_MAXIMIZEBOX == 0 {
                return false;
            }
            if IsZoomed(hwnd) != 0 {
                return true;
            }
            // Never ask for a minimize here: a minimize request from this process makes shell
            // mods hooking it (e.g. a Windhawk minimize animation) load Direct3D into this
            // process (measured +63 MB private).
            if IsIconic(hwnd) == 0 {
                // Asynchronous: runs on the window's thread. Its activation stays inside that
                // thread once the foreground is back with the user's window (verified live on a
                // window on another desktop: maximized, no view switch, focus unchanged).
                return ShowWindowAsync(hwnd, SW_MAXIMIZE) != 0;
            }
            // Started minimized (classic apps honour SW_SHOWMINNOACTIVE): "restore to
            // maximized", then restored without activation (verified live, no switch, no focus).
            let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
            wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            if GetWindowPlacement(hwnd, &mut wp) == 0 {
                return false;
            }
            wp.flags = WPF_RESTORETOMAXIMIZED | WPF_ASYNCWINDOWPLACEMENT;
            wp.showCmd = SW_SHOWMINNOACTIVE as u32;
            SetWindowPlacement(hwnd, &wp) != 0 && ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE) != 0
        }
    }
    fn recent_user_input(&self, ms: u32) -> bool {
        unsafe {
            let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
            GetLastInputInfo(&mut li) != 0 && GetTickCount().wrapping_sub(li.dwTime) <= ms
        }
    }
}
