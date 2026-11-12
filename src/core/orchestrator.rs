//! Space orchestrator (plan 6.3, plan 12.1): enters a helper desktop when a window is
//! maximized / fullscreen, returns it to its origin desktop when the state ends, and keeps the
//! global number of desktops tidy ("compaction": no gaps, never below the baseline B).

use super::learned::{FailReason, Learned, NoLearned};
use super::model::{Event, ManagedSpace, Schedule, State, WindowSystem};
use super::settings::{is_system_window, never_learn, LayoutRule, Settings, SYSTEM_HOSTS};
use crate::desktops::backend::Error;
use crate::desktops::{DesktopBackend, Guid};
use std::collections::{HashMap, HashSet};
use windows_sys::Win32::Foundation::HWND;

/// Events for a window are ignored for this long after our own enter/exit.
const LOCK_MS: u64 = 50;
const FLAPPING_MS: u64 = 800;
const SHORT_LIVED_MS: u64 = 1500;
/// "User did it" window for the Flapping exemption (plan 6.7).
const USER_INPUT_MS: u32 = 300;
const MAX_TOMBSTONES: usize = 32;
const MAX_RETRIES: u8 = 5;
const RETRY_MS: u32 = 300;
const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
/// A maximized window without an application view yet (Explorer creates it 7-200 ms after
/// SHOW) is looked at again this often, at most `MAX_VIEW_WAITS` times; never a failure.
const VIEW_WAIT_MS: u32 = 40;
const MAX_VIEW_WAITS: u8 = 25;
/// Learning (plan 6.7) only counts a failure right after the user maximized the window ...
const LEARN_INPUT_MS: u32 = 1000;
/// ... and not while the window is still starting up.
const YOUNG_MS: u64 = 2000;
const MAX_TRACKED: usize = 256;
/// Longest wait for the window of a launched profile application (plan 12.7).
const LAYOUT_TIMEOUT_MS: u64 = 30_000;
/// While profile applications are awaited, windows are also looked for this often ...
const LAYOUT_TICK_MS: u32 = 500;
/// ... and this often while a window waits for its application view.
const LAYOUT_FAST_TICK_MS: u32 = 40;
/// After its first window, a launched app is followed this long: further windows (main window
/// after a splash screen) go to its desktop, and focus / view taken by it are given back.
const LAYOUT_SETTLE_MS: u64 = 10_000;
const MAX_PROFILE_MAXIMIZE: u8 = 3;

/// Pseudo window handles of the host timers of the layout profile (never real windows).
pub fn layout_tick_token() -> HWND {
    (usize::MAX - 1) as HWND
}
pub fn layout_start_token() -> HWND {
    (usize::MAX - 2) as HWND
}

/// The profile being applied (plan 12.7).
struct LayoutRun {
    rules: Vec<LayoutRule>,
    /// Rules waiting for their window: (rule index, deadline in ms of `WindowSystem::now`,
    /// launched by the profile).
    pending: Vec<(usize, u64, bool)>,
    /// Rules whose launched app already has its window placed: (rule index, followed until).
    watch: Vec<(usize, u64)>,
    claimed: Vec<Claim>,
    /// App windows that existed when the run began (never taken as "new windows of an app").
    known: Vec<HWND>,
    /// Desktop the user was on when the run began; the run never leaves them elsewhere.
    return_to: Option<Guid>,
    /// The foreground window when the run began, given back when an app takes the focus.
    user_fg: HWND,
    user_fg_pid: u32,
    /// A window waits for its application view: the timer runs fast meanwhile.
    fast: bool,
}

/// A window the profile placed.
struct Claim {
    hwnd: HWND,
    /// Still to be kept maximized (launched by the profile, maximizable, attempts left).
    maximize: bool,
    tries: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Placed {
    Yes,
    /// No application view yet: try again shortly.
    NotYet,
    No,
}

pub struct Orchestrator<B: DesktopBackend, W: WindowSystem> {
    backend: B,
    ws: W,
    pub settings: Settings,
    learned: Box<dyn Learned>,
    spaces: HashMap<Guid, ManagedSpace>,
    by_window: HashMap<HWND, Guid>,
    /// Maximized windows we decided not to manage (plan 6.1); NOT a dictionary of all windows.
    rejected: HashSet<HWND>,
    /// Per-window transition locks: (hwnd, locked until, ms).
    locks: Vec<(HWND, u64)>,
    dragging: Option<HWND>,
    /// Vanished helper desktop -> its origin (lets ResolveOrigin walk chains of vanished origins).
    tombstones: Vec<(Guid, Guid)>,
    /// Desktops we created but could not roll back yet.
    leaked: Vec<Guid>,
    retries: HashMap<HWND, u8>,
    /// Baseline B (plan 12.1): the number of desktops the user wants to have; 0 = not known yet.
    baseline: usize,
    /// The desktop list as this app last left it; any other list means the user changed it.
    last_list: Vec<Guid>,
    /// A manual list change was absorbed; compaction is still owed (done by the next poll).
    need_compact: bool,
    dirty: bool,
    /// Backend is being re-initialised (Explorer restart): nothing is learned meanwhile.
    reinit: bool,
    /// Desktops of the layout profile that compaction must keep (plan 12.7), counted among
    /// the desktops that are not helper desktops of managed windows; 0 = no profile applied.
    pinned: usize,
    layout: Option<LayoutRun>,
    /// Windows the profile placed: they keep their desktop when maximized.
    profile_windows: HashSet<HWND>,
    /// Windows that already gave the learned list a strike (one per window, plan 6.7).
    struck: HashSet<HWND>,
    /// When a window was shown (young windows never count for learning).
    shown: HashMap<HWND, u64>,
    /// Maximized windows waiting for their application view: attempts so far.
    view_waits: HashMap<HWND, u8>,
    log: Vec<String>,
}

impl<B: DesktopBackend, W: WindowSystem> Orchestrator<B, W> {
    pub fn new(backend: B, ws: W, settings: Settings) -> Self {
        Orchestrator {
            backend,
            ws,
            settings,
            learned: Box::new(NoLearned),
            spaces: HashMap::new(),
            by_window: HashMap::new(),
            rejected: HashSet::new(),
            locks: Vec::new(),
            dragging: None,
            tombstones: Vec::new(),
            leaked: Vec::new(),
            retries: HashMap::new(),
            baseline: 0,
            last_list: Vec::new(),
            need_compact: false,
            dirty: false,
            reinit: false,
            pinned: 0,
            layout: None,
            profile_windows: HashSet::new(),
            struck: HashSet::new(),
            shown: HashMap::new(),
            view_waits: HashMap::new(),
            log: Vec::new(),
        }
    }

