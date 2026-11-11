//! Orchestrator tests with a fake backend and a fake window system.

use super::learned::{FailReason, Learned};
use super::*;
use crate::desktops::backend::{Error, Result as BResult};
use crate::desktops::{DesktopBackend, Guid};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use windows_sys::Win32::Foundation::HWND;

const TRANSIENT: Error = Error::Transient(0x800706BAu32 as i32);
const PERMANENT: Error = Error::Permanent(0x80004005u32 as i32);

fn g(n: u32) -> Guid {
    Guid { d1: n, d2: 0, d3: 0, d4: [0; 8] }
}
fn h(n: usize) -> HWND {
    n as HWND
}

struct Win {
    desktop: Guid,
    state: State,
    candidate: bool,
    can_move: bool,
}

#[derive(Default)]
struct World {
    desktops: RefCell<Vec<Guid>>,
    current: Cell<Guid>,
    wins: RefCell<HashMap<usize, Win>>,
    next: Cell<u32>,
    now: Cell<u64>,
    input: Cell<bool>,
    fail_create: Cell<Option<Error>>,
    fail_move: Cell<Option<Error>>,
    fail_remove: Cell<Option<Error>>,
    fail_can_move: Cell<Option<Error>>,
    move_noop: Cell<bool>,
    creates: Cell<u32>,
    can_move_calls: Cell<u32>,
}

impl World {
    fn new(n: u32) -> Rc<World> {
        let w = World::default();
        *w.desktops.borrow_mut() = (1..=n).map(g).collect();
        w.current.set(g(1));
        w.next.set(n + 1);
        w.now.set(10_000);
        Rc::new(w)
    }
    fn open(&self, id: usize, state: State) -> HWND {
        // New windows appear on the current desktop.
        let cur = self.current.get();
        self.wins.borrow_mut().insert(id, Win { desktop: cur, state, candidate: true, can_move: true });
        h(id)
    }
    fn set_state(&self, id: usize, s: State) {
        self.wins.borrow_mut().get_mut(&id).unwrap().state = s;
    }
    fn tick(&self, ms: u64) {
        self.now.set(self.now.get() + ms);
    }
    fn count(&self) -> usize {
        self.desktops.borrow().len()
    }
    fn desk_of(&self, id: usize) -> Guid {
        self.wins.borrow()[&id].desktop
    }
    fn last(&self) -> Guid {
        *self.desktops.borrow().last().unwrap()
    }
    /// The user deletes a desktop by hand: its windows go to the previous one.
    fn user_delete(&self, d: Guid) {
        let fb = self.desktops.borrow()[0];
        self.do_remove(d, fb);
    }
    fn do_remove(&self, d: Guid, fb: Guid) {
        for w in self.wins.borrow_mut().values_mut() {
            if w.desktop == d {
                w.desktop = fb;
            }
        }
        if self.current.get() == d {
            self.current.set(fb);
        }
        self.desktops.borrow_mut().retain(|x| *x != d);
    }
}

struct FB(Rc<World>);

impl DesktopBackend for FB {
    fn name(&self) -> &str {
        "fake"
    }
    fn desktops(&self) -> BResult<Vec<Guid>> {
        Ok(self.0.desktops.borrow().clone())
    }
    fn current(&self) -> BResult<Guid> {
        Ok(self.0.current.get())
    }
    fn can_move(&self, hwnd: HWND) -> BResult<bool> {
        self.0.can_move_calls.set(self.0.can_move_calls.get() + 1);
        if let Some(e) = self.0.fail_can_move.get() {
            return Err(e);
        }
        Ok(self.0.wins.borrow()[&(hwnd as usize)].can_move)
    }
    fn create(&self) -> BResult<Guid> {
        if let Some(e) = self.0.fail_create.get() {
            return Err(e);
        }
        let d = g(self.0.next.get());
        self.0.next.set(d.d1 + 1);
        self.0.desktops.borrow_mut().push(d);
        self.0.creates.set(self.0.creates.get() + 1);
        Ok(d)
    }
    fn move_window(&self, hwnd: HWND, desk: Guid) -> BResult<()> {
        if let Some(e) = self.0.fail_move.get() {
            return Err(e);
        }
        if !self.0.move_noop.get() {
            self.0.wins.borrow_mut().get_mut(&(hwnd as usize)).unwrap().desktop = desk;
        }
        Ok(())
    }
    fn switch(&self, desk: Guid, _animated: bool) -> BResult<()> {
        self.0.current.set(desk);
        Ok(())
    }
    fn remove(&self, desk: Guid, fallback: Guid) -> BResult<()> {
        if let Some(e) = self.0.fail_remove.get() {
            return Err(e);
        }
        self.0.do_remove(desk, fallback);
        Ok(())
    }
    fn window_desktop(&self, hwnd: HWND) -> Option<Guid> {
        let w = self.0.wins.borrow();
        let win = w.get(&(hwnd as usize))?;
        (win.state != State::Destroyed).then_some(win.desktop)
    }
}

