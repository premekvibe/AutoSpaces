//! Support reminder (plan 12.9): when to ask for support, kept as pure functions of the time
//! so they can be tested. File I/O and the dialog live in the host.

use crate::json::{parse_json, Json};

pub const DAY_SECS: u64 = 86_400;
/// Days after the first run (and between reminders).
pub const REMIND_AFTER_SECS: u64 = 30 * DAY_SECS;
/// The app must have been running this long in the current run.
pub const MIN_UPTIME_SECS: u64 = 30 * 60;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Support {
    /// Unix time of the first run.
    pub first_run: Option<u64>,
    /// Earliest unix time of the next reminder (0 = no limit besides `first_run`).
    pub next_after: u64,
    /// "Don't show again" was ticked.
    pub never: bool,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Ask now.
    Show,
    /// Not yet; the earliest moment is this many seconds away.
    WaitSecs(u64),
    /// Never again (ticked, or already shown in this run).
    Done,
}

impl Support {
    /// Repairs values that cannot be right (first run in the future after the clock was set
    /// back, a reminder date more than one period ahead). Returns true when something changed.
    pub fn normalize(&mut self, now: u64) -> bool {
        let before = self.clone();
        match self.first_run {
            None => self.first_run = Some(now),
            Some(f) if f > now => self.first_run = Some(now),
            _ => {}
        }
        if self.next_after > now + REMIND_AFTER_SECS {
            self.next_after = now + REMIND_AFTER_SECS;
        }
        *self != before
    }

    /// `uptime` is how long this run has lasted; `shown_this_run` limits it to one reminder per run.
    pub fn verdict(&self, now: u64, uptime: u64, shown_this_run: bool) -> Verdict {
        if self.never || shown_this_run {
            return Verdict::Done;
        }
        let first = self.first_run.unwrap_or(now).min(now);
        let due = (first + REMIND_AFTER_SECS).max(self.next_after.min(now + REMIND_AFTER_SECS));
        let wait = due.saturating_sub(now).max(MIN_UPTIME_SECS.saturating_sub(uptime));
        if wait == 0 {
            Verdict::Show
        } else {
            Verdict::WaitSecs(wait)
        }
    }

    /// After the dialog was closed: never again, or the next reminder in 30 days.
    pub fn closed(&mut self, now: u64, never: bool) {
        if never {
            self.never = true;
        } else {
            self.next_after = now + REMIND_AFTER_SECS;
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"firstRun\":{},\"nextAfter\":{},\"never\":{}}}\n",
            self.first_run.unwrap_or(0),
            self.next_after,
            self.never
        )
    }

    /// Tolerant: anything unreadable gives the default (first run = now, set by `normalize`).
    pub fn from_json(text: &str) -> Support {
        let Ok(root) = parse_json(text) else { return Support::default() };
        let num = |k: &str| root.get(k).and_then(Json::as_uint);
        Support {
            first_run: num("firstRun").filter(|&n| n > 0),
            next_after: num("nextAfter").unwrap_or(0),
            never: matches!(root.get("never"), Some(Json::Bool(true))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000;
    const UP: u64 = MIN_UPTIME_SECS;

    fn fresh(now: u64) -> Support {
        let mut s = Support::default();
        assert!(s.normalize(now));
        s
    }

    #[test]
    fn first_run_is_recorded_and_waits_30_days() {
        let s = fresh(T0);
        assert_eq!(s.first_run, Some(T0));
        assert_eq!(s.verdict(T0, UP, false), Verdict::WaitSecs(REMIND_AFTER_SECS));
    }

    #[test]
    fn day_29_waits_day_30_shows() {
        let s = fresh(T0);
        let d29 = T0 + 29 * DAY_SECS + 23 * 3600;
        assert_eq!(s.verdict(d29, UP, false), Verdict::WaitSecs(3600));
        assert_eq!(s.verdict(T0 + REMIND_AFTER_SECS, UP, false), Verdict::Show);
        assert_eq!(s.verdict(T0 + 90 * DAY_SECS, UP, false), Verdict::Show);
    }

    #[test]
    fn needs_30_minutes_of_this_run() {
        let s = fresh(T0);
        let late = T0 + 40 * DAY_SECS;
        assert_eq!(s.verdict(late, 10 * 60, false), Verdict::WaitSecs(20 * 60));
        assert_eq!(s.verdict(late, UP, false), Verdict::Show);
    }

    #[test]
    fn ticked_means_never() {
        let mut s = fresh(T0);
        s.closed(T0 + 31 * DAY_SECS, true);
        assert_eq!(s.verdict(T0 + 400 * DAY_SECS, UP, false), Verdict::Done);
    }

    #[test]
    fn only_once_per_run() {
        let s = fresh(T0);
        assert_eq!(s.verdict(T0 + 40 * DAY_SECS, UP, true), Verdict::Done);
    }

    #[test]
    fn closed_without_tick_repeats_after_30_days() {
        let mut s = fresh(T0);
        let shown = T0 + 35 * DAY_SECS;
        s.closed(shown, false);
        assert!(!s.never);
        assert_eq!(s.verdict(shown + 29 * DAY_SECS, UP, false), Verdict::WaitSecs(DAY_SECS));
        assert_eq!(s.verdict(shown + 30 * DAY_SECS, UP, false), Verdict::Show);
        // the new run still needs its 30 minutes
        assert_eq!(s.verdict(shown + 30 * DAY_SECS, 60, false), Verdict::WaitSecs(UP - 60));
    }

    #[test]
    fn clock_set_back_is_repaired() {
        // first run in the future: treated as now, so a full 30 days again
        let mut s = Support { first_run: Some(T0 + 500 * DAY_SECS), next_after: 0, never: false };
        assert_eq!(s.verdict(T0, UP, false), Verdict::WaitSecs(REMIND_AFTER_SECS));
        assert!(s.normalize(T0));
        assert_eq!(s.first_run, Some(T0));
        // a reminder date far in the future (clock was ahead before) is capped to 30 days
        let mut s = Support { first_run: Some(T0 - 100 * DAY_SECS), next_after: T0 + 900 * DAY_SECS, never: false };
        assert_eq!(s.verdict(T0, UP, false), Verdict::WaitSecs(REMIND_AFTER_SECS));
        assert!(s.normalize(T0));
        assert_eq!(s.next_after, T0 + REMIND_AFTER_SECS);
    }

    #[test]
    fn json_roundtrip_and_tolerance() {
        let s = Support { first_run: Some(T0), next_after: T0 + 5, never: true };
        assert_eq!(Support::from_json(&s.to_json()), s);
        for bad in ["", "{", "[]", "{\"firstRun\":\"x\",\"never\":3}", "{\"firstRun\":-5}"] {
            let r = Support::from_json(bad);
            assert_eq!((r.first_run, r.next_after, r.never), (None, 0, false), "{bad}");
        }
        // unknown fields are ignored
        assert_eq!(Support::from_json("{\"firstRun\":7,\"x\":1}").first_run, Some(7));
    }
}
