//! `state.json` (de)serialisation (plan 6.5); file I/O lives in the host.

use super::model::ManagedSpace;
use crate::desktops::Guid;
use crate::json::{parse_json, quote, Json};

pub fn to_json(spaces: &[ManagedSpace]) -> String {
    let mut s = String::from("{\"version\":1,\"spaces\":[");
    for (i, m) in spaces.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "\n  {{\"desktop\":\"{}\",\"hwnd\":{},\"pid\":{},\"origin\":\"{}\",\"process\":{},\"class\":{}}}",
            m.desktop,
            m.hwnd as usize,
            m.pid,
            m.origin,
            quote(&m.process),
            quote(&m.class)
        ));
    }
    s.push_str("\n]}\n");
    s
}

/// Entries with bad fields are skipped; a malformed file yields an error.
pub fn from_json(text: &str) -> Result<Vec<ManagedSpace>, String> {
    let root = parse_json(text)?;
    let Some(Json::Arr(list)) = root.get("spaces") else {
        return Err("missing 'spaces'".into());
    };
    let mut out = Vec::new();
    for o in list {
        let guid = |k: &str| o.get(k).and_then(Json::as_str).and_then(Guid::parse);
        let num = |k: &str| match o.get(k) {
            Some(Json::Num(n)) if *n >= 0 => Some(*n as usize),
            _ => None,
        };
        let text = |k: &str| o.get(k).and_then(Json::as_str).unwrap_or("").to_string();
        let (Some(desktop), Some(origin), Some(hwnd), Some(pid)) =
            (guid("desktop"), guid("origin"), num("hwnd"), num("pid"))
        else {
            continue;
        };
        out.push(ManagedSpace {
            desktop,
            hwnd: hwnd as windows_sys::Win32::Foundation::HWND,
            pid: pid as u32,
            origin,
            entered_at: 0,
            process: text("process"),
            class: text("class"),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let g = |n: u8| Guid { d1: n as u32, d2: 2, d3: 3, d4: [n; 8] };
        let v = vec![ManagedSpace {
            desktop: g(1),
            hwnd: 0x1234 as _,
            pid: 77,
            origin: g(2),
            entered_at: 99,
            process: "a\"b.exe".into(),
            class: "Cls".into(),
        }];
        let back = from_json(&to_json(&v)).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].desktop, g(1));
        assert_eq!(back[0].hwnd as usize, 0x1234);
        assert_eq!(back[0].process, "a\"b.exe");
        assert_eq!(from_json(&to_json(&[])).unwrap().len(), 0);
        assert!(from_json("{").is_err());
    }
}
