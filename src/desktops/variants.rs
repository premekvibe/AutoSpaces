//! `variants.json`: descriptions of the undocumented virtual desktop interfaces
//! (IIDs + vtable slot numbers, counted including IUnknown's 3 methods).
//! Parsed with a tiny JSON reader to keep the binary small.

use super::guid::Guid;

pub const EMBEDDED: &str = include_str!("../../variants.json");

/// `IApplicationViewCollection` – stable across versions so far; overridable per variant.
const DEFAULT_IID_VIEW_COLLECTION: &str = "1841C6D7-4F9D-42C0-AF41-8747538F10E5";
const DEFAULT_SLOT_GET_VIEW_FOR_HWND: usize = 6;

const REQUIRED_SLOTS: [&str; 9] = [
    "GetCount",
    "MoveViewToDesktop",
    "CanViewMoveDesktops",
    "GetCurrentDesktop",
    "GetDesktops",
    "SwitchDesktop",
    "CreateDesktop",
    "RemoveDesktop",
    "FindDesktop",
];
const OPTIONAL_SLOTS: [&str; 4] = [
    "SwitchDesktopWithAnimation",
    "WaitForAnimationToComplete",
    "SwitchDesktopAndMoveForegroundView",
    "SetDesktopName",
];
const MAX_SLOT: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Slots {
    pub get_count: usize,
    pub move_view: usize,
    pub can_view_move: usize,
    pub get_current: usize,
    pub get_desktops: usize,
    pub switch: usize,
    pub create: usize,
    pub remove: usize,
    pub find: usize,
    /// Optional slots present in this variant (name, slot).
    pub optional: Vec<(String, usize)>,
}

impl Slots {
    pub fn opt(&self, name: &str) -> Option<usize> {
        self.optional.iter().find(|(n, _)| n == name).map(|&(_, s)| s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: String,
    pub min_build: u32,
    pub max_build: Option<u32>,
    pub iid_manager: Guid,
    pub iid_desktop: Guid,
    pub iid_view_collection: Guid,
    /// Older Windows 11 builds take an extra HMONITOR as the first argument of
    /// GetCount / GetCurrentDesktop / GetDesktops / SwitchDesktop / CreateDesktop.
    pub monitor_param: bool,
    pub slots: Slots,
    pub desktop_get_id: usize,
    pub view_for_hwnd: usize,
}

impl Variant {
    pub fn matches_build(&self, build: u32) -> bool {
        build >= self.min_build && self.max_build.is_none_or(|m| build <= m)
    }
}

// ---------------------------------------------------------------- JSON

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_uint(&self) -> Option<u64> {
        match *self {
            Json::Num(n) if (0..4_000_000_000).contains(&n) => Some(n as u64),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {what} at byte {}", self.i))
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("unexpected token")
        }
    }
    fn value(&mut self, depth: u32) -> Result<Json, String> {
        if depth > 32 {
            return self.err("nesting too deep");
        }
        self.ws();
        match self.s.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'{') => {
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    m.push((k, self.value(depth + 1)?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return self.err("expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return self.err("expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let start = self.i;
                // Integers only: nothing in the schema needs fractions.
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()
                    .and_then(|t| t.parse::<i64>().ok())
                    .map(Json::Num)
                    .map_or_else(|| self.err("bad number"), Ok)
            }
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.i) != Some(&b'"') {
            return self.err("expected string");
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return self.err("unterminated string");
            };
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| "JSON: invalid UTF-8".into()),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return self.err("bad escape");
                    };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hex = self.s.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok());
                            let code = hex.and_then(|h| u32::from_str_radix(h, 16).ok());
                            self.i += 4;
                            match code.and_then(char::from_u32) {
                                Some(ch) => ch,
                                None => return self.err("bad \\u escape"),
                            }
                        }
                        _ => return self.err("bad escape"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }
}

pub fn parse_json(text: &str) -> Result<Json, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return p.err("trailing data");
    }
    Ok(v)
}

// ---------------------------------------------------------------- schema

fn guid_field(o: &Json, key: &str, default: Option<&str>) -> Result<Guid, String> {
    let s = match o.get(key) {
        Some(v) => v.as_str().ok_or_else(|| format!("'{key}' must be a string"))?,
        None => default.ok_or_else(|| format!("missing '{key}'"))?,
    };
    Guid::parse(s).ok_or_else(|| format!("'{key}' is not a GUID"))
}

