//! Choosing a variant of the internal interface (plan 5.1), free of COM so it can be tested.
//!
//! Safety order, cheapest and safest first:
//! 1. structural checks of `variants.json` (slot order, closed range for unverified variants,
//!    no two variants with one IID and overlapping builds) – done when parsing;
//! 2. the build range – a variant outside it is never touched;
//! 3. a variant whose earlier self-test never finished on this build (the process died during
//!    it) is skipped – see [`Guard`];
//! 4. `QueryService` with the variant's IID – Explorer answers E_NOINTERFACE for a wrong one;
//! 5. read-only calls compared with the registry, each one only after the previous one matched:
//!    GetCount, GetCurrentDesktop + GetId, GetDesktops (+ GetId of each), FindDesktop.
//!
//! No other method of a variant is called before all of this passed.

use super::guid::Guid;
use super::variants::Variant;

/// Read-only calls the self-test makes, in this order. Errors are human-readable.
pub trait Probe {
    fn count(&mut self) -> Result<i32, String>;
    fn current(&mut self) -> Result<Guid, String>;
    fn desktops(&mut self) -> Result<Vec<Guid>, String>;
    fn find(&mut self, id: Guid) -> Result<Guid, String>;
}

/// Compares the interface's view with the registry, stopping at the first mismatch so that a
/// wrong layout makes as few calls as possible. `reg_ids`: `VirtualDesktopIDs` (absent while
/// there has only ever been one desktop), `reg_cur`: `CurrentVirtualDesktop`.
pub fn self_test(p: &mut dyn Probe, reg_ids: Option<&[Guid]>, reg_cur: Option<Guid>) -> Result<String, String> {
    let n = p.count().map_err(|e| format!("GetCount: {e}"))?;
    let expected = reg_ids.map_or(1, |l| l.len());
    if n < 1 || n as usize != expected {
        return Err(format!("GetCount={n}, registry has {expected}"));
    }
    let cur = p.current().map_err(|e| format!("GetCurrentDesktop: {e}"))?;
    if cur.is_zero() {
        return Err("current desktop has a zero id".into());
    }
    match (reg_cur, reg_ids) {
        (Some(rc), _) if rc != cur => return Err(format!("current {cur} != registry {rc}")),
        (None, Some(l)) if !l.contains(&cur) => return Err(format!("current {cur} not in registry list")),
        _ => {}
    }
    let list = p.desktops().map_err(|e| format!("GetDesktops: {e}"))?;
    if list.len() != n as usize || !list.contains(&cur) {
        return Err(format!("GetDesktops returned {} desktop(s), current among them: {}", list.len(), list.contains(&cur)));
    }
    if let Some(l) = reg_ids {
        if l != list.as_slice() {
            return Err("GetDesktops order differs from the registry".into());
        }
    }
    let found = p.find(cur).map_err(|e| format!("FindDesktop: {e}"))?;
    if found != cur {
        return Err(format!("FindDesktop({cur}) returned {found}"));
    }
    Ok(format!("count {n}, current {cur}, list and FindDesktop match"))
}

/// Why a variant was not chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    Range,
    /// A previous self-test of this variant on this build did not finish.
    Crashed,
    /// QueryService refused the IID (HRESULT).
    Iid(i32),
    /// The view collection could not be obtained (HRESULT).
    Views(i32),
    SelfTest(String),
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Skip::Range => write!(f, "build range does not match"),
            Skip::Crashed => write!(f, "skipped: an earlier self-test on this build did not finish"),
            Skip::Iid(hr) => write!(f, "QueryService -> 0x{:08X}", *hr as u32),
            Skip::Views(hr) => write!(f, "view collection -> 0x{:08X}", *hr as u32),
            Skip::SelfTest(s) => write!(f, "self-test FAIL ({s})"),
        }
    }
}

/// What `choose` needs from the system for one variant.
pub trait Candidate {
    /// Steps 4 (+ the view collection): Err(Skip::Iid / Skip::Views) when refused.
    fn open(&mut self, v: &Variant) -> Result<(), Skip>;
    /// Step 5 on the variant opened last. Ok = description of what matched.
    fn test(&mut self, v: &Variant) -> Result<String, String>;
}

/// Variants worth trying on `build`, most specific (newest) range first; stable for equal ranges.
pub fn in_range(list: &[Variant], build: u32) -> Vec<&Variant> {
    let mut v: Vec<&Variant> = list.iter().filter(|v| v.matches_build(build)).collect();
    v.sort_by_key(|v| std::cmp::Reverse(v.min_build));
    v
}