struct FW(Rc<World>);

impl WindowSystem for FW {
    fn state(&self, hwnd: HWND) -> State {
        self.0.wins.borrow().get(&(hwnd as usize)).map_or(State::Destroyed, |w| w.state)
    }
    fn is_candidate(&self, hwnd: HWND) -> bool {
        self.0.wins.borrow().get(&(hwnd as usize)).is_some_and(|w| w.candidate)
    }
    fn process(&self, hwnd: HWND) -> (String, u32) {
        ("app.exe".into(), hwnd as u32 + 1000)
    }
    fn class(&self, _hwnd: HWND) -> String {
        "Cls".into()
    }
    fn app_windows(&self) -> Vec<HWND> {
        let w = self.0.wins.borrow();
        w.iter()
            .filter(|(_, v)| v.candidate && !matches!(v.state, State::Destroyed | State::Hidden))
            .map(|(k, _)| h(*k))
            .collect()
    }
    fn activate(&self, _hwnd: HWND) {}
    fn now(&self) -> u64 {
        self.0.now.get()
    }
    fn is_alive(&self, hwnd: HWND, pid: u32) -> bool {
        self.0.wins.borrow().get(&(hwnd as usize)).is_some_and(|w| w.state != State::Destroyed)
            && pid == hwnd as u32 + 1000
    }
    fn recent_user_input(&self, _ms: u32) -> bool {
        self.0.input.get()
    }
    fn exe_stamp(&self, _pid: u32) -> String {
        "1:1".into()
    }
}

type Log = Rc<RefCell<Vec<FailReason>>>;
struct Rec(Log);
impl Learned for Rec {
    fn blocked(&self, _: &str, _: &str) -> bool {
        false
    }
    fn confirm(&mut self, _: &str, _: &str, _: &str) -> bool {
        false
    }
    fn fail(&mut self, _: &str, _: &str, r: FailReason, _: &str) -> bool {
        self.0.borrow_mut().push(r);
        false
    }
    fn set_backend(&mut self, _: &str) {}
    fn list_blocked(&self) -> Vec<String> {
        Vec::new()
    }
    fn forget(&mut self, _: &str) {}
    fn forget_all(&mut self) {}
}

fn setup(n: u32) -> (Rc<World>, Orchestrator<FB, FW>, Log) {
    let w = World::new(n);
    let mut o = Orchestrator::new(FB(w.clone()), FW(w.clone()), old());
    let log: Log = Rc::default();
    o.set_learned(Box::new(Rec(log.clone())));
    (w, o, log)
}

use Event::LocationChange as Loc;

/// Sends an event after letting any transition lock expire.
fn ev(w: &World, o: &mut Orchestrator<FB, FW>, id: usize, e: Event) -> Option<Schedule> {
    w.tick(1000);
    o.on_event(h(id), e)
}