fn slot_of(map: &Json, key: &str) -> Result<Option<usize>, String> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => match v.as_uint() {
            Some(n) if (3..MAX_SLOT as u64).contains(&n) => Ok(Some(n as usize)),
            _ => Err(format!("slot '{key}' must be an integer in 3..{MAX_SLOT}")),
        },
    }
}

fn required_slot(map: &Json, key: &str) -> Result<usize, String> {
    slot_of(map, key)?.ok_or_else(|| format!("missing slot '{key}'"))
}

fn build_field(o: &Json, key: &str) -> Result<Option<u32>, String> {
    match o.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v.as_uint().map(|n| Some(n as u32)).ok_or_else(|| format!("'{key}' must be a non-negative integer")),
    }
}

fn parse_variant(o: &Json) -> Result<Variant, String> {
    let name = o.get("name").and_then(Json::as_str).ok_or("missing 'name'")?.to_string();
    let ctx = |e: String| format!("variant '{name}': {e}");
    let slots = o.get("slots").ok_or_else(|| ctx("missing 'slots'".into()))?;
    if let Json::Obj(m) = slots {
        if let Some((k, _)) = m.iter().find(|(k, _)| !REQUIRED_SLOTS.contains(&k.as_str()) && !OPTIONAL_SLOTS.contains(&k.as_str())) {
            return Err(ctx(format!("unknown slot '{k}'")));
        }
    } else {
        return Err(ctx("'slots' must be an object".into()));
    }
    let r = |k: &str| required_slot(slots, k).map_err(ctx);
    let mut optional = Vec::new();
    for k in OPTIONAL_SLOTS {
        if let Some(s) = slot_of(slots, k).map_err(ctx)? {
            optional.push((k.to_string(), s));
        }
    }
    let desktop_slots = o.get("desktopSlots").ok_or_else(|| ctx("missing 'desktopSlots'".into()))?;
    let view_for_hwnd = match o.get("viewSlots") {
        Some(v) => required_slot(v, "GetViewForHwnd").map_err(ctx)?,
        None => DEFAULT_SLOT_GET_VIEW_FOR_HWND,
    };
    let monitor_param = match o.get("monitorParam") {
        None => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(ctx("'monitorParam' must be a bool".into())),
    };
    Ok(Variant {
        min_build: build_field(o, "minBuild").map_err(ctx)?.unwrap_or(0),
        max_build: build_field(o, "maxBuild").map_err(ctx)?,
        iid_manager: guid_field(o, "iidManagerInternal", None).map_err(ctx)?,
        iid_desktop: guid_field(o, "iidDesktop", None).map_err(ctx)?,
        iid_view_collection: guid_field(o, "iidViewCollection", Some(DEFAULT_IID_VIEW_COLLECTION)).map_err(ctx)?,
        monitor_param,
        slots: Slots {
            get_count: r("GetCount")?,
            move_view: r("MoveViewToDesktop")?,
            can_view_move: r("CanViewMoveDesktops")?,
            get_current: r("GetCurrentDesktop")?,
            get_desktops: r("GetDesktops")?,
            switch: r("SwitchDesktop")?,
            create: r("CreateDesktop")?,
            remove: r("RemoveDesktop")?,
            find: r("FindDesktop")?,
            optional,
        },
        desktop_get_id: required_slot(desktop_slots, "GetId").map_err(ctx)?,
        view_for_hwnd,
        name,
    })
}

pub fn parse_variants(text: &str) -> Result<Vec<Variant>, String> {
    let root = parse_json(text)?;
    let Some(Json::Arr(list)) = root.get("variants") else {
        return Err("missing 'variants' array".into());
    };
    list.iter().map(parse_variant).collect()
}