    pub fn set_learned(&mut self, l: Box<dyn Learned>) {
        self.learned = l;
        self.learned.set_backend(self.backend.name());
    }
    pub fn learned_mut(&mut self) -> &mut dyn Learned {
        &mut *self.learned
    }
    /// Maximized/fullscreen windows that exist right now are left alone until the user
    /// restores them (startup, or the tray toggle going from off to on).
    pub fn reject_existing(&mut self) {
        for w in self.ws.app_windows() {
            if !self.by_window.contains_key(&w) && matches!(self.ws.state(w), State::Maximized | State::Fullscreen) {
                self.rejected.insert(w);
            }
        }
    }
    /// New settings from the dialog; effective at once. The tray toggle (`enabled`) is kept.
    /// Windows already left alone stay alone until the user restores them.
    pub fn apply_settings(&mut self, mut new: Settings) {
        new.enabled = self.settings.enabled;
        if !new.layout.enabled {
            self.pinned = 0;
            if self.layout.take().is_some() {
                self.ws.hold_foreground(false);
            }
        }
        self.settings = new;
    }
    /// Tray toggle.
    pub fn set_enabled(&mut self, on: bool) {
        if on && !self.settings.enabled {
            self.reject_existing();
        }
        self.settings.enabled = on;
    }
    /// Gives every managed window back to its (resolved) origin, switches there when the user
    /// is viewing a helper desktop, then compacts. Leaves the managed list empty.
    pub fn give_back_all(&mut self) {
        let Some(list) = self.sync_list() else { return };
        let all = self.snapshot();
        let targets: Vec<Guid> = all.iter().map(|sp| self.resolve_origin(sp.origin, &list)).collect();
        for (sp, target) in all.iter().zip(targets) {
            if self.ws.is_alive(sp.hwnd, sp.pid) && self.backend.move_window(sp.hwnd, target).is_ok() {
                self.note(format!("restore {:#x} {} -> {target}", sp.hwnd as usize, sp.process));
            }
            if self.backend.current().ok() == Some(sp.desktop) {
                let _ = self.backend.switch(target, false);
            }
            self.forget(sp, false);
            self.rejected.insert(sp.hwnd);
        }
        self.compact(None);
    }
    /// Tray: forget one process (or all when `None`); rejected windows get another chance.
    pub fn forget_learned(&mut self, process: Option<&str>) {
        match process {
            Some(p) => self.learned.forget(p),
            None => self.learned.forget_all(),
        }
        self.rejected.clear();
    }
    /// Explorer restarted: record nothing until `set_backend` delivers the new backend.
    pub fn suspend_learning(&mut self) {
        self.reinit = true;
    }
    #[allow(dead_code)]
    pub fn backend(&self) -> &B {
        &self.backend
    }
    #[allow(dead_code)]
    pub fn windows(&self) -> &W {
        &self.ws
    }
    /// Replaces the backend after an Explorer restart (the old COM objects are dead).
    pub fn set_backend(&mut self, b: B) {
        self.backend = b;
        self.learned.set_backend(self.backend.name());
        self.reinit = false;
        self.leaked.clear();
    }
    #[allow(dead_code)]
    pub fn is_managed(&self, hwnd: HWND) -> bool {
        self.by_window.contains_key(&hwnd)
    }
    #[cfg(test)]
    pub fn is_rejected(&self, hwnd: HWND) -> bool {
        self.rejected.contains(&hwnd)
    }
    /// Baseline desktop count B (0 until known).
    pub fn baseline(&self) -> usize {
        self.baseline
    }
    /// True once since the managed set changed (host then rewrites `state.json`).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
    pub fn drain_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }
    /// Managed spaces in a stable order.
    pub fn snapshot(&self) -> Vec<ManagedSpace> {
        let mut v: Vec<_> = self.spaces.values().cloned().collect();
        v.sort_by_key(|s| (s.entered_at, s.hwnd as usize));
        v
    }

    fn note(&mut self, s: String) {
        if self.log.len() < 64 {
            self.log.push(s);
        }
    }

    // ------------------------------------------------------------ events

    /// Handles one window event. Returns a timer request for the host, if any.
    pub fn on_event(&mut self, hwnd: HWND, ev: Event) -> Option<Schedule> {
        if hwnd.is_null() {
            return self.on_grace(); // the (window-less) grace timer of a disappeared window
        }
        if hwnd == layout_tick_token() {
            return self.layout_tick();
        }
        if hwnd == layout_start_token() {
            return self.start_layout();
        }
        match ev {
            Event::Destroy => {
                self.profile_windows.remove(&hwnd);
                self.struck.remove(&hwnd);
                self.shown.remove(&hwnd);
                self.view_waits.remove(&hwnd);
            }
            Event::Show => {
                let now = self.ws.now();
                if self.shown.len() >= MAX_TRACKED {
                    self.shown.retain(|_, t| now.saturating_sub(*t) < YOUNG_MS);
                }
                self.shown.insert(hwnd, now);
            }
            _ => {}
        }
        if self.layout.is_some() && !matches!(ev, Event::Destroy | Event::Hide) {
            if let Some(sch) = self.layout_event(hwnd) {
                return sch; // a window of a profile app: placed (or about to be) by the profile
            }
        }
        match ev {
            Event::MoveSizeStart => {
                self.dragging = Some(hwnd);
                return None;
            }
            Event::MoveSizeEnd | Event::Destroy => {
                if self.dragging == Some(hwnd) {
                    self.dragging = None;
                }
            }
            _ => {
                if self.dragging == Some(hwnd) {
                    return None; // deferred to MOVESIZEEND
                }
            }
        }
        if ev != Event::Recheck && self.is_locked(hwnd) {
            return None;
        }
        self.retry_leaked();
        let managed = self.by_window.contains_key(&hwnd);
        let sch = self.evaluate(hwnd, ev);
        if managed || self.by_window.contains_key(&hwnd) || !self.settings.enabled {
            return sch; // managed windows compact on their own enter/exit
        }
        match ev {
            // Cannot tell whether a vanished window was a candidate: check after the grace.
            Event::Destroy | Event::Hide if sch.is_none() => Some(Schedule {
                hwnd: std::ptr::null_mut(),
                delay_ms: self.settings.empty_desktop_grace_ms.max(10),
            }),
            // A window opening on a later desktop can turn an earlier empty one into a gap.
            Event::Show if self.ws.is_candidate(hwnd) => {
                self.compact(None);
                sch
            }
            _ => sch,
        }
    }