#[test]
fn reference_scenario_plan_6_4() {
    let (w, mut o, _) = setup(2);
    let (d1, d2) = (g(1), g(2));
    // 1. maximize A -> 3 desktops, user on 3, A on 3
    w.open(1, State::Normal);
    w.set_state(1, State::Maximized);
    let sch = ev(&w, &mut o, 1, Loc).expect("recheck scheduled");
    assert_eq!(sch.delay_ms, 60);
    let d3 = g(3);
    assert_eq!(*w.desktops.borrow(), vec![d1, d2, d3]);
    assert_eq!((w.current.get(), w.desk_of(1)), (d3, d3));
    // 2. B opened from desktop 3
    w.open(2, State::Normal);
    assert_eq!(w.desk_of(2), d3);
    // 3. maximize B -> 4 desktops, user on 4
    w.set_state(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    let d4 = g(4);
    assert_eq!(w.count(), 4);
    assert_eq!((w.current.get(), w.desk_of(2)), (d4, d4));
    // 4. A closed in the background -> 3 desktops, user stays with B, no switch
    w.set_state(1, State::Destroyed);
    ev(&w, &mut o, 1, Event::Destroy);
    assert_eq!(*w.desktops.borrow(), vec![d1, d2, d4]);
    assert_eq!((w.current.get(), w.desk_of(2)), (d4, d4));
    // 5. restore B -> origin (desktop 3) is gone -> desktop 1; 2 desktops; user on 1
    w.set_state(2, State::Normal);
    ev(&w, &mut o, 2, Loc);
    assert_eq!(*w.desktops.borrow(), vec![d1, d2]);
    assert_eq!((w.current.get(), w.desk_of(2)), (d1, d1));
    // 6. maximize B again, open C on its desktop
    w.set_state(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    let d5 = w.last();
    assert_eq!((w.count(), w.current.get(), w.desk_of(2)), (3, d5, d5));
    w.open(3, State::Normal);
    assert_eq!(w.desk_of(3), d5);
    // 7. minimize B -> desktop with C stays, user switched to 1
    w.set_state(2, State::Minimized);
    ev(&w, &mut o, 2, Event::MinimizeStart);
    assert_eq!(w.count(), 3);
    assert_eq!((w.current.get(), w.desk_of(2), w.desk_of(3)), (d1, d1, d5));
    assert!(!o.is_managed(h(2)));
    assert!(o.snapshot().is_empty());
}

#[test]
fn resolve_origin_walks_chain_of_vanished_origins() {
    let (w, mut o, _) = setup(1);
    // A: d1 -> d2; B opened on d2, maximized: d2 -> d3; C opened on d3, maximized: d3 -> d4.
    for id in 1..=3 {
        w.open(id, State::Normal);
        w.set_state(id, State::Maximized);
        ev(&w, &mut o, id, Loc);
    }
    assert_eq!(w.count(), 4);
    let (d2, d3, d4) = (g(2), g(3), g(4));
    assert_eq!(w.desk_of(3), d4);
    // The user deletes the middle desktops by hand.
    w.user_delete(d3);
    w.user_delete(d2);
    // Restore C: origin d3 -> (d3's origin d2) -> d2's origin d1.
    w.set_state(3, State::Normal);
    ev(&w, &mut o, 3, Loc);
    assert_eq!(*w.desktops.borrow(), vec![g(1)]);
    assert_eq!((w.current.get(), w.desk_of(3)), (g(1), g(1)));
}

#[test]
fn cannot_move_creates_nothing_and_rejects() {
    let (w, mut o, log) = setup(2);
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().can_move = false;
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.creates.get(), w.current.get()), (2, 0, g(1)));
    assert_eq!(*log.borrow(), vec![FailReason::CannotMove]);
    // Rejected: not asked again.
    ev(&w, &mut o, 1, Loc);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.can_move_calls.get(), 1);
    // Once no longer maximized, it is forgotten and may be tried again.
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.can_move_calls.get(), 2);
}

#[test]
fn move_failure_rolls_back() {
    let (w, mut o, log) = setup(2);
    w.open(1, State::Maximized);
    w.fail_move.set(Some(PERMANENT));
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.current.get(), w.desk_of(1)), (2, g(1), g(1)));
    assert_eq!(*log.borrow(), vec![FailReason::MoveFailed(PERMANENT.hr())]);
    assert!(!o.is_managed(h(1)));
    assert_eq!(w.creates.get(), 1);
}