/// Goes through the variants in the safe order; returns the index (into `list`) of the first one
/// that passed, plus the outcome for every variant tried. `guard` records unverified self-tests.
pub fn choose(
    list: &[Variant],
    build: u32,
    guard: &mut Guard,
    sys: &mut dyn Candidate,
) -> (Option<usize>, Vec<(String, Result<String, Skip>)>) {
    let mut out = Vec::new();
    for v in list.iter().filter(|v| !v.matches_build(build)) {
        out.push((v.name.clone(), Err(Skip::Range)));
    }
    for v in in_range(list, build) {
        if !v.verified && guard.crashed(&v.name, build) {
            out.push((v.name.clone(), Err(Skip::Crashed)));
            continue;
        }
        if let Err(s) = sys.open(v) {
            out.push((v.name.clone(), Err(s)));
            continue;
        }
        // An unverified layout could still be wrong in a way that kills this process during the
        // read-only calls: leave a note that only a finished test removes.
        if !v.verified {
            guard.begin(&v.name, build);
        }
        let r = sys.test(v);
        if !v.verified {
            guard.end();
        }
        match r {
            Ok(s) => {
                out.push((v.name.clone(), Ok(s)));
                let idx = list.iter().position(|x| std::ptr::eq(x, v));
                return (idx, out);
            }
            Err(s) => out.push((v.name.clone(), Err(Skip::SelfTest(s)))),
        }
    }
    (None, out)
}

/// Crash memory of the self-test (plan 5.1 step 4), kept as small text:
/// `pending <variant> <build>` while a test runs, `crashed <variant> <build>` for each test that
/// never finished. A pending line found at the next start becomes a crashed line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Guard {
    pub pending: Option<(String, u32)>,
    pub crashed: Vec<(String, u32)>,
    /// Set whenever the text must be written back.
    pub dirty: bool,
    /// File the notes are written to at once (none in tests).
    pub path: Option<std::path::PathBuf>,
}

impl Guard {
    /// Reads the notes from `path` (missing file = none) and writes them back at every change.
    pub fn open(path: std::path::PathBuf) -> Guard {
        let mut g = Guard::from_text(&std::fs::read_to_string(&path).unwrap_or_default());
        g.path = Some(path);
        g.flush();
        g
    }

    fn flush(&mut self) {
        if let (true, Some(p)) = (self.dirty, self.path.as_ref()) {
            // Nothing to remember: no file at all.
            let _ = if self.crashed.is_empty() && self.pending.is_none() {
                std::fs::remove_file(p).or(Ok(()))
            } else {
                std::fs::write(p, self.to_text())
            };
            self.dirty = false;
        }
    }

