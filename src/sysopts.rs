//! Autostart (HKCU Run, plan 12.2). The taskbar "all desktops" choice is not written by us (plan 12.8).

use crate::tray::wide;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "AutoSpaces";

fn read_string(key: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut len,
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn open_for_write(key: &str) -> Option<HKEY> {
    let mut h: HKEY = std::ptr::null_mut();
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut h,
            std::ptr::null_mut(),
        )
    };
    (r == ERROR_SUCCESS).then_some(h)
}

fn write_string(key: &str, name: &str, v: &str) -> bool {
    let Some(h) = open_for_write(key) else { return false };
    let w = wide(v);
    let r = unsafe { RegSetValueExW(h, wide(name).as_ptr(), 0, REG_SZ, w.as_ptr() as *const u8, (w.len() * 2) as u32) };
    unsafe { RegCloseKey(h) };
    r == ERROR_SUCCESS
}

fn delete_value(key: &str, name: &str) -> bool {
    let mut h: HKEY = std::ptr::null_mut();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(key).as_ptr(), 0, KEY_SET_VALUE, &mut h) != ERROR_SUCCESS {
            return true; // no key, nothing to delete
        }
        let r = RegDeleteValueW(h, wide(name).as_ptr());
        RegCloseKey(h);
        r == ERROR_SUCCESS || r == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND
    }
}

// ---------------------------------------------------------------- autostart

/// Command line stored in the Run key: the exe path in quotes.
pub fn run_command(exe: &str) -> String {
    format!("\"{exe}\"")
}

pub fn autostart_enabled() -> bool {
    read_string(RUN_KEY, RUN_NAME).is_some()
}

pub fn set_autostart(on: bool) -> bool {
    if on {
        match std::env::current_exe() {
            Ok(p) => write_string(RUN_KEY, RUN_NAME, &run_command(&p.to_string_lossy())),
            Err(_) => false,
        }
    } else {
        delete_value(RUN_KEY, RUN_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_is_quoted() {
        assert_eq!(run_command(r"C:\Program Files\A\autospaces.exe"), r#""C:\Program Files\A\autospaces.exe""#);
    }
}