    /// The grace timer fired: compact if the desktop is still empty.
    fn on_grace(&mut self) -> Option<Schedule> {
        if self.settings.enabled {
            self.compact(None);
        }
        None
    }

    /// Periodic cheap check (host timer): detects manual desktop-list changes and compacts.
    pub fn poll(&mut self) {
        if self.reinit || self.baseline == 0 {
            return;
        }
        // The desktop registry key also changes when the view is switched: an app of the
        // profile may just have pulled the user to its desktop.
        if let Some(mut run) = self.layout.take() {
            self.layout_guard(&mut run);
            self.layout = Some(run);
        }
        if let Ok(list) = self.backend.desktops() {
            if list != self.last_list {
                self.absorb(&list);
            }
        }
        if self.need_compact && self.settings.enabled {
            self.compact(None);
        }
    }

    fn evaluate(&mut self, hwnd: HWND, ev: Event) -> Option<Schedule> {
        let state = self.ws.state(hwnd);
        if self.by_window.contains_key(&hwnd) {
            let Some(list) = self.reconcile() else {
                return self.retry(hwnd);
            };
            let d = *self.by_window.get(&hwnd)?; // gone: its desktop was deleted by the user
            return match state {
                State::Maximized | State::Fullscreen => {
                    self.check_moved_away(hwnd, d);
                    None
                }
                State::Destroyed => self.exit_space(hwnd, d, true, state, &list),
                _ => self.exit_space(hwnd, d, false, state, &list),
            };
        }
        match state {
            State::Maximized | State::Fullscreen => {
                if self.rejected.contains(&hwnd) {
                    None
                } else {
                    self.try_enter(hwnd, state, ev)
                }
            }
            _ => {
                self.rejected.remove(&hwnd);
                None
            }
        }
    }

    fn try_enter(&mut self, hwnd: HWND, state: State, ev: Event) -> Option<Schedule> {
        let s = &self.settings;
        if !s.enabled
            || (state == State::Maximized && !s.trigger_on_maximize)
            || (state == State::Fullscreen && !s.trigger_on_fullscreen)
        {
            return None;
        }
        if !self.ws.is_candidate(hwnd) || self.profile_windows.contains(&hwnd) {
            self.rejected.insert(hwnd); // profile windows keep the desktop the profile gave them
            return None;
        }
        // Fullscreen is detected geometrically, so let the window settle first;
        // maximize is a discrete state and is acted on immediately.
        if state == State::Fullscreen && ev != Event::Recheck && s.fullscreen_settle_ms > 0 {
            return Some(Schedule { hwnd, delay_ms: s.fullscreen_settle_ms });
        }
        self.enter_space(hwnd, state == State::Fullscreen)
    }

    // ------------------------------------------------------------ enter

    /// Transactional: nothing is left behind when any step fails.
    fn enter_space(&mut self, hwnd: HWND, fullscreen: bool) -> Option<Schedule> {
        let (process, pid) = self.ws.process(hwnd);
        let class = self.ws.class(hwnd);
        if is_system_window(&process, &class) {
            self.rejected.insert(hwnd); // lock screen, touch keyboard, shell: never touched
            return None;
        }
        if self.settings.is_excluded(&process) {
            self.note(format!("skip {:#x} {process}: excluded in settings", hwnd as usize));
            self.rejected.insert(hwnd);
            return None;
        }
        // O(1) memory lookup; the exe stamp is only read for apps already on the list.
        if self.settings.learning && self.learned.blocked(&process, &class) {
            let stamp = self.ws.exe_stamp(pid);
            if self.learned.confirm(&process, &class, &stamp) {
                self.note(format!("skip {:#x} {process} {class}: on learned list", hwnd as usize));
                self.rejected.insert(hwnd);
                return None;
            }
        }
        let can = self.backend.can_move(hwnd);
        if !matches!(can, Err(e) if e.is_no_view()) {
            self.view_waits.remove(&hwnd);
        }
        match can {
            Ok(true) => {}
            Ok(false) => {
                self.reject(hwnd, &process, &class, pid, FailReason::CannotMove);
                return None;
            }
            Err(e) if e.is_transient() => return None,
            // Shown maximized a moment ago: Explorer has not created its view yet.
            Err(e) if e.is_no_view() => return self.wait_view(hwnd),
            Err(e) => {
                self.reject(hwnd, &process, &class, pid, fail_reason(e, FailReason::NoView));
                return None;
            }
        }
        let cur = self.backend.current().ok()?;
        let origin = match self.backend.window_desktop(hwnd) {
            Some(g) if g.is_zero() => {
                self.rejected.insert(hwnd); // pinned to all desktops
                return None;
            }
            Some(g) => g,
            None => cur,
        };
        if self.on_profile_desktop(&process, origin) {
            self.note(format!("skip {:#x} {process}: on its profile desktop", hwnd as usize));
            self.rejected.insert(hwnd);
            return None;
        }
        // Alone on a desktop that is not #1: it already has a desktop to itself.
        if let Some(list) = self.sync_list() {
            if list.first() != Some(&origin) && !self.occupancy(Some(hwnd)).contains(&origin) {
                self.note(format!("skip {:#x}: alone on its desktop", hwnd as usize));
                self.rejected.insert(hwnd);
                return None;
            }
        }
        let viewing = cur == origin;
        let (d, created) = match self.find_empty(origin) {
            Some(d) => (d, false),
            None => (self.do_create().ok()?, true),
        };
        if let Err(e) = self.backend.move_window(hwnd, d) {
            self.rollback(hwnd, d, origin, created);
            if !e.is_transient() {
                self.reject(hwnd, &process, &class, pid, fail_reason(e, FailReason::MoveFailed(e.hr())));
            }
            return None;
        }
        if self.backend.window_desktop(hwnd) != Some(d) {
            self.rollback(hwnd, d, origin, created);
            self.reject(hwnd, &process, &class, pid, FailReason::MoveNotApplied);
            return None;
        }
        if viewing && self.backend.switch(d, self.settings.switch_animation).is_err() {
            self.rollback(hwnd, d, origin, created);
            return None;
        }
        self.ws.activate(hwnd);
        let entered_at = self.ws.now();
        self.note(format!("enter {:#x} {process} {class} {origin} -> {d}", hwnd as usize));
        self.spaces
            .insert(
            d,
            ManagedSpace { desktop: d, hwnd, pid, origin, entered_at, process, class, fullscreen },
        );
        self.by_window.insert(hwnd, d);
        self.dirty = true;
        // The origin may have become an empty gap; the window now sits on `d` (counted once).
        self.compact(None);
        self.lock(hwnd)
    }