    pub fn from_text(text: &str) -> Guard {
        let mut g = Guard::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(kind), Some(name), Some(build)) = (it.next(), it.next(), it.next().and_then(|b| b.parse().ok())) else {
                continue;
            };
            match kind {
                // Left over from a run that died during the test.
                "pending" | "crashed" => {
                    if !g.crashed.iter().any(|(n, b)| n == name && *b == build) {
                        g.crashed.push((name.to_string(), build));
                    }
                    g.dirty |= kind == "pending";
                }
                _ => {}
            }
        }
        g
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (n, b) in &self.crashed {
            s.push_str(&format!("crashed {n} {b}\n"));
        }
        if let Some((n, b)) = &self.pending {
            s.push_str(&format!("pending {n} {b}\n"));
        }
        s
    }

    pub fn crashed(&self, name: &str, build: u32) -> bool {
        self.crashed.iter().any(|(n, b)| n == name && *b == build)
    }

    pub fn begin(&mut self, name: &str, build: u32) {
        self.pending = Some((name.to_string(), build));
        self.dirty = true;
        self.flush();
    }

    pub fn end(&mut self) {
        self.pending = None;
        self.dirty = true;
        self.flush();
    }

    /// Remembers at once that `name` must not be tried again on `build` (e.g. Explorer died
    /// during an undocumented call: the call lives in Explorer, so this process survives it).
    pub fn record_crash(&mut self, name: &str, build: u32) {
        if !self.crashed(name, build) {
            self.crashed.push((name.to_string(), build));
        }
        self.dirty = true;
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktops::variants::{parse_variants, EMBEDDED};

    fn g(n: u32) -> Guid {
        Guid::parse(&format!("{n:08X}-0000-0000-0000-000000000001")).unwrap()
    }

    /// Fake interface: answers from fixed values and records the calls made.
    struct Fake {
        count: Result<i32, String>,
        current: Guid,
        list: Vec<Guid>,
        find_returns: Option<Guid>,
        calls: Vec<&'static str>,
    }

    impl Fake {
        fn good(list: &[Guid], cur: Guid) -> Fake {
            Fake { count: Ok(list.len() as i32), current: cur, list: list.to_vec(), find_returns: None, calls: Vec::new() }
        }
    }

    impl Probe for Fake {
        fn count(&mut self) -> Result<i32, String> {
            self.calls.push("count");
            self.count.clone()
        }
        fn current(&mut self) -> Result<Guid, String> {
            self.calls.push("current");
            Ok(self.current)
        }
        fn desktops(&mut self) -> Result<Vec<Guid>, String> {
            self.calls.push("desktops");
            Ok(self.list.clone())
        }
        fn find(&mut self, id: Guid) -> Result<Guid, String> {
            self.calls.push("find");
            Ok(self.find_returns.unwrap_or(id))
        }
    }

    #[test]
    fn self_test_passes_when_everything_matches() {
        let reg = [g(1), g(2), g(3)];
        let mut f = Fake::good(&reg, g(2));
        assert!(self_test(&mut f, Some(&reg), Some(g(2))).is_ok());
        assert_eq!(f.calls, ["count", "current", "desktops", "find"]);
        // One desktop, no VirtualDesktopIDs value yet.
        let mut f = Fake::good(&[g(7)], g(7));
        assert!(self_test(&mut f, None, Some(g(7))).is_ok());
    }

    #[test]
    fn self_test_stops_at_first_mismatch() {
        let reg = [g(1), g(2)];
        // Wrong count: nothing else is called.
        let mut f = Fake::good(&reg, g(1));
        f.count = Ok(5);
        assert!(self_test(&mut f, Some(&reg), Some(g(1))).is_err());
        assert_eq!(f.calls, ["count"]);
        f.calls.clear();
        f.count = Err("0x80004001".into());
        assert!(self_test(&mut f, Some(&reg), Some(g(1))).is_err());
        assert_eq!(f.calls, ["count"]);
        // Wrong current desktop: GetDesktops and FindDesktop are not called.
        let mut f = Fake::good(&reg, g(2));
        assert!(self_test(&mut f, Some(&reg), Some(g(1))).is_err());
        assert_eq!(f.calls, ["count", "current"]);
        let mut f = Fake::good(&reg, Guid::ZERO);
        assert!(self_test(&mut f, Some(&reg), None).is_err());
        // Current unknown to the registry list.
        let mut f = Fake::good(&reg, g(9));
        assert!(self_test(&mut f, Some(&reg), None).is_err());
        // Desktop list in a different order.
        let mut f = Fake::good(&reg, g(1));
        f.list = vec![g(2), g(1)];
        assert!(self_test(&mut f, Some(&reg), Some(g(1))).is_err());
        assert_eq!(f.calls, ["count", "current", "desktops"]);
        // FindDesktop returning another desktop.
        let mut f = Fake::good(&reg, g(1));
        f.find_returns = Some(g(2));
        assert!(self_test(&mut f, Some(&reg), Some(g(1))).is_err());
        // Zero or negative counts.
        let mut f = Fake::good(&reg, g(1));
        f.count = Ok(0);
        assert!(self_test(&mut f, None, None).is_err());
    }

    /// System double: which IIDs Explorer accepts and which variants pass the self-test.
    struct Sys {
        accepts: Vec<&'static str>,
        passes: Vec<&'static str>,
        opened: Vec<String>,
        tested: Vec<String>,
    }

    impl Sys {
        fn new(accepts: &[&'static str], passes: &[&'static str]) -> Sys {
            Sys { accepts: accepts.to_vec(), passes: passes.to_vec(), opened: Vec::new(), tested: Vec::new() }
        }
    }

    impl Candidate for Sys {
        fn open(&mut self, v: &Variant) -> Result<(), Skip> {
            self.opened.push(v.name.clone());
            let iid = v.iid_manager.to_string();
            if self.accepts.iter().any(|a| iid.starts_with(a)) {
                Ok(())
            } else {
                Err(Skip::Iid(0x80004002u32 as i32))
            }
        }
        fn test(&mut self, v: &Variant) -> Result<String, String> {
            self.tested.push(v.name.clone());
            if self.passes.contains(&v.name.as_str()) {
                Ok("ok".into())
            } else {
                Err("mismatch".into())
            }
        }
    }

    fn pick(build: u32, sys: &mut Sys, guard: &mut Guard) -> Option<String> {
        let list = parse_variants(EMBEDDED).unwrap();
        choose(&list, build, guard, sys).0.map(|i| list[i].name.clone())
    }

    const ALL: [&str; 6] = ["win11-24h2", "win11-23h2-3085", "win11-22h2-2215", "win11-22h2", "win11-21h2", "win10"];

    #[test]
    fn selection_by_build_and_iid() {
        let mut gd = Guard::default();
        // This machine: 26340, 53F5CA0B accepted → the verified 24H2 variant, nothing else touched.
        let mut s = Sys::new(&["53F5CA0B"], &ALL);
        assert_eq!(pick(26340, &mut s, &mut gd).as_deref(), Some("win11-24h2"));
        assert_eq!(s.opened, ["win11-24h2"]);
        assert!(gd.to_text().is_empty(), "verified variants leave no guard notes");
        // 23H2 after UBR 3085: same IID as 24H2, but only the 22621..22631 layout is in range.
        let mut s = Sys::new(&["53F5CA0B"], &ALL);
        assert_eq!(pick(22631, &mut s, &mut gd).as_deref(), Some("win11-23h2-3085"));
        assert!(!s.opened.contains(&"win11-24h2".to_string()));
        // 22H2 between UBR 2215 and 3085: the 53F5CA0B variant is refused by IID first.
        let mut s = Sys::new(&["A3175F2D"], &ALL);
        assert_eq!(pick(22621, &mut s, &mut gd).as_deref(), Some("win11-22h2-2215"));
        assert_eq!(s.tested, ["win11-22h2-2215"]);
        // Early 22H2 vs 21H2: same IID, the build decides.
        let mut s = Sys::new(&["B2F925B9"], &ALL);
        assert_eq!(pick(22621, &mut s, &mut gd).as_deref(), Some("win11-22h2"));
        let mut s = Sys::new(&["B2F925B9"], &ALL);
        assert_eq!(pick(22000, &mut s, &mut gd).as_deref(), Some("win11-21h2"));
        assert_eq!(s.opened, ["win11-21h2"]);
        // Windows 10 22H2.
        let mut s = Sys::new(&["F31574D6"], &ALL);
        assert_eq!(pick(19045, &mut s, &mut gd).as_deref(), Some("win10"));
    }

    #[test]
    fn unsupported_builds_and_failed_self_tests() {
        let mut gd = Guard::default();
        // Windows 10 1607 / Server 2022: no variant in range, nothing is opened.
        for build in [14393, 20348, 22631 + 1, 25000] {
            let mut s = Sys::new(&["53F5CA0B", "A3175F2D", "B2F925B9", "F31574D6"], &ALL);
            assert_eq!(pick(build, &mut s, &mut gd), None, "{build}");
            assert!(s.opened.is_empty(), "{build}");
        }
        // Known IID, but the self-test fails: rejected, nothing else in range accepted.
        let mut s = Sys::new(&["B2F925B9"], &[]);
        assert_eq!(pick(22000, &mut s, &mut gd), None);
        assert_eq!(s.tested, ["win11-21h2"]);
        // Unknown IID on a known build (e.g. a newer servicing change): refused before any call.
        let mut s = Sys::new(&["4970BA3D"], &ALL);
        assert_eq!(pick(22631, &mut s, &mut gd), None);
        assert!(s.tested.is_empty());
        let list = parse_variants(EMBEDDED).unwrap();
        let (_, out) = choose(&list, 22631, &mut gd, &mut Sys::new(&[], &[]));
        assert_eq!(out.len(), list.len(), "every variant is reported");
        assert!(out.iter().any(|(n, r)| n == "win10" && *r == Err(Skip::Range)));
    }

    #[test]
    fn guard_remembers_unfinished_tests() {
        let mut gd = Guard::default();
        let mut s = Sys::new(&["B2F925B9"], &["win11-21h2"]);
        assert_eq!(pick(22000, &mut s, &mut gd).as_deref(), Some("win11-21h2"));
        assert_eq!(gd.pending, None);
        // Simulate a process that died during the self-test.
        gd.begin("win11-21h2", 22000);
        let after_crash = Guard::from_text(&gd.to_text());
        assert!(after_crash.dirty && after_crash.crashed("win11-21h2", 22000));
        assert!(!after_crash.crashed("win11-21h2", 22001));
        let mut gd = after_crash;
        let mut s = Sys::new(&["B2F925B9"], &["win11-21h2"]);
        assert_eq!(pick(22000, &mut s, &mut gd), None);
        assert!(s.opened.is_empty(), "a crashed variant is not even opened");
        // The text survives a round trip; junk lines are ignored.
        let t = gd.to_text();
        assert_eq!(Guard::from_text(&t).crashed, gd.crashed);
        assert!(Guard::from_text("garbage\npending x\ncrashed y notanumber\n").crashed.is_empty());
    }
}
