//! Learned list of unmovable apps (plan 6.7), persisted to `learned.json`.
//!
//! In memory a small `HashMap`; `blocked()` never touches the disk. The file is rewritten
//! only when an entry changes (never on the hot path before the desktop switch: failures
//! are recorded after a rejected enter or after an exit).

use super::settings::never_learn;
use crate::json::{parse_json, quote, Json};
use std::collections::HashMap;
use std::path::PathBuf;

/// Failure reasons, matching the table in plan 6.7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailReason {
    NoView,
    CannotMove,
    /// `MoveViewToDesktop` failed (HRESULT).
    MoveFailed(i32),
    MoveNotApplied,
    Flapping,
    ShortLived,
    #[allow(dead_code)] // fallback only (later phase)
    HideShowFailed,
    AccessDenied,
}

impl FailReason {
    pub fn name(self) -> &'static str {
        match self {
            FailReason::NoView => "NoView",
            FailReason::CannotMove => "CannotMove",
            FailReason::MoveFailed(_) => "MoveFailed",
            FailReason::MoveNotApplied => "MoveNotApplied",
            FailReason::Flapping => "Flapping",
            FailReason::ShortLived => "ShortLived",
            FailReason::HideShowFailed => "HideShowFailed",
            FailReason::AccessDenied => "AccessDenied",
        }
    }
    /// Strikes needed to block (Flapping: within `FLAPPING_WINDOW_S`). Never one: a single
    /// failure can be a window in an unusual moment. The orchestrator counts at most one strike
    /// per window and only for real attempts (plan 6.7), so these are distinct windows.
    fn threshold(self) -> u32 {
        threshold_of(self.name())
    }
}

/// Strikes needed to block, by reason name (also applied to entries loaded from older files).
fn threshold_of(reason: &str) -> u32 {
    match reason {
        // Cheap refusals (nothing was created): only a pattern is worth remembering.
        "NoView" | "CannotMove" | "HideShowFailed" | "Flapping" => 3,
        // A desktop was created and removed again for nothing.
        _ => 2,
    }
}

const REASONS: [&str; 8] = [
    "NoView",
    "CannotMove",
    "MoveFailed",
    "MoveNotApplied",
    "Flapping",
    "ShortLived",
    "HideShowFailed",
    "AccessDenied",
];
const FLAPPING_WINDOW_S: u64 = 600;

fn is_soft(reason: &str) -> bool {
    reason == "Flapping" || reason == "ShortLived"
}

pub trait Learned {
    /// In-memory lookup only (exact class, then `*`).
    fn blocked(&self, process: &str, class: &str) -> bool;
    /// Called when `blocked` was true, with the current exe stamp: drops the entry when the
    /// exe changed (app updated). Returns whether the entry still blocks.
    fn confirm(&mut self, process: &str, class: &str, stamp: &str) -> bool;
    /// Records one failure; returns true when this made the entry block.
    fn fail(&mut self, process: &str, class: &str, reason: FailReason, stamp: &str) -> bool;
    fn set_backend(&mut self, name: &str);
    /// Sorted process names with an applicable blocking entry.
    fn list_blocked(&self) -> Vec<String>;
    /// Forgets every entry of a process (all classes).
    fn forget(&mut self, process: &str);
    fn forget_all(&mut self);
    /// Blocked processes with their readable names (empty = unknown), sorted by process.
    fn list_named(&self) -> Vec<(String, String)> {
        self.list_blocked().into_iter().map(|p| (p, String::new())).collect()
    }
    /// Stores the readable name for every entry of `process` that has none yet.
    fn set_name(&mut self, _process: &str, _name: &str) {}
}

/// Blocks nothing, remembers nothing.
pub struct NoLearned;