    fn reject(&mut self, hwnd: HWND, process: &str, class: &str, pid: u32, reason: FailReason) {
        let counts = self.counts_for_learning(hwnd, process, class);
        let tag = if counts { "" } else { " (not counted)" };
        self.note(format!("reject {:#x} {process} {class} {reason:?}{tag}", hwnd as usize));
        if counts {
            self.record(process, class, pid, reason);
        }
        self.rejected.insert(hwnd);
    }

    /// Plan 6.7: a failed attempt counts for the learned list only when it was a real attempt on
    /// a settled window the user just maximized: a candidate, not cloaked, on the desktop being
    /// viewed, shown more than `YOUNG_MS` ago, no profile run or Explorer restart going on, not a
    /// system window, and at most once per window (one odd window never blocks its app alone).
    fn counts_for_learning(&mut self, hwnd: HWND, process: &str, class: &str) -> bool {
        if !self.settings.learning || self.reinit || self.layout.is_some() || never_learn(process, class) {
            return false;
        }
        let now = self.ws.now();
        let young = self.shown.get(&hwnd).is_some_and(|t| now.saturating_sub(*t) < YOUNG_MS);
        let settled = !young
            && self.ws.is_candidate(hwnd)
            && !self.ws.is_cloaked(hwnd)
            && self.backend.current().ok().is_some_and(|c| self.backend.window_desktop(hwnd) == Some(c))
            && self.ws.recent_user_input(LEARN_INPUT_MS);
        if !settled || self.struck.contains(&hwnd) {
            return false;
        }
        if self.struck.len() >= MAX_TRACKED {
            self.struck.clear();
        }
        self.struck.insert(hwnd)
    }

    /// Waits for the application view of a maximized window (never counted as a failure).
    fn wait_view(&mut self, hwnd: HWND) -> Option<Schedule> {
        let n = self.view_waits.entry(hwnd).or_insert(0);
        *n += 1;
        if *n > MAX_VIEW_WAITS {
            self.view_waits.remove(&hwnd);
            self.note(format!("skip {:#x}: no application view", hwnd as usize));
            self.rejected.insert(hwnd);
            return None;
        }
        if self.view_waits.len() > MAX_TRACKED {
            self.view_waits.clear();
        }
        Some(Schedule { hwnd, delay_ms: VIEW_WAIT_MS })
    }

    /// Feeds the learned list, unless learning is off, the backend is being re-initialised, the
    /// profile is being applied or the window is a system one.
    fn record(&mut self, process: &str, class: &str, pid: u32, reason: FailReason) {
        if !self.settings.learning || self.reinit || self.layout.is_some() || never_learn(process, class) {
            return;
        }
        let stamp = self.ws.exe_stamp(pid);
        if self.learned.fail(process, class, reason, &stamp) {
            self.note(format!("learned: {process} {class} blocked ({})", reason.name()));
            let name = self.ws.app_name(pid);
            self.learned.set_name(process, &name);
        }
    }

    /// The first empty desktop (never #1, not the origin, not a profile desktop, not held by another
    /// managed space), in order.
    /// Empty desktops only exist at the end (plan 12.1); None means "create one".
    fn find_empty(&mut self, origin: Guid) -> Option<Guid> {
        let list = self.sync_list()?;
        if list.len() < 2 {
            return None;
        }
        let occ = self.occupancy(None);
        // Desktops of the layout profile stay theirs even when empty (plan 12.7).
        (1..list.len())
            .find(|&i| {
                let d = list[i];
                d != origin && !self.spaces.contains_key(&d) && !occ.contains(&d) && !self.is_pinned(&list, i)
            })
            .map(|i| list[i])
    }

    /// Undo a failed enter: a desktop we created is removed, a reused one just emptied again.
    fn rollback(&mut self, hwnd: HWND, d: Guid, fallback: Guid, created: bool) {
        if !created {
            let _ = self.backend.move_window(hwnd, fallback);
            return;
        }
        if self.do_remove(d, fallback).is_err() {
            self.leaked.push(d);
        }
    }

    /// Retries removing helper desktops whose rollback failed earlier.
    fn retry_leaked(&mut self) {
        if self.leaked.is_empty() {
            return;
        }
        let (Ok(list), Ok(cur)) = (self.backend.desktops(), self.backend.current()) else {
            return;
        };
        for d in std::mem::take(&mut self.leaked) {
            if list.contains(&d) {
                if self.count_on(d, None) > 0 {
                    continue; // someone is using it now; leave it alone
                }
                if self.do_remove(d, cur).is_err() {
                    self.leaked.push(d);
                }
            }
        }
    }

    // ------------------------------------------------------------ desktop count management

    /// Creates a desktop; the resulting list is our own doing, not a manual change.
    fn do_create(&mut self) -> Result<Guid, Error> {
        let d = self.backend.create()?;
        self.refresh_last();
        Ok(d)
    }

    fn do_remove(&mut self, d: Guid, fallback: Guid) -> Result<(), Error> {
        self.backend.remove(d, fallback)?;
        self.refresh_last();
        Ok(())
    }

    fn refresh_last(&mut self) {
        if let Ok(l) = self.backend.desktops() {
            self.last_list = l;
        }
    }

    /// Reads the desktop list and absorbs manual changes into B first.
    fn sync_list(&mut self) -> Option<Vec<Guid>> {
        let list = self.backend.desktops().ok()?;
        self.absorb(&list);
        Some(list)
    }

    /// Plan 12.1: a list we did not leave behind is the user's doing: a new desktop raises B,
    /// a removed one lowers it (min 1). Helper desktops of managed windows do not count.
    fn absorb(&mut self, list: &[Guid]) {
        if self.baseline == 0 {
            self.baseline = list.len().max(1);
            self.last_list = list.to_vec();
            self.dirty = true;
            return;
        }
        if self.last_list == list {
            return;
        }
        let added = list.iter().filter(|d| !self.last_list.contains(d)).count();
        let removed = self.last_list.iter().filter(|d| !list.contains(d) && !self.spaces.contains_key(d)).count();
        let b = (self.baseline + added).saturating_sub(removed).max(1);
        self.note(format!("desktops changed manually (+{added} -{removed}): baseline {} -> {b}", self.baseline));
        self.baseline = b;
        self.pinned = self.pinned.min(b);
        self.last_list = list.to_vec();
        self.need_compact = true;
        self.dirty = true;
    }

