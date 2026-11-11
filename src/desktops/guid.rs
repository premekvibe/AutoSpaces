//! GUID with the in-memory layout of the Win32 `GUID` (little-endian first three fields).

use std::fmt;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

impl Guid {
    pub const ZERO: Guid = Guid { d1: 0, d2: 0, d3: 0, d4: [0; 8] };

    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    /// From the raw 16-byte memory representation (as stored in registry blobs).
    pub fn from_bytes(b: &[u8]) -> Option<Guid> {
        if b.len() != 16 {
            return None;
        }
        let mut d4 = [0u8; 8];
        d4.copy_from_slice(&b[8..16]);
        Some(Guid {
            d1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            d2: u16::from_le_bytes([b[4], b[5]]),
            d3: u16::from_le_bytes([b[6], b[7]]),
            d4,
        })
    }

    /// Parses `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`, optionally wrapped in braces.
    pub fn parse(s: &str) -> Option<Guid> {
        let s = s.trim();
        let s = s
            .strip_prefix('{')
            .and_then(|x| x.strip_suffix('}'))
            .unwrap_or(s);
        let b = s.as_bytes();
        if b.len() != 36 || b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return None;
        }
        let hex: Vec<u8> = b.iter().copied().filter(|&c| c != b'-').collect();
        if hex.len() != 32 {
            return None;
        }
        let mut v = [0u8; 16];
        for (i, pair) in hex.chunks(2).enumerate() {
            let h = (pair[0] as char).to_digit(16)?;
            let l = (pair[1] as char).to_digit(16)?;
            v[i] = (h * 16 + l) as u8;
        }
        let mut d4 = [0u8; 8];
        d4.copy_from_slice(&v[8..16]);
        Some(Guid {
            d1: u32::from_be_bytes([v[0], v[1], v[2], v[3]]),
            d2: u16::from_be_bytes([v[4], v[5]]),
            d3: u16::from_be_bytes([v[6], v[7]]),
            d4,
        })
    }

    pub fn as_ptr(&self) -> *const windows_sys::core::GUID {
        self as *const Guid as *const windows_sys::core::GUID
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = &self.d4;
        write!(
            f,
            "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            self.d1, self.d2, self.d3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        let s = "53F5CA0B-158F-4124-900C-057158060B27";
        let g = Guid::parse(s).unwrap();
        assert_eq!(g.d1, 0x53F5CA0B);
        assert_eq!(g.d2, 0x158F);
        assert_eq!(g.d3, 0x4124);
        assert_eq!(g.d4, [0x90, 0x0C, 0x05, 0x71, 0x58, 0x06, 0x0B, 0x27]);
        assert_eq!(g.to_string(), s);
        assert_eq!(Guid::parse("{a5cd92ff-29be-454c-8d04-d82879fb3f1b}").unwrap().to_string(),
                   "A5CD92FF-29BE-454C-8D04-D82879FB3F1B");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Guid::parse("").is_none());
        assert!(Guid::parse("53F5CA0B-158F-4124-900C-057158060B2").is_none());
        assert!(Guid::parse("53F5CA0B_158F-4124-900C-057158060B27").is_none());
        assert!(Guid::parse("ZZF5CA0B-158F-4124-900C-057158060B27").is_none());
    }

    #[test]
    fn from_bytes_is_memory_layout() {
        let g = Guid::parse("3F07F4BE-B107-441A-AF0F-39D82529072C").unwrap();
        let raw: [u8; 16] = unsafe { std::mem::transmute(g) };
        assert_eq!(raw[0], 0xBE); // little-endian d1
        assert_eq!(Guid::from_bytes(&raw), Some(g));
        assert!(Guid::from_bytes(&raw[..15]).is_none());
    }
}
