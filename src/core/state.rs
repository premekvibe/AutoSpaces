//! `state.json` (de)serialisation (plan 6.5); file I/O lives in the host.

use super::model::ManagedSpace;
use crate::desktops::Guid;
use crate::json::{parse_json, quote, Json};

/// What `state.json` holds: managed spaces and the baseline desktop count B (plan 12.1).
pub struct Stored {
    pub spaces: Vec<ManagedSpace>,
    pub baseline: Option<usize>,
}

pub fn to_json(spaces: &[ManagedSpace], baseline: usize) -> String {
    let mut s = format!("{{\"version\":2,\"baseline\":{baseline},\"spaces\":[");
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

/// Entries with bad fields are skipped; a malformed file yields an error. The `borrowed`
/// field of older files is ignored.
pub fn from_json(text: &str) -> Result<Stored, String> {
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
            fullscreen: false,
        });
    }
    let baseline = match root.get("baseline") {
        Some(Json::Num(n)) if *n >= 1 => Some(*n as usize),
        _ => None,
    };
    Ok(Stored { spaces: out, baseline })
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
            fullscreen: false,
        }];
        let back = from_json(&to_json(&v, 4)).unwrap();
        assert_eq!(back.baseline, Some(4));
        let back = back.spaces;
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].desktop, g(1));
        assert_eq!(back[0].hwnd as usize, 0x1234);
        assert_eq!(back[0].process, "a\"b.exe");
        assert_eq!(from_json(&to_json(&[], 1)).unwrap().spaces.len(), 0);
        assert!(from_json("{").is_err());
    }

    #[test]
    fn reads_old_file_with_borrowed_and_no_baseline() {
        let old = r#"{"version":1,"spaces":[{"desktop":"00000001-0000-0000-0000-000000000000","hwnd":1,"pid":2,"origin":"00000001-0000-0000-0000-000000000000","process":"x.exe","class":"C","borrowed":true}]}"#;
        let st = from_json(old).unwrap();
        assert_eq!((st.spaces.len(), st.baseline), (1, None));
    }
}