    /// Desktops holding at least one app window (excluding `exclude`), enumerated once, plus the
    /// desktops of live managed windows. Pinned windows and windows whose desktop is unknown
    /// (cloaked / foreign shell windows report none) do not count.
    fn occupancy(&self, exclude: Option<HWND>) -> HashSet<Guid> {
        let mut occ = HashSet::new();
        for w in self.ws.app_windows() {
            if Some(w) == exclude {
                continue;
            }
            match self.backend.window_desktop(w) {
                Some(g) if !g.is_zero() => {
                    occ.insert(g);
                }
                _ => {}
            }
        }
        for sp in self.spaces.values() {
            if Some(sp.hwnd) != exclude && self.ws.is_alive(sp.hwnd, sp.pid) {
                occ.insert(sp.desktop);
            }
        }
        occ
    }

    /// Invariants 1-5 of plan 12.1. `hint` = (desktop, where to go) for the case that the
    /// desktop the user is viewing is removed because a managed window just left it.
    fn compact(&mut self, hint: Option<(Guid, Guid)>) {
        if self.layout.is_some() {
            self.need_compact = true; // paused while the profile is being applied (plan 12.7)
            return;
        }
        self.need_compact = false;
        let Some(mut list) = self.sync_list() else { return };
        let mut cur = self.backend.current().ok();
        let occ = self.occupancy(None);
        // Pass 1: empty desktops followed by an occupied one (#1 is never removed).
        let mut i = 1;
        while i < list.len() {
            if !self.is_pinned(&list, i) && !occ.contains(&list[i]) && list[i + 1..].iter().any(|x| occ.contains(x)) {
                if !self.remove_at(&mut list, i, hint, &mut cur) {
                    break;
                }
            } else {
                i += 1;
            }
        }
        // Pass 2: trailing empty desktops above B.
        while list.len() > self.baseline.max(1) && list.last().is_some_and(|d| !occ.contains(d)) {
            let last = list.len() - 1;
            if !self.remove_at(&mut list, last, hint, &mut cur) {
                break;
            }
        }
        // Pass 3: never fewer than B.
        while list.len() < self.baseline {
            match self.do_create() {
                Ok(d) => {
                    self.note(format!("compact: created {d}"));
                    list.push(d);
                }
                Err(_) => break,
            }
        }
    }

    /// Desktop `i` belongs to the layout profile and must stay even when empty.
    fn is_pinned(&self, list: &[Guid], i: usize) -> bool {
        let helper = |d: &Guid| self.spaces.contains_key(d);
        !helper(&list[i]) && list[..i].iter().filter(|d| !helper(d)).count() < self.pinned
    }

    /// Removes `list[k]` (k >= 1). A viewed desktop goes to the hint's target, else the previous one.
    fn remove_at(&mut self, list: &mut Vec<Guid>, k: usize, hint: Option<(Guid, Guid)>, cur: &mut Option<Guid>) -> bool {
        let d = list[k];
        let target = match hint {
            Some((hd, ht)) if hd == d && *cur == Some(d) && list.contains(&ht) && ht != d => ht,
            _ => list[k - 1],
        };
        if self.do_remove(d, target).is_err() {
            return false;
        }
        if self.spaces.values().any(|s| s.origin == d) || self.tombstones.iter().any(|t| t.1 == d) {
            self.tombstone(d, list[k - 1]);
        }
        self.note(format!("compact: removed {d}"));
        if *cur == Some(d) {
            *cur = Some(target);
        }
        list.remove(k);
        true
    }

    // ------------------------------------------------------------ exit

    fn exit_space(&mut self, hwnd: HWND, d: Guid, closed: bool, state: State, list: &[Guid]) -> Option<Schedule> {
        let sp = self.spaces.get(&d)?.clone();
        let target = self.resolve_origin(sp.origin, list);
        let occ = self.occupancy(Some(hwnd));
        let others = occ.contains(&d);
        // Fast path (one call: moves W, switches the view, deletes) only where compaction
        // would delete the helper desktop anyway: not #1, and above B or followed by a used one.
        let doomed = !others
            && list.first() != Some(&d)
            && (list.len() > self.baseline.max(1)
                || list.iter().skip_while(|x| **x != d).skip(1).any(|x| occ.contains(x)));
        let res = if doomed {
            self.do_remove(d, target)
        } else {
            let moved = if closed { Ok(()) } else { self.backend.move_window(hwnd, target) };
            moved.and_then(|()| match self.backend.current() {
                Ok(c) if c == d && (!others || self.settings.switch_back_when_others_remain) => {
                    self.backend.switch(target, false)
                }
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            })
        };
        if let Err(e) = res {
            if e.is_transient() {
                return self.retry(hwnd);
            }
            self.note(format!("exit {:#x} failed: {e}; releasing", hwnd as usize));
        } else {
            let elapsed = self.ws.now().saturating_sub(sp.entered_at);
            if sp.fullscreen && (closed || state == State::Hidden) && elapsed < SHORT_LIVED_MS {
                self.record(&sp.process, &sp.class, sp.pid, FailReason::ShortLived);
            } else if elapsed < FLAPPING_MS && !self.ws.recent_user_input(USER_INPUT_MS) {
                self.record(&sp.process, &sp.class, sp.pid, FailReason::Flapping);
            }
        }
        self.note(format!("exit {:#x} closed={closed} others={others} -> {target}", hwnd as usize));
        self.forget(&sp, doomed);
        self.compact(Some((d, target)));
        self.lock(hwnd)
    }

    /// The managed window was moved to another desktop by the user: stop managing it.
    fn check_moved_away(&mut self, hwnd: HWND, d: Guid) {
        let Some(cur) = self.backend.window_desktop(hwnd) else { return };
        if cur == d || cur.is_zero() {
            return;
        }
        let Some(sp) = self.spaces.get(&d).cloned() else { return };
        self.note(format!("{:#x} moved away from {d} to {cur}; released", hwnd as usize));
        self.forget(&sp, false);
        self.rejected.insert(hwnd); // do not fight the user
        self.compact(Some((d, cur))); // its helper desktop is an ordinary empty one now
    }

