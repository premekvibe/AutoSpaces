//! Desktop list / current desktop as recorded by Explorer in the registry.

use super::guid::Guid;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;

const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn read_binary(key: &str, value: &str) -> Option<Vec<u8>> {
    let (k, v) = (wide(key), wide(value));
    let mut len: u32 = 0;
    unsafe {
        let (k, v) = (k.as_ptr(), v.as_ptr());
        let no_type = std::ptr::null_mut();
        if RegGetValueW(HKEY_CURRENT_USER, k, v, RRF_RT_REG_BINARY, no_type, std::ptr::null_mut(), &mut len) != 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if RegGetValueW(
            HKEY_CURRENT_USER,
            k,
            v,
            RRF_RT_REG_BINARY,
            no_type,
            buf.as_mut_ptr().cast(),
            &mut len,
        ) != 0
        {
            return None;
        }
        buf.truncate(len as usize);
        Some(buf)
    }
}

/// Windows 10 keeps (some of) the values per session.
fn session_key() -> String {
    let mut sid = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut sid) };
    format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{sid}\VirtualDesktops")
}

fn read_value(value: &str) -> Option<Vec<u8>> {
    read_binary(KEY, value).or_else(|| read_binary(&session_key(), value))
}

/// Concatenated 16-byte GUIDs → list. Trailing partial chunks are ignored.
pub fn parse_ids(blob: &[u8]) -> Vec<Guid> {
    blob.as_chunks::<16>().0.iter().filter_map(|c| Guid::from_bytes(c)).collect()
}

/// Desktop IDs in order; `None` when the value does not exist (e.g. never more than one desktop).
pub fn desktop_ids() -> Option<Vec<Guid>> {
    read_value("VirtualDesktopIDs").map(|b| parse_ids(&b))
}

pub fn current_desktop() -> Option<Guid> {
    read_value("CurrentVirtualDesktop").and_then(|b| Guid::from_bytes(&b))
}

/// Update build revision (the "3085" in 22631.3085), for diagnostics only.
pub fn ubr() -> Option<u32> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let (k, v) = (wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), wide("UBR"));
    let (mut val, mut len) = (0u32, 4u32);
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut val as *mut u32).cast(),
            &mut len,
        )
    };
    (r == 0).then_some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_to_guid_list() {
        let a = Guid::parse("11111111-2222-3333-4455-66778899AABB").unwrap();
        let b = Guid::parse("3F07F4BE-B107-441A-AF0F-39D82529072C").unwrap();
        let mut blob = Vec::new();
        for g in [a, b] {
            let raw: [u8; 16] = unsafe { std::mem::transmute(g) };
            blob.extend_from_slice(&raw);
        }
        assert_eq!(parse_ids(&blob), vec![a, b]);
        blob.push(0xFF); // garbage tail
        assert_eq!(parse_ids(&blob), vec![a, b]);
        assert!(parse_ids(&[]).is_empty());
        // explicit byte order: first field is little-endian
        assert_eq!(parse_ids(&blob[..16])[0].d1, 0x11111111);
        assert_eq!(&blob[4..6], &[0x22, 0x22]);
    }
}