#[test]
fn move_not_applied_rolls_back() {
    let (w, mut o, log) = setup(2);
    w.open(1, State::Maximized);
    w.move_noop.set(true);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.current.get(), w.desk_of(1)), (2, g(1), g(1)));
    assert_eq!(*log.borrow(), vec![FailReason::MoveNotApplied]);
}

#[test]
fn transient_errors_leave_no_state_and_do_not_learn() {
    let (w, mut o, log) = setup(2);
    w.open(1, State::Maximized);
    // can_move
    w.fail_can_move.set(Some(TRANSIENT));
    ev(&w, &mut o, 1, Loc);
    w.fail_can_move.set(None);
    // create
    w.fail_create.set(Some(TRANSIENT));
    ev(&w, &mut o, 1, Loc);
    w.fail_create.set(None);
    // move_window (rolled back, desktop count restored)
    w.fail_move.set(Some(TRANSIENT));
    ev(&w, &mut o, 1, Loc);
    w.fail_move.set(None);
    assert_eq!((w.count(), w.current.get(), w.desk_of(1)), (2, g(1), g(1)));
    assert!(log.borrow().is_empty());
    assert!(!o.is_managed(h(1)));
    // Not rejected: the next event succeeds.
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
    assert_eq!(w.count(), 3);
}

#[test]
fn failed_rollback_is_cleaned_up_later() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    w.fail_move.set(Some(TRANSIENT));
    w.fail_remove.set(Some(TRANSIENT));
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.count(), 3); // leaked helper desktop
    w.fail_remove.set(None);
    w.fail_move.set(None);
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc); // any event retries the cleanup first
    assert_eq!(w.count(), 2);
}

#[test]
fn transient_exit_error_keeps_state_and_retries() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Normal);
    w.fail_remove.set(Some(TRANSIENT));
    let sch = ev(&w, &mut o, 1, Loc).expect("retry requested");
    assert_eq!(sch.hwnd, h(1));
    assert!(o.is_managed(h(1)));
    w.fail_remove.set(None);
    ev(&w, &mut o, 1, Event::Recheck);
    assert!(!o.is_managed(h(1)));
    assert_eq!((w.count(), w.current.get()), (2, g(1)));
}

#[test]
fn manually_deleted_desktop_is_dropped_without_reentering() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    let d = g(3);
    w.user_delete(d);
    assert_eq!(w.desk_of(1), g(1));
    ev(&w, &mut o, 1, Loc); // still maximized, now on desktop 1
    assert!(!o.is_managed(h(1)));
    assert_eq!((w.count(), w.creates.get()), (2, 1)); // not re-entered
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.count(), 2);
}

#[test]
fn window_moved_to_another_desktop_is_released() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.wins.borrow_mut().get_mut(&1).unwrap().desktop = g(2); // the user moved it
    ev(&w, &mut o, 1, Loc);
    assert!(!o.is_managed(h(1)));
    assert_eq!(w.count(), 2); // empty helper desktop removed
}

#[test]
fn duplicate_and_rapid_events_enter_once() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    for _ in 0..5 {
        w.tick(10); // inside the 50 ms lock
        assert_eq!(o.on_event(h(1), Loc), None);
        assert_eq!(o.on_event(h(1), Event::Show), None);
    }
    ev(&w, &mut o, 1, Loc);
    ev(&w, &mut o, 1, Event::Show);
    assert_eq!((w.creates.get(), w.count()), (1, 3));
}

#[test]
fn events_inside_lock_are_ignored_then_state_is_reread_once() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Normal);
    w.tick(10);
    assert_eq!(o.on_event(h(1), Loc), None);
    assert!(o.is_managed(h(1))); // ignored inside the lock
    w.tick(100);
    o.on_event(h(1), Event::Recheck); // timer re-reads the state
    assert!(!o.is_managed(h(1)));
    assert_eq!((w.count(), w.current.get()), (2, g(1)));
}