    /// Drops a managed entry; `desktop_gone` records a tombstone for ResolveOrigin.
    fn forget(&mut self, sp: &ManagedSpace, desktop_gone: bool) {
        self.spaces.remove(&sp.desktop);
        self.by_window.remove(&sp.hwnd);
        self.retries.remove(&sp.hwnd);
        if desktop_gone {
            self.tombstone(sp.desktop, sp.origin);
        }
        self.dirty = true;
    }

    fn tombstone(&mut self, d: Guid, origin: Guid) {
        self.tombstones.retain(|t| t.0 != d);
        self.tombstones.push((d, origin));
        if self.tombstones.len() > MAX_TOMBSTONES {
            self.tombstones.remove(0);
        }
    }

    /// Plan 6.3 ResolveOrigin: follow vanished origins down their chain, else the first desktop.
    fn resolve_origin(&self, origin: Guid, list: &[Guid]) -> Guid {
        let first = list.first().copied().unwrap_or(origin);
        let mut o = origin;
        for _ in 0..=(MAX_TOMBSTONES + self.spaces.len()) {
            if list.contains(&o) {
                return o;
            }
            o = self
                .spaces
                .get(&o)
                .map(|s| s.origin)
                .or_else(|| self.tombstones.iter().find(|t| t.0 == o).map(|t| t.1))
                .unwrap_or(first);
        }
        first
    }

    /// Drops managed entries whose desktop no longer exists. `None` on backend error.
    pub fn reconcile(&mut self) -> Option<Vec<Guid>> {
        let list = self.sync_list()?;
        let gone: Vec<Guid> = self.spaces.keys().filter(|d| !list.contains(d)).copied().collect();
        for d in gone {
            if let Some(sp) = self.spaces.get(&d).cloned() {
                self.note(format!("desktop {d} vanished; {:#x} released", sp.hwnd as usize));
                self.forget(&sp, true);
                self.rejected.insert(sp.hwnd);
            }
        }
        Some(list)
    }

    /// Number of real app windows on `d`, optionally ignoring one.
    fn count_on(&self, d: Guid, exclude: Option<HWND>) -> usize {
        self.ws
            .app_windows()
            .into_iter()
            .filter(|&w| Some(w) != exclude && self.backend.window_desktop(w) == Some(d))
            .count()
    }

    // ------------------------------------------------------------ locks and retries

    fn lock(&mut self, hwnd: HWND) -> Option<Schedule> {
        let now = self.ws.now();
        self.locks.retain(|l| l.1 > now && l.0 != hwnd);
        self.locks.push((hwnd, now + LOCK_MS));
        // Re-read the window state once after the lock expires.
        Some(Schedule { hwnd, delay_ms: LOCK_MS as u32 + 10 })
    }

    fn is_locked(&self, hwnd: HWND) -> bool {
        let now = self.ws.now();
        self.locks.iter().any(|l| l.0 == hwnd && l.1 > now)
    }

    fn retry(&mut self, hwnd: HWND) -> Option<Schedule> {
        let n = self.retries.entry(hwnd).or_insert(0);
        *n += 1;
        if *n > MAX_RETRIES {
            self.retries.remove(&hwnd);
            if let Some(d) = self.by_window.remove(&hwnd) {
                self.spaces.remove(&d);
                self.dirty = true;
                self.note(format!("giving up on {:#x}", hwnd as usize));
            }
            return None;
        }
        Some(Schedule { hwnd, delay_ms: RETRY_MS })
    }

    // ------------------------------------------------------------ layout profile (plan 12.7)

    /// The `k`-th (1-based) desktop that is not a helper desktop of a managed window.
    fn layout_target(&mut self, k: u32) -> Option<Guid> {
        let list = self.sync_list()?;
        list.into_iter().filter(|d| !self.spaces.contains_key(d)).nth(k.max(1) as usize - 1)
    }

    /// Host timer request: when to start the profile after launch (None = profile is off).
    pub fn layout_start_schedule(&self) -> Option<Schedule> {
        self.settings
            .layout
            .enabled
            .then(|| Schedule { hwnd: layout_start_token(), delay_ms: self.settings.layout.start_delay_ms.max(10) })
    }

    /// True while launched profile applications are still awaited or settling.
    #[cfg(test)]
    pub fn layout_running(&self) -> bool {
        self.layout.is_some()
    }

    /// Applies the profile now: completes the desktops, moves the windows of running
    /// applications, launches the others and awaits their windows by events (plus a short timer
    /// that only runs meanwhile). Returns the timer request for the host. Never switches the view.
    pub fn start_layout(&mut self) -> Option<Schedule> {
        if self.reinit || self.baseline == 0 {
            self.note("layout: backend not ready".into());
            return None;
        }
        let lay = self.settings.layout.clone();
        let n = lay.count();
        if self.layout.take().is_some() {
            self.ws.hold_foreground(false);
        }
        let return_to = self.backend.current().ok();
        let list = self.sync_list()?;
        self.baseline = self.baseline.max(n);
        self.pinned = n;
        self.dirty = true;
        let mut have = list.iter().filter(|d| !self.spaces.contains_key(d)).count();
        while have < n {
            match self.do_create() {
                Ok(d) => {
                    self.note(format!("layout: created {d}"));
                    have += 1;
                }
                Err(e) => {
                    self.note(format!("layout: cannot create desktop: {e}"));
                    break;
                }
            }
        }
        let wins = self.ws.app_windows();
        let user_fg = self.ws.foreground();
        let user_fg_pid = if user_fg.is_null() { 0 } else { self.ws.process(user_fg).1 };
        let mut run = LayoutRun {
            rules: lay.rules,
            pending: Vec::new(),
            watch: Vec::new(),
            claimed: Vec::new(),
            known: wins.clone(),
            return_to,
            user_fg,
            user_fg_pid,
            fast: false,
        };
        let deadline = self.ws.now() + LAYOUT_TIMEOUT_MS;
        self.ws.hold_foreground(true);
        for i in 0..run.rules.len() {
            let process = run.rules[i].process.clone();
            match self.layout_find(&run, &wins, &process) {
                // A running app: its window is only moved (the user arranged it already).
                Some(w) => {
                    if self.place(&mut run, i, w, false) == Placed::NotYet {
                        run.pending.push((i, deadline, false));
                    }
                }
                None => {
                    let (cmd, args) = (run.rules[i].launch.clone(), run.rules[i].args.clone());
                    if self.ws.launch(&cmd, &args) {
                        self.note(format!("layout: launched {cmd}"));
                        run.pending.push((i, deadline, true));
                    } else {
                        self.note(format!("layout: cannot launch {cmd}"));
                    }
                }
            }
        }
        self.layout_continue(run)
    }