/// Embedded list, or `variants.json` next to the exe when present and valid.
/// Returns the list plus a note describing the source.
pub fn load() -> (Vec<Variant>, String) {
    let embedded = || parse_variants(EMBEDDED).unwrap_or_default();
    let path = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.join("variants.json")));
    match path.and_then(|p| std::fs::read_to_string(&p).ok().map(|t| (p, t))) {
        Some((p, text)) => match parse_variants(&text) {
            Ok(v) => (v, format!("variants: {}", p.display())),
            Err(e) => (embedded(), format!("variants: embedded ({} ignored: {e})", p.display())),
        },
        None => (embedded(), "variants: embedded".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_file_parses() {
        let v = parse_variants(EMBEDDED).unwrap();
        assert_eq!(v.len(), 1);
        let w = &v[0];
        assert_eq!(w.name, "win11-24h2");
        assert_eq!(w.min_build, 26100);
        assert_eq!(w.max_build, None);
        assert_eq!(w.iid_manager.to_string(), "53F5CA0B-158F-4124-900C-057158060B27");
        assert_eq!(w.iid_desktop.to_string(), "3F07F4BE-B107-441A-AF0F-39D82529072C");
        assert_eq!(w.iid_view_collection.to_string(), DEFAULT_IID_VIEW_COLLECTION);
        assert!(!w.monitor_param);
        let s = &w.slots;
        assert_eq!(
            (s.get_count, s.move_view, s.can_view_move, s.get_current, s.get_desktops),
            (3, 4, 5, 6, 7)
        );
        assert_eq!((s.switch, s.create, s.remove, s.find), (9, 11, 13, 14));
        assert_eq!(s.opt("SwitchDesktopAndMoveForegroundView"), Some(10));
        assert_eq!(s.opt("SetDesktopName"), Some(16));
        assert_eq!(s.opt("SwitchDesktopWithAnimation"), Some(22));
        assert_eq!(s.opt("WaitForAnimationToComplete"), Some(24));
        assert_eq!(w.desktop_get_id, 4);
        assert_eq!(w.view_for_hwnd, 6);
        assert!(w.matches_build(26100) && w.matches_build(26340) && !w.matches_build(22631));
    }

    const MIN: &str = r#"{"variants":[{"name":"x","minBuild":1,"maxBuild":5,"iidManagerInternal":"53F5CA0B-158F-4124-900C-057158060B27",
        "iidDesktop":"3F07F4BE-B107-441A-AF0F-39D82529072C","monitorParam":true,
        "slots":{"GetCount":3,"MoveViewToDesktop":4,"CanViewMoveDesktops":5,"GetCurrentDesktop":6,"GetDesktops":7,
                 "SwitchDesktop":9,"CreateDesktop":10,"RemoveDesktop":12,"FindDesktop":13},
        "desktopSlots":{"GetId":4},"viewSlots":{"GetViewForHwnd":7}}]}"#;

    #[test]
    fn optional_fields_and_build_range() {
        let v = &parse_variants(MIN).unwrap()[0];
        assert!(v.monitor_param);
        assert!(v.slots.optional.is_empty());
        assert_eq!(v.view_for_hwnd, 7);
        assert!(v.matches_build(5) && !v.matches_build(6) && !v.matches_build(0));
    }

    #[test]
    fn rejects_invalid_variants() {
        assert!(parse_variants(&MIN.replace("\"FindDesktop\":13", "\"Bogus\":13")).is_err());
        assert!(parse_variants(&MIN.replace("\"GetCount\":3", "\"GetCount\":2")).is_err());
        assert!(parse_variants(&MIN.replace("\"GetCount\":3", "\"GetCount\":3.5")).is_err());
        assert!(parse_variants(&MIN.replace("53F5CA0B-", "53F5CA0B")).is_err());
        assert!(parse_variants(r#"{"variants":{}}"#).is_err());
        assert!(parse_variants(r#"{"variants":[]} x"#).is_err());
    }

    #[test]
    fn json_basics() {
        let j = parse_json(r#" {"a":[1,-25,true,false,null],"b":"x\"\\\u0041\n"} "#).unwrap();
        assert_eq!(
            j.get("a"),
            Some(&Json::Arr(vec![Json::Num(1), Json::Num(-25), Json::Bool(true), Json::Bool(false), Json::Null]))
        );
        assert_eq!(j.get("b"), Some(&Json::Str("x\"\\A\n".into())));
        assert!(parse_json("{\"a\":}").is_err());
        assert!(parse_json("[1,]").is_err());
        assert!(parse_json("[1.5]").is_err());
        assert!(parse_json("\"abc").is_err());
        assert_eq!(parse_json("\u{feff}[]"), Ok(Json::Arr(vec![])));
    }
}