#[test]
fn drag_defers_until_move_size_end() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Normal);
    ev(&w, &mut o, 1, Event::MoveSizeStart);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.creates.get(), 0);
    ev(&w, &mut o, 1, Event::MoveSizeEnd);
    assert_eq!((w.creates.get(), w.count()), (1, 3));
    // Dragging a managed maximized window down: exit deferred until the drop.
    w.tick(2000);
    o.on_event(h(1), Event::MoveSizeStart);
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
    ev(&w, &mut o, 1, Event::MoveSizeEnd);
    assert!(!o.is_managed(h(1)));
    assert_eq!(w.count(), 2);
}

#[test]
fn fullscreen_waits_for_settle_timer() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Fullscreen);
    let s = ev(&w, &mut o, 1, Loc).expect("settle timer");
    assert_eq!((s.hwnd, s.delay_ms), (h(1), 60));
    assert_eq!(w.creates.get(), 0);
    ev(&w, &mut o, 1, Event::Recheck);
    assert_eq!((w.creates.get(), w.count()), (1, 3));
    // A fullscreen that was only transient is not entered.
    let (w, mut o, _) = setup(2);
    w.open(1, State::Fullscreen);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Event::Recheck);
    assert_eq!(w.creates.get(), 0);
}

#[test]
fn disabled_blocks_entry_but_not_exit() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    o.settings.enabled = false;
    w.current.set(g(2));
    w.open(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    assert_eq!(w.creates.get(), 1);
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.count(), 2);
}

#[test]
fn flapping_and_short_lived_feed_learned_list() {
    let (w, mut o, log) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.tick(100);
    w.set_state(1, State::Normal);
    o.on_event(h(1), Event::Recheck);
    assert_eq!(*log.borrow(), vec![FailReason::Flapping]);
    log.borrow_mut().clear();
    // User input exempts Flapping.
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.input.set(true);
    w.tick(100);
    w.set_state(1, State::Normal);
    o.on_event(h(1), Event::Recheck);
    assert!(log.borrow().is_empty());
    // A maximized (not fullscreen) window closed within 1.5 s is not ShortLived.
    w.input.set(false);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.tick(1000);
    w.set_state(1, State::Destroyed);
    o.on_event(h(1), Event::Destroy);
    assert!(log.borrow().is_empty());
    // Fullscreen closed within 1.5 s: ShortLived.
    w.open(1, State::Fullscreen);
    ev(&w, &mut o, 1, Loc);
    ev(&w, &mut o, 1, Event::Recheck);
    w.tick(1000);
    w.set_state(1, State::Destroyed);
    o.on_event(h(1), Event::Destroy);
    assert_eq!(*log.borrow(), vec![FailReason::ShortLived]);
    assert_eq!(w.count(), 2);
}

#[test]
fn others_remaining_keeps_desktop_and_switches_back_only_when_viewing() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.open(2, State::Normal); // on the helper desktop
    let helper = g(3);
    // The user is looking elsewhere: no switch.
    w.current.set(g(2));
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.current.get(), w.desk_of(1), w.desk_of(2)), (3, g(2), g(1), helper));
}

#[test]
fn adopt_removes_empty_desktops_and_readopts_live_windows() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.open(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    let stored = o.snapshot();
    assert_eq!(stored.len(), 2);
    // "Restart": a new orchestrator; window 2 died meanwhile.
    w.set_state(2, State::Destroyed);
    let mut o2 = Orchestrator::new(FB(w.clone()), FW(w.clone()), old());
    o2.adopt(stored);
    assert!(o2.is_managed(h(1)));
    assert!(!o2.is_managed(h(2)));
    assert_eq!(w.count(), 3); // the dead window's desktop is gone
    // And a window that was restored while we were away leaves its space right away.
    let stored = o2.snapshot();
    w.set_state(1, State::Normal);
    let mut o3 = Orchestrator::new(FB(w.clone()), FW(w.clone()), old());
    o3.adopt(stored);
    assert!(!o3.is_managed(h(1)));
    assert_eq!(w.count(), 2);
}