    /// A window of `process` that is not yet claimed and not managed.
    fn layout_find(&self, run: &LayoutRun, wins: &[HWND], process: &str) -> Option<HWND> {
        wins.iter().copied().find(|w| {
            !run.claimed.iter().any(|c| c.hwnd == *w) && !self.by_window.contains_key(w) && self.ws.process(*w).0 == process
        })
    }

    /// Moves `hwnd` to the desktop of rule `i` (no view switch); a window the profile launched is
    /// also maximized, without activation. The window is registered as placed: the ordinary logic
    /// never moves it to a helper desktop (plan 12.7).
    fn place(&mut self, run: &mut LayoutRun, i: usize, hwnd: HWND, launched: bool) -> Placed {
        let (desk, proc) = (run.rules[i].desktop, run.rules[i].process.clone());
        let Some(target) = self.layout_target(desk) else {
            self.note(format!("layout: {proc}: desktop {desk} does not exist"));
            return Placed::No;
        };
        if self.backend.window_desktop(hwnd) != Some(target) {
            let res = match self.backend.can_move(hwnd) {
                Ok(true) => self.backend.move_window(hwnd, target),
                Ok(false) => {
                    self.note(format!("layout: {proc} {:#x}: window cannot be moved", hwnd as usize));
                    return Placed::No;
                }
                Err(e) => Err(e),
            };
            match res {
                Ok(()) => {}
                // Explorer has not created the window's view yet (or is restarting): soon again.
                Err(e) if e.is_no_view() || e.is_transient() => {
                    run.fast = true;
                    return Placed::NotYet;
                }
                Err(e) => {
                    self.note(format!("layout: {proc}: move failed: {e}"));
                    return Placed::No;
                }
            }
            if self.backend.window_desktop(hwnd) != Some(target) {
                self.note(format!("layout: {proc}: move was not applied"));
                return Placed::No;
            }
        }
        self.note(format!("layout: {proc} {:#x} -> desktop {desk}", hwnd as usize));
        run.claimed.push(Claim { hwnd, maximize: launched, tries: 0 });
        self.profile_windows.insert(hwnd);
        self.rejected.insert(hwnd);
        // Focus back to the user first: the maximize then stays inside the app (no view switch).
        self.layout_guard(run);
        if launched {
            self.layout_maximize(run, run.claimed.len() - 1);
        }
        Placed::Yes
    }

    /// Maximizes a launched profile window without activating it (at most a few times: an app may
    /// restore its own saved placement right after showing the window).
    fn layout_maximize(&mut self, run: &mut LayoutRun, c: usize) {
        let hwnd = run.claimed[c].hwnd;
        if !run.claimed[c].maximize
            || matches!(self.ws.state(hwnd), State::Maximized | State::Fullscreen | State::Hidden | State::Destroyed)
        {
            return;
        }
        run.claimed[c].tries += 1;
        run.claimed[c].maximize = run.claimed[c].tries < MAX_PROFILE_MAXIMIZE;
        if self.ws.maximize_inactive(hwnd) {
            self.note(format!("layout: {:#x} maximized", hwnd as usize));
        } else {
            run.claimed[c].maximize = false; // e.g. a splash screen without a maximize box
        }
    }

    /// Undoes what launched apps did to the user (plan 12.7): Windows switches the view to a
    /// window that activates itself on another desktop, and the app takes the focus. A switch
    /// while a profile window is in the foreground is undone (no animation) and the focus goes
    /// back; any other switch is the user's and becomes the place to stay.
    fn layout_guard(&mut self, run: &mut LayoutRun) {
        let Ok(cur) = self.backend.current() else { return };
        let fg = self.ws.foreground();
        // A placed window, or a new window of a profile app (not one the user already had open).
        let by_app = !fg.is_null()
            && (run.claimed.iter().any(|c| c.hwnd == fg)
                || (!run.known.contains(&fg) && {
                    let p = self.ws.process(fg).0;
                    run.rules.iter().any(|r| r.process == p)
                }));
        if let Some(rt) = run.return_to {
            if cur != rt {
                if by_app {
                    if self.backend.desktops().is_ok_and(|l| l.contains(&rt)) && self.backend.switch(rt, false).is_ok() {
                        self.note(format!("layout: view returned to {rt} (an app activated its window)"));
                    }
                } else if !fg.is_null() {
                    run.return_to = Some(cur);
                }
            }
        }
        if by_app && fg != run.user_fg && !run.user_fg.is_null() && self.ws.is_alive(run.user_fg, run.user_fg_pid) {
            self.ws.activate(run.user_fg);
        }
    }

    /// Keeps the run alive (and a timer going) while rules wait for windows or settle; else ends it.
    fn layout_continue(&mut self, mut run: LayoutRun) -> Option<Schedule> {
        let now = self.ws.now();
        run.watch.retain(|w| now < w.1);
        if run.pending.is_empty() && run.watch.is_empty() {
            self.layout = None;
            self.end_layout(run);
            return None;
        }
        let delay_ms = if run.fast { LAYOUT_FAST_TICK_MS } else { LAYOUT_TICK_MS };
        self.layout = Some(run);
        Some(Schedule { hwnd: layout_tick_token(), delay_ms })
    }

    fn end_layout(&mut self, mut run: LayoutRun) {
        self.ws.hold_foreground(false);
        self.layout_guard(&mut run);
        self.note("layout: done".into());
        self.dirty = true;
        if self.settings.enabled {
            self.compact(None);
        }
    }

    /// A window event while the profile runs. `Some(timer)` = the event was the profile's
    /// (a window of a profile app): the ordinary logic must not see it.
    fn layout_event(&mut self, hwnd: HWND) -> Option<Option<Schedule>> {
        let mut run = self.layout.take()?;
        let was_fast = run.fast;
        let consumed = self.layout_claim(&mut run, hwnd);
        if consumed {
            self.layout_guard(&mut run);
        }
        let sch = self.layout_continue(run);
        // Only a newly needed fast timer is passed on: restarting the timer on every event of a
        // busy window could starve it.
        let fast_now = self.layout.as_ref().is_some_and(|r| r.fast && !was_fast);
        consumed.then_some(if fast_now { sch } else { None })
    }