impl Learned for NoLearned {
    fn blocked(&self, _: &str, _: &str) -> bool {
        false
    }
    fn confirm(&mut self, _: &str, _: &str, _: &str) -> bool {
        false
    }
    fn fail(&mut self, _: &str, _: &str, _: FailReason, _: &str) -> bool {
        false
    }
    fn set_backend(&mut self, _: &str) {}
    fn list_blocked(&self) -> Vec<String> {
        Vec::new()
    }
    fn forget(&mut self, _: &str) {}
    fn forget_all(&mut self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    reason: String,
    hresult: Option<i32>,
    strikes: u32,
    blocked: bool,
    backend: String,
    stamp: String,
    first: String,
    last: String,
    /// Readable app name for display only (empty = unknown).
    name: String,
}

type Key = (String, String);

pub struct LearnedStore {
    entries: HashMap<Key, Entry>,
    /// Recent Flapping strike times (unix s); memory only.
    recent: HashMap<Key, Vec<u64>>,
    backend: String,
    retry_days: u32,
    path: Option<PathBuf>,
    /// Unix seconds.
    clock: Box<dyn Fn() -> u64>,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn date_of(unix_s: u64) -> String {
    let (y, m, d) = civil_from_days((unix_s / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    ((1..=12).contains(&m) && (1..=31).contains(&d) && it.next().is_none()).then(|| days_from_civil(y, m, d))
}

impl LearnedStore {
    /// Loads `path` (missing or malformed file = empty list).
    pub fn load(path: PathBuf, backend: &str, retry_days: u32) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut s = Self::with_clock(Some(path), backend, retry_days, Box::new(unix_now));
        s.entries = from_json(&text).unwrap_or_default();
        if s.drop_outdated() {
            s.save();
        }
        s
    }

    /// Drops entries the current rules would not have made: system hosts and windows, and blocks
    /// with fewer strikes than needed now (older versions blocked after one `CannotMove`).
    fn drop_outdated(&mut self) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|k, e| !never_learn(&k.0, &k.1) && !(e.blocked && e.strikes < threshold_of(&e.reason)));
        self.entries.len() != before
    }

    pub fn with_clock(path: Option<PathBuf>, backend: &str, retry_days: u32, clock: Box<dyn Fn() -> u64>) -> Self {
        LearnedStore {
            entries: HashMap::new(),
            recent: HashMap::new(),
            backend: backend.to_string(),
            retry_days,
            path,
            clock,
        }
    }

    fn expired(&self, e: &Entry) -> bool {
        is_soft(&e.reason)
            && ((self.clock)() / 86_400) as i64 - parse_date(&e.last).unwrap_or(0) >= i64::from(self.retry_days)
    }

    fn active(&self, e: &Entry) -> bool {
        e.blocked && e.backend == self.backend && !self.expired(e)
    }

    /// Key of the applicable blocking entry for (process, class).
    fn find(&self, process: &str, class: &str) -> Option<Key> {
        [class, "*"]
            .into_iter()
            .map(|c| (process.to_string(), c.to_string()))
            .find(|k| self.entries.get(k).is_some_and(|e| self.active(e)))
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, to_json(&self.entries)).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

impl Learned for LearnedStore {
    fn blocked(&self, process: &str, class: &str) -> bool {
        self.find(process, class).is_some()
    }

    fn confirm(&mut self, process: &str, class: &str, stamp: &str) -> bool {
        let Some(key) = self.find(process, class) else { return false };
        if stamp.is_empty() {
            return true;
        }
        let e = self.entries.get_mut(&key).expect("found");
        if e.stamp.is_empty() {
            e.stamp = stamp.to_string(); // learned while the process could not be inspected
        } else if e.stamp != stamp {
            self.entries.remove(&key);
            self.recent.remove(&key);
            self.save();
            return false;
        } else {
            return true;
        }
        self.save();
        true
    }