#[test]
fn non_candidate_is_rejected_and_pinned_window_is_skipped() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().candidate = false;
    ev(&w, &mut o, 1, Loc);
    w.open(2, State::Maximized);
    w.wins.borrow_mut().get_mut(&2).unwrap().desktop = Guid::ZERO;
    ev(&w, &mut o, 2, Loc);
    assert_eq!(w.creates.get(), 0);
}

fn old() -> Settings {
    Settings { reuse_empty_desktops: false, ..Settings::default() }
}

fn setup_reuse(n: u32) -> (Rc<World>, Orchestrator<FB, FW>) {
    let w = World::new(n);
    let o = Orchestrator::new(FB(w.clone()), FW(w.clone()), Settings::default());
    (w, o)
}

#[test]
fn empty_desktop_is_reused_without_create() {
    let (w, mut o) = setup_reuse(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.creates.get(), w.count()), (0, 2));
    assert_eq!((w.current.get(), w.desk_of(1)), (g(2), g(2)));
    assert!(o.snapshot()[0].borrowed);
}

#[test]
fn borrowed_desktop_survives_restore_and_close() {
    let (w, mut o) = setup_reuse(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.current.get(), w.desk_of(1)), (2, g(1), g(1)));
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.current.get(), g(2));
    w.set_state(1, State::Destroyed);
    ev(&w, &mut o, 1, Event::Destroy);
    assert_eq!((w.count(), w.current.get()), (2, g(1)));
    assert_eq!(w.creates.get(), 0);
}

#[test]
fn borrowed_not_viewed_stays_and_others_keep_it() {
    let (w, mut o) = setup_reuse(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.current.set(g(1)); // user looks elsewhere
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.current.get(), w.desk_of(1)), (2, g(1), g(1)));
}

#[test]
fn non_empty_and_origin_desktops_are_skipped() {
    let (w, mut o) = setup_reuse(3);
    w.open(9, State::Normal);
    w.wins.borrow_mut().get_mut(&9).unwrap().desktop = g(2); // d2 busy
    w.open(1, State::Maximized); // origin d1 (empty of others but is origin)
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.creates.get(), w.desk_of(1)), (0, g(3)));
    // Now d1 is empty again but is the origin of the next window on d1.
    let (w, mut o) = setup_reuse(2);
    w.open(1, State::Maximized);
    w.current.set(g(1));
    w.wins.borrow_mut().get_mut(&1).unwrap().desktop = g(2);
    w.open(2, State::Maximized); // on d1? current d1
    w.wins.borrow_mut().get_mut(&2).unwrap().desktop = g(1);
    w.wins.borrow_mut().get_mut(&1).unwrap().state = State::Normal;
    w.wins.borrow_mut().get_mut(&1).unwrap().desktop = g(2);
    ev(&w, &mut o, 2, Loc); // d2 busy, d1 is origin -> create
    assert_eq!(w.creates.get(), 1);
}

#[test]
fn desktop_of_another_managed_space_is_never_chosen() {
    let (w, mut o) = setup_reuse(3);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc); // borrows d2, user now on d2
    w.current.set(g(1));
    // Make d2 look empty (window minimised away from it), still managed.
    w.wins.borrow_mut().get_mut(&1).unwrap().state = State::Minimized;
    w.open(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    assert_eq!(w.desk_of(2), g(3)); // d2 skipped, empty d3 borrowed
    w.open(3, State::Maximized);
    w.current.set(g(1));
    w.wins.borrow_mut().get_mut(&3).unwrap().desktop = g(1);
    ev(&w, &mut o, 3, Loc);
    assert_eq!(w.creates.get(), 1);
}

#[test]
fn setting_off_keeps_old_behaviour() {
    let (w, mut o, _) = setup(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.creates.get(), w.count()), (1, 3));
    assert!(!o.snapshot()[0].borrowed);
}

#[test]
fn borrowed_rollback_keeps_desktop() {
    let (w, mut o) = setup_reuse(2);
    w.open(1, State::Maximized);
    w.move_noop.set(true);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.creates.get(), w.desk_of(1)), (2, 0, g(1)));
}