    fn layout_claim(&mut self, run: &mut LayoutRun, hwnd: HWND) -> bool {
        if let Some(c) = run.claimed.iter().position(|c| c.hwnd == hwnd) {
            self.layout_maximize(run, c);
            return true;
        }
        if self.by_window.contains_key(&hwnd) || !self.ws.is_candidate(hwnd) {
            return false;
        }
        let process = self.ws.process(hwnd).0;
        let now = self.ws.now();
        if let Some(pos) = run.pending.iter().position(|p| run.rules[p.0].process == process) {
            let (i, _, launched) = run.pending[pos];
            return match self.place(run, i, hwnd, launched) {
                Placed::Yes => {
                    run.pending.remove(pos);
                    run.watch.push((i, now + LAYOUT_SETTLE_MS));
                    true
                }
                Placed::NotYet => true,
                Placed::No => {
                    run.pending.remove(pos);
                    false
                }
            };
        }
        // Another new window of a just launched app (the main window after a splash screen, a
        // window the app re-creates): it follows the first one instead of getting a helper desktop.
        if run.known.contains(&hwnd) {
            return false;
        }
        match run.watch.iter().find(|w| now < w.1 && run.rules[w.0].process == process) {
            Some(&(i, _)) => self.place(run, i, hwnd, true) != Placed::No,
            None => false,
        }
    }

    /// Timer while the profile runs: late windows, giving up after the limit, the guard.
    fn layout_tick(&mut self) -> Option<Schedule> {
        let mut run = self.layout.take()?;
        run.fast = false;
        let now = self.ws.now();
        let wins = self.ws.app_windows();
        for (i, deadline, launched) in std::mem::take(&mut run.pending) {
            let process = run.rules[i].process.clone();
            match self.layout_find(&run, &wins, &process) {
                Some(w) => match self.place(&mut run, i, w, launched) {
                    Placed::Yes => run.watch.push((i, now + LAYOUT_SETTLE_MS)),
                    Placed::NotYet => run.pending.push((i, deadline, launched)),
                    Placed::No => {}
                },
                None if now >= deadline => self.note(format!("layout: gave up waiting for a window of {process}")),
                None => run.pending.push((i, deadline, launched)),
            }
        }
        for w in wins {
            if !run.known.contains(&w) && !run.claimed.iter().any(|c| c.hwnd == w) && !self.by_window.contains_key(&w) {
                let process = self.ws.process(w).0;
                if let Some(&(i, _)) = run.watch.iter().find(|x| now < x.1 && run.rules[x.0].process == process) {
                    self.place(&mut run, i, w, true);
                }
            }
        }
        self.layout_guard(&mut run);
        self.layout_continue(run)
    }

    /// A maximized window of a profile app on the desktop its rule gives it stays there
    /// (plan 12.7); only while the profile is on and was applied.
    fn on_profile_desktop(&mut self, process: &str, desktop: Guid) -> bool {
        if !self.settings.layout.enabled || self.pinned == 0 {
            return false;
        }
        let Ok(list) = self.backend.desktops() else { return false };
        let ordinary: Vec<Guid> = list.into_iter().filter(|d| !self.spaces.contains_key(d)).collect();
        self.settings
            .layout
            .rules
            .iter()
            .any(|r| r.process == process && ordinary.get(r.desktop.max(1) as usize - 1) == Some(&desktop))
    }

    /// Current desktops and windows as a profile: (number of desktops, one rule per
    /// application and desktop). Reads exe version info, so call it on request only.
    pub fn capture_layout(&self) -> (usize, Vec<LayoutRule>) {
        let Ok(list) = self.backend.desktops() else { return (0, Vec::new()) };
        let ordinary: Vec<Guid> = list.into_iter().filter(|d| !self.spaces.contains_key(d)).collect();
        let mut rules: Vec<LayoutRule> = Vec::new();
        for w in self.ws.app_windows() {
            let Some(rank) = self.backend.window_desktop(w).and_then(|d| ordinary.iter().position(|x| *x == d)) else {
                continue;
            };
            let (process, pid) = self.ws.process(w);
            if process.is_empty() || SYSTEM_HOSTS.contains(&process.as_str()) {
                continue;
            }
            let desktop = rank as u32 + 1;
            if rules.iter().any(|r| r.desktop == desktop && r.process == process) {
                continue;
            }
            let path = self.ws.launch_command(pid);
            if let Some(r) = LayoutRule::new(desktop, &self.ws.app_name(pid), &path, "", &process) {
                rules.push(r);
            }
        }
        rules.sort_by(|a, b| (a.desktop, &a.process).cmp(&(b.desktop, &b.process)));
        (ordinary.len(), rules)
    }

    // ------------------------------------------------------------ startup

    /// Plan 6.5 / 12.1: reconciles `state.json` contents with reality after a restart and
    /// fixes the baseline B. Empty helper desktops are removed, desktops whose window lives are
    /// re-adopted. After a crash (stored spaces exist) B is the stored one, else the current count.
    pub fn adopt(&mut self, stored: Vec<ManagedSpace>, stored_baseline: Option<usize>) {
        let Ok(list) = self.backend.desktops() else { return };
        self.last_list = list.clone();
        self.baseline = list.len().max(1);
        let had_state = !stored.is_empty();
        for mut sp in stored {
            if !list.contains(&sp.desktop) {
                continue;
            }
            if self.count_on(sp.desktop, None) == 0 {
                let target = self.resolve_origin(sp.origin, &list);
                let _ = self.do_remove(sp.desktop, target);
                self.note(format!("startup: removed empty {}", sp.desktop));
                continue;
            }
            if self.ws.is_alive(sp.hwnd, sp.pid) && self.backend.window_desktop(sp.hwnd) == Some(sp.desktop) {
                sp.entered_at = self.ws.now();
                self.by_window.insert(sp.hwnd, sp.desktop);
                self.spaces.insert(sp.desktop, sp);
            }
        }
        self.refresh_last();
        let count = self.last_list.len();
        self.baseline = match stored_baseline {
            Some(b) if had_state => b,
            _ => count.saturating_sub(self.spaces.len()),
        }
        .max(1);
        self.dirty = true;
        // A window that was restored while we were not running leaves its space now.
        let adopted: Vec<HWND> = self.by_window.keys().copied().collect();
        for h in adopted {
            let _ = self.evaluate(h, Event::Recheck);
        }
        self.reject_existing();
        if self.settings.enabled {
            self.compact(None);
        }
    }
}

fn fail_reason(e: Error, otherwise: FailReason) -> FailReason {
    if e.hr() == E_ACCESSDENIED {
        FailReason::AccessDenied
    } else {
        otherwise
    }
}
