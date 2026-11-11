//! Space orchestrator (plan 6.3): enters a helper desktop when a window is maximized /
//! fullscreen and returns it to its origin desktop when the state ends.

use super::learned::{FailReason, Learned, NoLearned};
use super::model::{Event, ManagedSpace, Schedule, State, WindowSystem};
use super::settings::Settings;
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
    dirty: bool,
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
            dirty: false,
            log: Vec::new(),
        }
    }

    #[allow(dead_code)] // wired up with the learned list (later phase)
    pub fn set_learned(&mut self, l: Box<dyn Learned>) {
        self.learned = l;
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
        self.leaked.clear();
    }
    #[allow(dead_code)]
    pub fn is_managed(&self, hwnd: HWND) -> bool {
        self.by_window.contains_key(&hwnd)
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
        self.evaluate(hwnd, ev)
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
        if !self.ws.is_candidate(hwnd) {
            self.rejected.insert(hwnd);
            return None;
        }
        // Fullscreen is detected geometrically, so let the window settle first;
        // maximize is a discrete state and is acted on immediately.
        if state == State::Fullscreen && ev != Event::Recheck && s.fullscreen_settle_ms > 0 {
            return Some(Schedule { hwnd, delay_ms: s.fullscreen_settle_ms });
        }
        self.enter_space(hwnd)
    }

    // ------------------------------------------------------------ enter

    /// Transactional: nothing is left behind when any step fails.
    fn enter_space(&mut self, hwnd: HWND) -> Option<Schedule> {
        let (process, pid) = self.ws.process(hwnd);
        let class = self.ws.class(hwnd);
        if self.learned.blocked(&process, &class) {
            self.rejected.insert(hwnd);
            return None;
        }
        match self.backend.can_move(hwnd) {
            Ok(true) => {}
            Ok(false) => {
                self.reject(hwnd, &process, &class, FailReason::CannotMove);
                return None;
            }
            Err(e) if e.is_transient() => return None,
            Err(e) => {
                self.reject(hwnd, &process, &class, fail_reason(e, FailReason::NoView));
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
        let viewing = cur == origin;
        let d = self.backend.create().ok()?;
        if let Err(e) = self.backend.move_window(hwnd, d) {
            self.rollback(d, origin);
            if !e.is_transient() {
                self.reject(hwnd, &process, &class, fail_reason(e, FailReason::MoveFailed(e.hr())));
            }
            return None;
        }
        if self.backend.window_desktop(hwnd) != Some(d) {
            self.rollback(d, origin);
            self.reject(hwnd, &process, &class, FailReason::MoveNotApplied);
            return None;
        }
        if viewing && self.backend.switch(d, self.settings.switch_animation).is_err() {
            self.rollback(d, origin);
            return None;
        }
        self.ws.activate(hwnd);
        let entered_at = self.ws.now();
        self.note(format!("enter {:#x} {process} {class} {origin} -> {d}", hwnd as usize));
        self.spaces
            .insert(d, ManagedSpace { desktop: d, hwnd, pid, origin, entered_at, process, class });
        self.by_window.insert(hwnd, d);
        self.dirty = true;
        self.lock(hwnd)
    }

    fn reject(&mut self, hwnd: HWND, process: &str, class: &str, reason: FailReason) {
        self.note(format!("reject {:#x} {process} {class} {reason:?}", hwnd as usize));
        self.learned.fail(process, class, reason, true);
        self.rejected.insert(hwnd);
    }

    fn rollback(&mut self, d: Guid, fallback: Guid) {
        if self.backend.remove(d, fallback).is_err() {
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
                if self.backend.remove(d, cur).is_err() {
                    self.leaked.push(d);
                }
            }
        }
    }

    // ------------------------------------------------------------ exit

    fn exit_space(&mut self, hwnd: HWND, d: Guid, closed: bool, state: State, list: &[Guid]) -> Option<Schedule> {
        let sp = self.spaces.get(&d)?.clone();
        let target = self.resolve_origin(sp.origin, list);
        let others = self.count_on(d, Some(hwnd));
        let res = if others == 0 {
            self.backend.remove(d, target) // one call: moves W, switches the view, deletes
        } else {
            let moved = if closed { Ok(()) } else { self.backend.move_window(hwnd, target) };
            moved.and_then(|()| match self.backend.current() {
                Ok(c) if c == d && self.settings.switch_back_when_others_remain => {
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
            if (closed || state == State::Hidden) && elapsed < SHORT_LIVED_MS {
                self.learned.fail(&sp.process, &sp.class, FailReason::ShortLived, false);
            } else if elapsed < FLAPPING_MS && !self.ws.recent_user_input(USER_INPUT_MS) {
                self.learned.fail(&sp.process, &sp.class, FailReason::Flapping, false);
            }
        }
        self.note(format!("exit {:#x} closed={closed} others={others} -> {target}", hwnd as usize));
        self.forget(&sp, others == 0);
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
        let empty = self.count_on(d, Some(hwnd)) == 0;
        let removed = empty && self.backend.remove(d, cur).is_ok();
        self.forget(&sp, removed);
        self.rejected.insert(hwnd); // do not fight the user
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
        let list = self.backend.desktops().ok()?;
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

    // ------------------------------------------------------------ startup

    /// Plan 6.5: reconciles `state.json` contents with reality after a restart.
    /// Empty helper desktops are removed, desktops whose window lives are re-adopted.
    pub fn adopt(&mut self, stored: Vec<ManagedSpace>) {
        let Some(list) = self.reconcile() else { return };
        let mut adopted = Vec::new();
        for mut sp in stored {
            if !list.contains(&sp.desktop) {
                continue;
            }
            if self.count_on(sp.desktop, None) == 0 {
                let target = self.resolve_origin(sp.origin, &list);
                let _ = self.backend.remove(sp.desktop, target);
                self.note(format!("startup: removed empty {}", sp.desktop));
                continue;
            }
            if self.ws.is_alive(sp.hwnd, sp.pid) && self.backend.window_desktop(sp.hwnd) == Some(sp.desktop) {
                sp.entered_at = self.ws.now();
                adopted.push(sp.hwnd);
                self.by_window.insert(sp.hwnd, sp.desktop);
                self.spaces.insert(sp.desktop, sp);
            }
        }
        self.dirty = true;
        // A window that was restored while we were not running leaves its space now.
        for h in adopted {
            let _ = self.evaluate(h, Event::Recheck);
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
