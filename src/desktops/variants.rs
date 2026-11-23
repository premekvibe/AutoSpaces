//! `variants.json`: descriptions of the undocumented virtual desktop interfaces
//! (IIDs + vtable slot numbers, counted including IUnknown's 3 methods).
//! Parsed with a tiny JSON reader to keep the binary small.

use super::guid::Guid;
use crate::json::{parse_json, Json};

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
    /// Checked on a real system (`"verified": true`). Unverified variants (slots taken from public
    /// sources only) need a closed build range and are used only after the read-only self-test.
    pub verified: bool,
    /// `IVirtualDesktopPinnedApps` (plan 12.13), only where two independent sources agree.
    pub pinned_apps: Option<PinnedApps>,
}

/// `IVirtualDesktopPinnedApps`: obtained by `QueryService(service, iid)` on the immersive shell.
/// Used only to pin a carried window for the duration of the carry (plan 12.13).
#[derive(Clone, Debug, PartialEq)]
pub struct PinnedApps {
    pub service: Guid,
    pub iid: Guid,
    pub is_view_pinned: usize,
    pub pin_view: usize,
    pub unpin_view: usize,
}

fn parse_pinned(o: &Json) -> Result<PinnedApps, String> {
    let slots = o.get("slots").ok_or("pinnedApps: missing 'slots'")?;
    if let Json::Obj(m) = slots {
        if let Some((k, _)) = m.iter().find(|(k, _)| !["IsViewPinned", "PinView", "UnpinView"].contains(&k.as_str())) {
            return Err(format!("pinnedApps: unknown slot '{k}'"));
        }
    } else {
        return Err("pinnedApps: 'slots' must be an object".into());
    }
    let p = PinnedApps {
        service: guid_field(o, "service", None).map_err(|e| format!("pinnedApps: {e}"))?,
        iid: guid_field(o, "iid", None).map_err(|e| format!("pinnedApps: {e}"))?,
        is_view_pinned: required_slot(slots, "IsViewPinned")?,
        pin_view: required_slot(slots, "PinView")?,
        unpin_view: required_slot(slots, "UnpinView")?,
    };
    // Same order in every known layout: IsViewPinned < PinView < UnpinView.
    if !(p.is_view_pinned < p.pin_view && p.pin_view < p.unpin_view) {
        return Err("pinnedApps: slots must be IsViewPinned < PinView < UnpinView".into());
    }
    Ok(p)
}

impl Variant {
    pub fn matches_build(&self, build: u32) -> bool {
        build >= self.min_build && self.max_build.is_none_or(|m| build <= m)
    }

    fn overlaps(&self, o: &Variant) -> bool {
        let (a_max, b_max) = (self.max_build.unwrap_or(u32::MAX), o.max_build.unwrap_or(u32::MAX));
        self.min_build <= b_max && o.min_build <= a_max
    }
}

/// Order of the methods common to every known layout (plan 5.1): GetCount < MoveViewToDesktop <
/// CanViewMoveDesktops < GetCurrentDesktop < GetDesktops < SwitchDesktop < CreateDesktop <
/// RemoveDesktop < FindDesktop. The self-test checks GetCount, GetCurrentDesktop, GetDesktops and
/// FindDesktop, so the untestable (changing) methods are bracketed by tested ones. A typo in a
/// user-supplied file that breaks this order is rejected before anything is called.
fn check_slot_order(s: &Slots) -> Result<(), String> {
    let order = [
        ("GetCount", s.get_count),
        ("MoveViewToDesktop", s.move_view),
        ("CanViewMoveDesktops", s.can_view_move),
        ("GetCurrentDesktop", s.get_current),
        ("GetDesktops", s.get_desktops),
        ("SwitchDesktop", s.switch),
        ("CreateDesktop", s.create),
        ("RemoveDesktop", s.remove),
        ("FindDesktop", s.find),
    ];
    if let Some(w) = order.windows(2).find(|w| w[0].1 >= w[1].1) {
        return Err(format!("slot '{}' must be below '{}'", w[0].0, w[1].0));
    }
    for (name, slot) in &s.optional {
        let ok = match name.as_str() {
            "SwitchDesktopAndMoveForegroundView" => *slot > s.switch && *slot < s.create,
            _ => *slot > s.find,
        };
        if !ok {
            return Err(format!("slot '{name}' is out of place"));
        }
    }
    let mut all: Vec<usize> = order.iter().map(|o| o.1).chain(s.optional.iter().map(|o| o.1)).collect();
    all.sort_unstable();
    if all.windows(2).any(|w| w[0] == w[1]) {
        return Err("two methods share a slot".into());
    }
    Ok(())
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
    let verified = match o.get("verified") {
        None => false, // unknown provenance = unverified
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(ctx("'verified' must be a bool".into())),
    };
    let max_build = build_field(o, "maxBuild").map_err(ctx)?;
    if !verified && max_build.is_none() {
        return Err(ctx("an unverified variant needs 'maxBuild'".into()));
    }
    let v = Variant {
        min_build: build_field(o, "minBuild").map_err(ctx)?.unwrap_or(0),
        max_build,
        verified,
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
        pinned_apps: match o.get("pinnedApps") {
            None | Some(Json::Null) => None,
            Some(p) => Some(parse_pinned(p).map_err(ctx)?),
        },
        name: name.clone(),
    };
    if v.max_build.is_some_and(|m| m < v.min_build) {
        return Err(ctx("'maxBuild' is below 'minBuild'".into()));
    }
    check_slot_order(&v.slots).map_err(ctx)?;
    Ok(v)
}