    fn fail(&mut self, process: &str, class: &str, reason: FailReason, stamp: &str) -> bool {
        if never_learn(process, class) {
            return false;
        }
        let now = (self.clock)();
        let key: Key = (process.to_string(), class.to_string());
        if let Some(e) = self.entries.get(&key) {
            let changed = !e.stamp.is_empty() && !stamp.is_empty() && e.stamp != stamp;
            if changed || e.backend != self.backend || self.expired(e) {
                self.entries.remove(&key);
                self.recent.remove(&key);
            }
        }
        if self.blocked(process, class) {
            return false;
        }
        let today = date_of(now);
        let name = reason.name();
        let e = self.entries.entry(key.clone()).or_insert_with(|| Entry {
            reason: name.into(),
            hresult: None,
            strikes: 0,
            blocked: false,
            backend: self.backend.clone(),
            stamp: String::new(),
            first: today.clone(),
            last: today.clone(),
            name: String::new(),
        });
        if e.reason != name {
            e.reason = name.into();
            e.strikes = 0;
            self.recent.remove(&key);
        }
        e.hresult = if let FailReason::MoveFailed(hr) = reason { Some(hr) } else { None };
        if !stamp.is_empty() {
            e.stamp = stamp.to_string();
        }
        e.last = today;
        if reason == FailReason::Flapping {
            let v = self.recent.entry(key).or_default();
            v.retain(|t| now.saturating_sub(*t) < FLAPPING_WINDOW_S);
            v.push(now);
            e.strikes = v.len() as u32;
        } else {
            e.strikes += 1;
        }
        e.blocked = e.strikes >= reason.threshold();
        let blocked = e.blocked;
        self.save();
        blocked
    }

    fn set_backend(&mut self, name: &str) {
        self.backend = name.to_string();
    }

