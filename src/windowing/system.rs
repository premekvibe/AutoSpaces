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
    fn app_name(&self, pid: u32) -> String {
        process_path(pid).map(|p| version_name(&p)).unwrap_or_default()
    }
    fn recent_user_input(&self, ms: u32) -> bool {
        unsafe {
            let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
            GetLastInputInfo(&mut li) != 0 && GetTickCount().wrapping_sub(li.dwTime) <= ms
        }
    }
}