pub fn parse_variants(text: &str) -> Result<Vec<Variant>, String> {
    let root = parse_json(text)?;
    let Some(Json::Arr(list)) = root.get("variants") else {
        return Err("missing 'variants' array".into());
    };
    let list: Vec<Variant> = list.iter().map(parse_variant).collect::<Result<_, _>>()?;
    // One IID may stand for different layouts (B2F925B9 in 21H2 and early 22H2, 53F5CA0B in 23H2
    // and 24H2): only the build range tells them apart, so such ranges must not overlap.
    for (i, a) in list.iter().enumerate() {
        if let Some(b) = list[i + 1..].iter().find(|b| b.iid_manager == a.iid_manager && a.overlaps(b)) {
            return Err(format!("variants '{}' and '{}' share an IID and overlapping builds", a.name, b.name));
        }
    }
    Ok(list)
}

/// Embedded list, or `variants.json` next to the exe when present and valid.
/// Returns the list plus a note describing the source.
pub fn load() -> (Vec<Variant>, String) {
    let embedded = || parse_variants(EMBEDDED).unwrap_or_default();
    let path = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.join("variants.json")));
    // Notes name the file only: the folder may contain the user's name (they go to logs and reports).
    match path.and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(text) => match parse_variants(&text) {
            Ok(v) => (v, "variants: variants.json next to the exe".to_string()),
            Err(e) => (embedded(), format!("variants: embedded (variants.json next to the exe ignored: {e})")),
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
        let names: Vec<&str> = v.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["win11-24h2", "win11-23h2-3085", "win11-22h2-2215", "win11-22h2", "win11-21h2", "win10"]);
        // Only the variant checked on a real system is marked verified.
        assert!(v.iter().all(|x| x.verified == (x.name == "win11-24h2")));
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
        assert!(parse_variants(&MIN.replace("\"monitorParam\":true", "\"verified\":1")).is_err());
    }

    #[test]
    fn older_variants_from_sources() {
        let v = parse_variants(EMBEDDED).unwrap();
        let by = |n: &str| v.iter().find(|x| x.name == n).unwrap().clone();
        let w10 = by("win10");
        assert_eq!((w10.min_build, w10.max_build), (17763, Some(19045)));
        assert_eq!(w10.iid_manager.to_string(), "F31574D6-B682-4CDC-BD56-1827860ABEC6");
        assert_eq!(w10.iid_desktop.to_string(), "FF72FFDD-BE7E-43FC-9C03-AD81681E88E4");
        assert!(!w10.monitor_param && w10.slots.optional.is_empty());
        assert_eq!((w10.slots.switch, w10.slots.create, w10.slots.remove, w10.slots.find), (9, 10, 11, 12));
        // 21H2 and early 22H2 share the IID; 22H2 has GetAllCurrentDesktops at 7, so everything shifts.
        let (a, b) = (by("win11-21h2"), by("win11-22h2"));
        assert_eq!(a.iid_manager, b.iid_manager);
        assert!(a.monitor_param && b.monitor_param);
        assert_eq!((a.slots.get_desktops, a.slots.switch, a.slots.create, a.slots.find), (7, 9, 10, 13));
        assert_eq!((b.slots.get_desktops, b.slots.switch, b.slots.create, b.slots.find), (8, 10, 11, 14));
        assert!(!a.overlaps(&b));
        // 23H2 (UBR 3085+) shares the IID with 24H2 but lacks SwitchDesktopAndMoveForegroundView.
        let (c, d) = (by("win11-23h2-3085"), by("win11-24h2"));
        assert_eq!(c.iid_manager, d.iid_manager);
        assert_eq!((c.slots.create, c.slots.remove, c.slots.find), (10, 12, 13));
        assert!(!c.overlaps(&d) && c.matches_build(22631) && !c.matches_build(26100));
        // No animation slots where only one source lists them.
        for n in ["win10", "win11-21h2", "win11-22h2", "win11-22h2-2215", "win11-23h2-3085"] {
            assert_eq!(by(n).slots.opt("SwitchDesktopWithAnimation"), None, "{n}");
            assert!(!by(n).matches_build(26340), "{n}");
        }
    }

    #[test]
    fn unverified_needs_closed_range_and_sane_slots() {
        // Unverified without maxBuild: rejected; verified may stay open-ended.
        let open = MIN.replace("\"maxBuild\":5,", "");
        assert!(parse_variants(&open).is_err());
        assert!(parse_variants(&open.replace("\"monitorParam\"", "\"verified\":true,\"monitorParam\"")).is_ok());
        assert!(parse_variants(&MIN.replace("\"maxBuild\":5", "\"maxBuild\":0")).is_err());
        // Slot order and uniqueness.
        assert!(parse_variants(&MIN.replace("\"CreateDesktop\":10", "\"CreateDesktop\":8")).is_err());
        assert!(parse_variants(&MIN.replace("\"FindDesktop\":13", "\"FindDesktop\":11")).is_err());
        assert!(parse_variants(&MIN.replace("\"GetCount\":3", "\"GetCount\":30")).is_err());
        let opt = |s: &str| MIN.replace("\"FindDesktop\":13", &format!("\"FindDesktop\":13,{s}"));
        assert!(parse_variants(&opt("\"SetDesktopName\":15")).is_ok());
        assert!(parse_variants(&opt("\"SetDesktopName\":13")).is_err());
        assert!(parse_variants(&opt("\"SetDesktopName\":11")).is_err());
        assert!(parse_variants(&opt("\"SwitchDesktopAndMoveForegroundView\":8")).is_err());
        assert!(parse_variants(&opt("\"SwitchDesktopWithAnimation\":20,\"WaitForAnimationToComplete\":20")).is_err());
    }

    #[test]
    fn pinning_interface_from_two_sources_in_every_variant() {
        for v in parse_variants(EMBEDDED).unwrap() {
            let p = v.pinned_apps.as_ref().unwrap_or_else(|| panic!("{}", v.name));
            assert_eq!(p.service.to_string(), "B5A399E7-1C87-46B8-88E9-FC5747B171BD", "{}", v.name);
            assert_eq!(p.iid.to_string(), "4CE81583-1E4C-4632-A621-07A53543148F", "{}", v.name);
            assert_eq!((p.is_view_pinned, p.pin_view, p.unpin_view), (6, 7, 8), "{}", v.name);
        }
        // Optional: a variant without it parses; a wrong order or an unknown slot does not.
        assert_eq!(parse_variants(MIN).unwrap()[0].pinned_apps, None);
        let with = |slots: &str| {
            MIN.replace(
                "\"desktopSlots\"",
                &format!(
                    "\"pinnedApps\":{{\"service\":\"B5A399E7-1C87-46B8-88E9-FC5747B171BD\",\"iid\":\"4CE81583-1E4C-4632-A621-07A53543148F\",\"slots\":{{{slots}}}}},\"desktopSlots\""
                ),
            )
        };
        assert!(parse_variants(&with("\"IsViewPinned\":6,\"PinView\":7,\"UnpinView\":8")).is_ok());
        assert!(parse_variants(&with("\"IsViewPinned\":7,\"PinView\":6,\"UnpinView\":8")).is_err());
        assert!(parse_variants(&with("\"IsViewPinned\":6,\"PinView\":7")).is_err());
        assert!(parse_variants(&with("\"IsViewPinned\":6,\"PinView\":7,\"UnpinView\":8,\"PinAppID\":4")).is_err());
    }

    #[test]
    fn same_iid_needs_disjoint_builds() {
        let one = &MIN[r#"{"variants":["#.len()..MIN.len() - 2];
        let two = |second: &str| format!(r#"{{"variants":[{one},{second}]}}"#);
        let other = one.replace("\"name\":\"x\"", "\"name\":\"y\"");
        assert!(parse_variants(&two(&other)).is_err());
        assert!(parse_variants(&two(&other.replace("\"minBuild\":1,\"maxBuild\":5", "\"minBuild\":6,\"maxBuild\":9"))).is_ok());
        assert!(parse_variants(&two(&other.replace("53F5CA0B-", "A3175F2D-"))).is_ok());
    }
}