    fn list_blocked(&self) -> Vec<String> {
        let mut v: Vec<String> =
            self.entries.iter().filter(|(_, e)| self.active(e)).map(|(k, _)| k.0.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    fn list_named(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = Vec::new();
        for (k, e) in self.entries.iter().filter(|(_, e)| self.active(e)) {
            match v.iter_mut().find(|(p, _)| *p == k.0) {
                Some(x) => {
                    if x.1.is_empty() {
                        x.1 = e.name.clone();
                    }
                }
                None => v.push((k.0.clone(), e.name.clone())),
            }
        }
        v.sort();
        v
    }

    fn set_name(&mut self, process: &str, name: &str) {
        if name.is_empty() {
            return;
        }
        let mut changed = false;
        for (k, e) in self.entries.iter_mut() {
            if k.0 == process && e.name.is_empty() {
                e.name = name.to_string();
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    fn forget(&mut self, process: &str) {
        let before = self.entries.len();
        self.entries.retain(|k, _| k.0 != process);
        self.recent.retain(|k, _| k.0 != process);
        if self.entries.len() != before {
            self.save();
        }
    }

    fn forget_all(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.recent.clear();
            self.save();
        }
    }
}

fn to_json(entries: &HashMap<Key, Entry>) -> String {
    let mut keys: Vec<&Key> = entries.keys().collect();
    keys.sort();
    let mut s = String::from("{\"version\":1,\"entries\":[");
    for (i, k) in keys.into_iter().enumerate() {
        let e = &entries[k];
        if i > 0 {
            s.push(',');
        }
        let hr = e.hresult.map_or(String::new(), |h| format!("\"hresult\":\"{:#010X}\",", h as u32));
        s.push_str(&format!(
            "\n  {{\"process\":{},\"class\":{},\"reason\":{},{}\"strikes\":{},\"blocked\":{},\"backend\":{},\"exeStamp\":{},\"first\":{},\"last\":{}{}}}",
            quote(&k.0),
            quote(&k.1),
            quote(&e.reason),
            hr,
            e.strikes,
            e.blocked,
            quote(&e.backend),
            quote(&e.stamp),
            quote(&e.first),
            quote(&e.last),
            if e.name.is_empty() { String::new() } else { format!(",\"name\":{}", quote(&e.name)) }
        ));
    }
    s.push_str("\n]}\n");
    s
}

/// Entries with bad fields are skipped; a malformed file yields an error.
fn from_json(text: &str) -> Result<HashMap<Key, Entry>, String> {
    let root = parse_json(text)?;
    let Some(Json::Arr(list)) = root.get("entries") else {
        return Err("missing 'entries'".into());
    };
    let mut out = HashMap::new();
    for o in list {
        let text = |k: &str| o.get(k).and_then(Json::as_str).unwrap_or("").to_string();
        let (process, reason) = (text("process"), text("reason"));
        if process.is_empty() || !REASONS.contains(&reason.as_str()) {
            continue;
        }
        let class = match text("class") {
            c if c.is_empty() => "*".to_string(),
            c => c,
        };
        let hresult = o
            .get("hresult")
            .and_then(Json::as_str)
            .and_then(|h| u32::from_str_radix(h.trim_start_matches("0x").trim_start_matches("0X"), 16).ok())
            .map(|h| h as i32);
        let e = Entry {
            reason,
            hresult,
            strikes: o.get("strikes").and_then(Json::as_uint).unwrap_or(1) as u32,
            blocked: matches!(o.get("blocked"), Some(Json::Bool(true))),
            backend: text("backend"),
            stamp: text("exeStamp"),
            first: text("first"),
            last: text("last"),
            name: text("name"),
        };
        out.insert((process, class), e);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    const DAY: u64 = 86_400;
    const T0: u64 = 1_800_000_000;

    fn store(backend: &str) -> (LearnedStore, Rc<Cell<u64>>) {
        let t = Rc::new(Cell::new(T0));
        let t2 = t.clone();
        (LearnedStore::with_clock(None, backend, 30, Box::new(move || t2.get())), t)
    }

    /// Fails until the entry blocks (the number of strikes depends on the reason).
    fn block(s: &mut LearnedStore, p: &str, c: &str, r: FailReason, stamp: &str) {
        assert!((0..5).any(|_| s.fail(p, c, r, stamp)), "{p} {c} {r:?} never blocks");
    }

    fn strikes_to_block(reason: FailReason) -> u32 {
        let (mut s, _) = store("b");
        for n in 1..=5 {
            if s.fail("a.exe", "C", reason, "1:1") {
                assert!(s.blocked("a.exe", "C"));
                return n;
            }
            assert!(!s.blocked("a.exe", "C"));
        }
        0
    }

    #[test]
    fn strike_thresholds_per_reason() {
        for r in [FailReason::NoView, FailReason::CannotMove, FailReason::HideShowFailed, FailReason::Flapping] {
            assert_eq!(strikes_to_block(r), 3, "{r:?}");
        }
        for r in [
            FailReason::MoveFailed(-5),
            FailReason::AccessDenied,
            FailReason::MoveNotApplied,
            FailReason::ShortLived,
        ] {
            assert_eq!(strikes_to_block(r), 2, "{r:?}");
        }
    }

    #[test]
    fn flapping_needs_three_within_ten_minutes() {
        let (mut s, t) = store("b");
        assert!(!s.fail("a.exe", "C", FailReason::Flapping, ""));
        t.set(T0 + 300);
        assert!(!s.fail("a.exe", "C", FailReason::Flapping, ""));
        t.set(T0 + 700); // the first strike fell out of the window
        assert!(!s.fail("a.exe", "C", FailReason::Flapping, ""));
        t.set(T0 + 800);
        assert!(s.fail("a.exe", "C", FailReason::Flapping, ""));
    }

    #[test]
    fn exe_stamp_change_drops_entry() {
        let (mut s, _) = store("b");
        block(&mut s, "a.exe", "C", FailReason::CannotMove, "10:20");
        assert!(s.confirm("a.exe", "C", "10:20"));
        assert!(!s.confirm("a.exe", "C", "11:30"));
        assert!(!s.blocked("a.exe", "C"));
        // A failure with a new stamp also replaces a stale entry (strikes restart).
        s.fail("b.exe", "C", FailReason::ShortLived, "1:1");
        assert!(!s.fail("b.exe", "C", FailReason::ShortLived, "2:2"));
        // Unknown stamp (process already gone) is filled in later.
        block(&mut s, "c.exe", "C", FailReason::CannotMove, "");
        assert!(s.confirm("c.exe", "C", "5:5"));
        assert!(!s.confirm("c.exe", "C", "6:6"));
    }

    #[test]
    fn other_backend_entries_are_ignored() {
        let (mut s, _) = store("internal:a");
        block(&mut s, "a.exe", "C", FailReason::CannotMove, "1:1");
        assert!(s.blocked("a.exe", "C"));
        s.set_backend("fallback");
        assert!(!s.blocked("a.exe", "C"));
        assert!(s.list_blocked().is_empty());
        s.set_backend("internal:a");
        assert!(s.blocked("a.exe", "C"));
    }

    #[test]
    fn soft_reasons_expire_hard_never() {
        let (mut s, t) = store("b");
        s.fail("soft.exe", "C", FailReason::ShortLived, "");
        s.fail("soft.exe", "C", FailReason::ShortLived, "");
        block(&mut s, "hard.exe", "C", FailReason::MoveFailed(1), "");
        t.set(T0 + 29 * DAY);
        assert!(s.blocked("soft.exe", "C"));
        t.set(T0 + 31 * DAY);
        assert!(!s.blocked("soft.exe", "C"));
        assert!(s.blocked("hard.exe", "C"));
        // After expiry the app gets a fresh start.
        assert!(!s.fail("soft.exe", "C", FailReason::ShortLived, ""));
    }

    #[test]
    fn wildcard_class_forget_one_and_all() {
        let (mut s, _) = store("b");
        block(&mut s, "a.exe", "X", FailReason::CannotMove, "");
        block(&mut s, "a.exe", "Y", FailReason::CannotMove, "");
        block(&mut s, "b.exe", "X", FailReason::CannotMove, "");
        assert!(!s.blocked("a.exe", "Z"));
        let e = s.entries[&("b.exe".to_string(), "X".to_string())].clone();
        s.entries.insert(("w.exe".into(), "*".into()), e);
        assert!(s.blocked("w.exe", "anything"));
        assert_eq!(s.list_blocked(), ["a.exe", "b.exe", "w.exe"]);
        s.forget("a.exe");
        assert!(!s.blocked("a.exe", "X") && !s.blocked("a.exe", "Y") && s.blocked("b.exe", "X"));
        s.forget_all();
        assert!(s.list_blocked().is_empty());
    }

    #[test]
    fn json_roundtrip_and_dates() {
        let dir = std::env::temp_dir().join(format!("as-learned-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("learned.json");
        let t = Rc::new(Cell::new(T0));
        let t2 = t.clone();
        let mut s = LearnedStore::with_clock(Some(path.clone()), "internal:x", 30, Box::new(move || t2.get()));
        block(&mut s, "a\"b.exe", "Cls", FailReason::MoveFailed(0x8007_0005_u32 as i32), "99:123");
        s.fail("c.exe", "*", FailReason::Flapping, "");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"hresult\":\"0x80070005\"") && text.contains("\"exeStamp\":\"99:123\""));
        let back = LearnedStore::load(path.clone(), "internal:x", 30);
        assert_eq!(back.entries, s.entries);
        assert!(back.blocked("a\"b.exe", "Cls") && !back.blocked("c.exe", "*"));
        assert_eq!(back.entries[&("c.exe".to_string(), "*".to_string())].strikes, 1);
        // Forgetting is persisted.
        s.forget_all();
        assert!(LearnedStore::load(path, "internal:x", 30).entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(from_json("{").is_err());
        assert_eq!(date_of(1_794_355_200), "2026-11-11");
        assert_eq!(parse_date("2026-11-11"), Some(1_794_355_200 / 86_400));
        assert_eq!(parse_date("nope"), None);
    }

    #[test]
    fn names_old_and_new_format() {
        // Old file without "name": loads, name unknown.
        let old = r#"{"version":1,"entries":[{"process":"pdr.exe","class":"V","reason":"CannotMove","strikes":3,"blocked":true,"backend":"b","exeStamp":"1:2","first":"2026-11-11","last":"2026-11-11"}]}"#;
        let m = from_json(old).unwrap();
        assert_eq!(m[&("pdr.exe".to_string(), "V".to_string())].name, "");
        let dir = std::env::temp_dir().join(format!("as-learned-n-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("learned.json");
        std::fs::write(&path, old).unwrap();
        let mut s = LearnedStore::load(path.clone(), "b", 30);
        assert_eq!(s.list_named(), vec![("pdr.exe".to_string(), String::new())]);
        s.set_name("pdr.exe", "CyberLink PowerDirector 365");
        s.set_name("pdr.exe", "Other"); // an existing name is kept
        s.set_name("none.exe", "X");
        let back = LearnedStore::load(path, "b", 30);
        assert_eq!(back.list_named(), vec![("pdr.exe".to_string(), "CyberLink PowerDirector 365".to_string())]);
        assert!(back.blocked("pdr.exe", "V"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_processes_are_never_learned() {
        let (mut s, _) = store("b");
        for _ in 0..5 {
            assert!(!s.fail("lockapp.exe", "Windows.UI.Core.CoreWindow", FailReason::CannotMove, ""));
            assert!(!s.fail("explorer.exe", "CabinetWClass", FailReason::MoveFailed(1), ""));
            assert!(!s.fail("x.exe", "Windows.UI.Core.CoreWindow", FailReason::MoveFailed(1), ""));
        }
        assert!(s.entries.is_empty());
    }

    #[test]
    fn one_strike_blocks_from_old_versions_are_dropped_on_load() {
        // The learned.json of a user, as written by the previous version.
        let old = r#"{"version":1,"entries":[
  {"process":"audacity4.exe","class":"Qt6101QWindowIcon","reason":"CannotMove","strikes":1,"blocked":true,"backend":"b","exeStamp":"1:1","first":"2026-11-12","last":"2026-11-12"},
  {"process":"explorer.exe","class":"LockScreenBackstopFrame","reason":"CannotMove","strikes":1,"blocked":true,"backend":"b","exeStamp":"1:1","first":"2026-11-11","last":"2026-11-11"},
  {"process":"lockapp.exe","class":"Windows.UI.Core.CoreWindow","reason":"CannotMove","strikes":1,"blocked":true,"backend":"b","exeStamp":"1:1","first":"2026-11-11","last":"2026-11-11"},
  {"process":"msedge.exe","class":"Chrome_WidgetWin_1","reason":"CannotMove","strikes":1,"blocked":true,"backend":"b","exeStamp":"1:1","first":"2026-11-11","last":"2026-11-11"},
  {"process":"textinputhost.exe","class":"Windows.UI.Core.CoreWindow","reason":"CannotMove","strikes":1,"blocked":true,"backend":"b","exeStamp":"1:1","first":"2026-11-12","last":"2026-11-12"},
  {"process":"pdr.exe","class":"V","reason":"MoveFailed","hresult":"0x80004005","strikes":2,"blocked":true,"backend":"b","exeStamp":"1:1","first":"2026-11-12","last":"2026-11-12"},
  {"process":"half.exe","class":"V","reason":"CannotMove","strikes":1,"blocked":false,"backend":"b","exeStamp":"1:1","first":"2026-11-12","last":"2026-11-12"}
]}"#;
        let dir = std::env::temp_dir().join(format!("as-learned-m-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("learned.json");
        std::fs::write(&path, old).unwrap();
        let s = LearnedStore::load(path.clone(), "b", 30);
        assert_eq!(s.list_blocked(), ["pdr.exe"]);
        assert_eq!(s.entries.len(), 2, "a pending strike stays");
        // The file itself was rewritten without them.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("msedge.exe") && !text.contains("lockapp.exe") && text.contains("pdr.exe"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