#[test]
fn adopt_never_removes_borrowed() {
    let (w, mut o) = setup_reuse(2);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    let stored = o.snapshot();
    w.set_state(1, State::Destroyed);
    let mut o2 = Orchestrator::new(FB(w.clone()), FW(w.clone()), Settings::default());
    o2.adopt(stored);
    assert_eq!(w.count(), 2);
    assert!(!o2.is_managed(h(1)));
}

// ---------------------------------------------------------------- learned list

use super::learned::LearnedStore;
use std::rc::Rc as R;

fn setup_store(n: u32) -> (Rc<World>, Orchestrator<FB, FW>, Rc<Cell<u64>>) {
    let w = World::new(n);
    let mut o = Orchestrator::new(FB(w.clone()), FW(w.clone()), old());
    let t = R::new(Cell::new(1_800_000_000u64));
    let t2 = t.clone();
    o.set_learned(Box::new(LearnedStore::with_clock(None, "fake", 30, Box::new(move || t2.get()))));
    (w, o, t)
}

#[test]
fn hard_failure_blocks_process_for_good_and_rejects_window() {
    let (w, mut o, _) = setup_store(2);
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().can_move = false;
    ev(&w, &mut o, 1, Loc);
    assert_eq!(w.can_move_calls.get(), 1);
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
    // A new window of the same process is never entered: no can_move, create or move calls.
    w.wins.borrow_mut().get_mut(&1).unwrap().can_move = true;
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.open(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    assert_eq!((w.can_move_calls.get(), w.creates.get(), w.count()), (1, 0, 2));
    assert!(!o.is_managed(h(1)) && !o.is_managed(h(2)) && o.is_rejected(h(2)));
    // Forgetting lets the next maximize through.
    o.forget_learned(Some("app.exe"));
    ev(&w, &mut o, 2, Loc);
    assert!(o.is_managed(h(2)));
}

#[test]
fn learning_off_records_and_blocks_nothing() {
    let (w, mut o, _) = setup_store(2);
    o.settings.learning = false;
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().can_move = false;
    ev(&w, &mut o, 1, Loc);
    assert!(o.learned_mut().list_blocked().is_empty());
    // Entry made while learning was on is not applied once it is off.
    o.settings.learning = true;
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
    o.settings.learning = false;
    w.wins.borrow_mut().get_mut(&1).unwrap().can_move = true;
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
}

#[test]
fn move_not_applied_blocks_on_second_strike_and_transient_is_not_recorded() {
    let (w, mut o, _) = setup_store(2);
    w.open(1, State::Maximized);
    w.fail_move.set(Some(TRANSIENT));
    ev(&w, &mut o, 1, Loc);
    w.fail_move.set(None);
    w.move_noop.set(true);
    ev(&w, &mut o, 1, Loc);
    assert!(o.learned_mut().list_blocked().is_empty());
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
}

#[test]
fn nothing_is_learned_while_backend_reinitialises() {
    let (w, mut o, _) = setup_store(2);
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().can_move = false;
    o.suspend_learning();
    ev(&w, &mut o, 1, Loc);
    assert!(o.learned_mut().list_blocked().is_empty());
    o.set_backend(FB(w.clone()));
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
}

#[test]
fn flapping_learns_after_three_but_not_after_user_input() {
    let (w, mut o, _) = setup_store(2);
    w.open(1, State::Maximized);
    for round in 0..3 {
        w.input.set(round == 0); // first one was the user's doing
        ev(&w, &mut o, 1, Loc);
        w.tick(100);
        w.set_state(1, State::Normal);
        o.on_event(h(1), Event::Recheck);
        w.set_state(1, State::Maximized);
    }
    assert!(o.learned_mut().list_blocked().is_empty());
    w.input.set(false);
    for _ in 0..2 {
        ev(&w, &mut o, 1, Loc);
        w.tick(100);
        w.set_state(1, State::Normal);
        o.on_event(h(1), Event::Recheck);
        w.set_state(1, State::Maximized);
    }
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
}
