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
    /// Process name per window id (default `app.exe`).
    procs: RefCell<HashMap<usize, String>>,
    /// Commands passed to `WindowSystem::launch`.
    launched: RefCell<Vec<String>>,
    launch_fails: Cell<bool>,
    /// Foreground window id (0 = none).
    fg: Cell<usize>,
    /// Windows `activate` was called for.
    activated: RefCell<Vec<usize>>,
    /// Windows `maximize_inactive` was called for.
    maximized: RefCell<Vec<usize>>,
    /// Windows `stop_flash` was called for (in order).
    unflashed: RefCell<Vec<usize>>,
    /// Windows without a maximize box (splash screens).
    no_max_box: RefCell<Vec<usize>>,
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
        let p = self.0.procs.borrow().get(&(hwnd as usize)).cloned().unwrap_or_else(|| "app.exe".into());
        (p, hwnd as u32 + 1000)
    }
    fn exe_path(&self, pid: u32) -> String {
        let id = pid as usize - 1000;
        format!(r"C:\apps\{}", self.0.procs.borrow().get(&id).cloned().unwrap_or_else(|| "app.exe".into()))
    }
    fn app_name(&self, pid: u32) -> String {
        format!("Name {}", pid - 1000)
    }
    fn launch(&self, command: &str, _args: &str) -> bool {
        self.0.launched.borrow_mut().push(command.to_string());
        !self.0.launch_fails.get()
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
    fn activate(&self, hwnd: HWND) {
        self.0.activated.borrow_mut().push(hwnd as usize);
        self.0.fg.set(hwnd as usize);
    }
    fn foreground(&self) -> HWND {
        h(self.0.fg.get())
    }
    fn stop_flash(&self, hwnd: HWND) {
        self.0.unflashed.borrow_mut().push(hwnd as usize);
    }
    fn maximize_inactive(&self, hwnd: HWND) -> bool {
        let id = hwnd as usize;
        if self.0.no_max_box.borrow().contains(&id) {
            return false;
        }
        self.0.maximized.borrow_mut().push(id);
        if let Some(w) = self.0.wins.borrow_mut().get_mut(&id) {
            w.state = State::Maximized;
        }
        true
    }
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

/// One foreign window on every desktop: no empty desktop exists, so entering creates one.
fn fill(w: &World) {
    let n = w.count();
    for i in 1..=n {
        let hw = w.open(900 + i, State::Normal);
        w.wins.borrow_mut().get_mut(&(900 + i)).unwrap().desktop = g(i as u32);
        let _ = hw;
    }
}

fn setup(n: u32) -> (Rc<World>, Orchestrator<FB, FW>, Log) {
    let w = World::new(n);
    fill(&w);
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
    w.input.set(true); // the user maximized it
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
    w.input.set(true);
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
    w.input.set(true);
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
    o2.adopt(stored, None);
    assert!(o2.is_managed(h(1)));
    assert!(!o2.is_managed(h(2)));
    assert_eq!(w.count(), 3); // the dead window's desktop is gone
    // And a window that was restored while we were away leaves its space right away.
    let stored = o2.snapshot();
    w.set_state(1, State::Normal);
    let mut o3 = Orchestrator::new(FB(w.clone()), FW(w.clone()), old());
    o3.adopt(stored, None);
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
    Settings::default()
}

type Orch = Orchestrator<FB, FW>;

/// A fresh orchestrator "started" on the current world (baseline = desktop count).
fn started(w: &Rc<World>) -> Orch {
    let mut o = Orchestrator::new(FB(w.clone()), FW(w.clone()), Settings::default());
    o.adopt(Vec::new(), None);
    o
}

/// Places window `id` (a normal, visible one) on desktop `d`.
fn put(w: &World, id: usize, d: u32) -> HWND {
    let hw = w.open(id, State::Normal);
    w.wins.borrow_mut().get_mut(&id).unwrap().desktop = g(d);
    hw
}

fn list(w: &World) -> Vec<u32> {
    w.desktops.borrow().iter().map(|d| d.d1).collect()
}

/// A window disappears: Destroy event, then the grace timer.
fn vanish(w: &World, o: &mut Orch, id: usize) {
    w.set_state(id, State::Destroyed);
    let sch = ev(w, o, id, Event::Destroy).expect("grace timer requested");
    assert!(sch.hwnd.is_null());
    assert_eq!(sch.delay_ms, 300);
    w.tick(300);
    o.on_event(sch.hwnd, Event::Recheck);
}

#[test]
fn worked_example_plan_12_1() {
    // B = 3: 1 windows | 2 windows | 3 empty
    let w = World::new(3);
    put(&w, 10, 1);
    put(&w, 20, 2);
    put(&w, 21, 2);
    let mut o = started(&w);
    assert_eq!((o.baseline(), list(&w)), (3, vec![1, 2, 3]));
    // maximize A (from desktop 2) -> first empty desktop is 3
    w.current.set(g(2));
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().desktop = g(2);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((list(&w), w.desk_of(1).d1, w.current.get().d1), (vec![1, 2, 3], 3, 3));
    // maximize B (from desktop 2) -> a new desktop 4
    w.current.set(g(2));
    w.open(2, State::Maximized);
    w.wins.borrow_mut().get_mut(&2).unwrap().desktop = g(2);
    ev(&w, &mut o, 2, Loc);
    assert_eq!((list(&w), w.desk_of(2).d1, w.current.get().d1), (vec![1, 2, 3, 4], 4, 4));
    // all windows on 2 closed (the user is on 4): gap 2 is removed -> 1 windows | A | B
    for id in [20, 21] {
        w.set_state(id, State::Destroyed);
    }
    w.tick(1000);
    o.on_event(h(0), Event::Recheck);
    assert_eq!(list(&w), vec![1, 3, 4]);
    assert_eq!((w.desk_of(1).d1, w.desk_of(2).d1, w.current.get().d1), (3, 4, 4));
    // A and B closed: 1 windows | empty | empty (B = 3 kept, nothing above)
    w.set_state(1, State::Destroyed);
    ev(&w, &mut o, 1, Event::Destroy);
    w.set_state(2, State::Destroyed);
    ev(&w, &mut o, 2, Event::Destroy);
    assert_eq!(w.count(), 3);
    assert_eq!(w.current.get(), w.desktops.borrow()[0]); // switched to the origin
    assert!(o.snapshot().is_empty());
}

#[test]
fn desktop_one_is_never_removed_even_when_empty() {
    let w = World::new(3);
    put(&w, 1, 2);
    put(&w, 2, 3);
    let mut o = started(&w);
    w.tick(1000);
    o.on_event(h(0), Event::Recheck);
    assert_eq!(list(&w), vec![1, 2, 3]); // #1 empty but kept, no gap, count == B
    assert_eq!((w.desk_of(1).d1, w.desk_of(2).d1), (2, 3));
}

#[test]
fn gap_at_baseline_is_removed_and_recreated_at_the_end() {
    let w = World::new(3);
    put(&w, 1, 1);
    put(&w, 2, 3);
    let mut o = started(&w);
    o.on_event(h(0), Event::Recheck);
    // desktop 2 was a gap: removed, a new empty one appended; count stays B
    assert_eq!((w.count(), w.creates.get()), (3, 1));
    assert_eq!(list(&w), vec![1, 3, 4]);
    assert_eq!((w.desk_of(1).d1, w.desk_of(2).d1), (1, 3));
}

#[test]
fn trailing_empties_above_baseline_go_at_or_below_stay() {
    let w = World::new(2);
    put(&w, 1, 1);
    let mut o = started(&w);
    assert_eq!(o.baseline(), 2);
    // Two extra desktops appear without us noticing (as if left over): absorbed as manual -> B = 4
    w.desktops.borrow_mut().push(g(7));
    w.desktops.borrow_mut().push(g(8));
    o.poll();
    assert_eq!((o.baseline(), w.count()), (4, 4));
    // The user deletes them again by hand: B shrinks back and nothing is recreated.
    w.user_delete(g(7));
    w.user_delete(g(8));
    o.poll();
    assert_eq!((o.baseline(), w.count()), (2, 2));
    // Above B: 2 takes the empty desktop 2, 3 needs a new desktop 3 (count 3 > B = 2).
    put(&w, 2, 1);
    w.current.set(g(1));
    w.wins.borrow_mut().get_mut(&2).unwrap().state = State::Maximized;
    ev(&w, &mut o, 2, Loc);
    w.current.set(g(1));
    put(&w, 3, 1);
    w.wins.borrow_mut().get_mut(&3).unwrap().state = State::Maximized;
    ev(&w, &mut o, 3, Loc);
    assert_eq!((w.count(), w.desk_of(2).d1, w.desk_of(3).d1), (3, 2, 3));
    // The last one leaves: its desktop is trailing and above B -> removed.
    w.set_state(3, State::Normal);
    ev(&w, &mut o, 3, Loc);
    assert_eq!(list(&w), vec![1, 2]);
    // The next one leaves: trailing, but count == B -> kept (empty).
    w.set_state(2, State::Normal);
    ev(&w, &mut o, 2, Loc);
    assert_eq!(list(&w), vec![1, 2]);
}

#[test]
fn several_trailing_empties_above_baseline_are_all_removed() {
    let w = World::new(2);
    put(&w, 9, 1);
    let mut o = started(&w);
    for id in 1..=4 {
        w.open(id, State::Maximized);
        w.current.set(g(1));
        w.wins.borrow_mut().get_mut(&id).unwrap().desktop = g(1);
        ev(&w, &mut o, id, Loc);
    }
    assert_eq!(w.count(), 5);
    for id in 1..=4 {
        w.set_state(id, State::Destroyed); // gone without us seeing the events
    }
    o.on_event(h(0), Event::Recheck);
    assert_eq!(list(&w), vec![1, 2]);
}

#[test]
fn viewed_desktop_removal_goes_to_origin_for_managed_exit_else_previous() {
    // Managed exit: origin d2, helper d4 (B = 2, so it is doomed); user views d4.
    let w = World::new(2);
    put(&w, 9, 1);
    let mut o = started(&w);
    w.current.set(g(2));
    w.open(1, State::Maximized);
    w.wins.borrow_mut().get_mut(&1).unwrap().desktop = g(2);
    put(&w, 8, 2); // another window stays on d2
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.count(), w.desk_of(1).d1, w.current.get().d1), (3, 3, 3));
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((list(&w), w.desk_of(1).d1, w.current.get().d1), (vec![1, 2], 2, 2));
    // Non-managed: user views a middle desktop that empties -> previous desktop.
    let w = World::new(3);
    put(&w, 1, 1);
    put(&w, 2, 2);
    put(&w, 3, 3);
    let mut o = started(&w);
    w.current.set(g(2));
    vanish(&w, &mut o, 2);
    assert_eq!((list(&w), w.current.get().d1), (vec![1, 3, 4], 1));
    // The same for the managed window being minimized while its origin is gone: previous desktop.
    assert_eq!(w.desk_of(3).d1, 3);
}

#[test]
fn grace_timer_keeps_desktop_when_window_reappears() {
    let w = World::new(3);
    put(&w, 1, 1);
    put(&w, 2, 2);
    put(&w, 3, 3);
    let mut o = started(&w);
    w.set_state(2, State::Destroyed);
    let sch = ev(&w, &mut o, 2, Event::Destroy).expect("timer");
    assert_eq!((w.count(), w.creates.get()), (3, 0)); // nothing happens before the grace ends
    assert_eq!(list(&w), vec![1, 2, 3]);
    // The app opens its main window on that desktop within the grace.
    put(&w, 4, 2);
    w.tick(300);
    o.on_event(sch.hwnd, Event::Recheck);
    assert_eq!((list(&w), w.creates.get()), (vec![1, 2, 3], 0));
    // Without a replacement the gap goes (and one appears at the end: B = 3).
    vanish(&w, &mut o, 4);
    assert_eq!(list(&w), vec![1, 3, 4]);
}

#[test]
fn window_shown_on_later_desktop_removes_gap_at_once() {
    let w = World::new(3);
    put(&w, 1, 1);
    let mut o = started(&w);
    assert_eq!(list(&w), vec![1, 2, 3]);
    w.current.set(g(3));
    put(&w, 2, 3);
    w.tick(1000);
    o.on_event(h(2), Event::Show); // no timer involved
    assert_eq!(list(&w), vec![1, 3, 4]);
    // A non-candidate window being shown does nothing.
    w.wins.borrow_mut().get_mut(&2).unwrap().candidate = false;
    put(&w, 3, 4);
    w.wins.borrow_mut().get_mut(&3).unwrap().candidate = false;
    let before = w.creates.get();
    o.on_event(h(3), Event::Show);
    assert_eq!(w.creates.get(), before);
}

#[test]
fn manual_add_raises_baseline_manual_remove_lowers_it_not_below_one() {
    let w = World::new(2);
    put(&w, 1, 1);
    put(&w, 2, 2);
    let mut o = started(&w);
    assert_eq!(o.baseline(), 2);
    w.desktops.borrow_mut().push(g(9)); // the user adds a desktop
    o.poll();
    assert_eq!((o.baseline(), w.count()), (3, 3)); // kept (empty, within B)
    w.user_delete(g(9));
    o.poll();
    assert_eq!((o.baseline(), w.count()), (2, 2));
    // Occupied desktop deleted by hand: B-1, its windows moved to #1 by the fake.
    w.user_delete(g(2));
    o.poll();
    assert_eq!((o.baseline(), w.count()), (1, 1));
    w.user_delete(g(1)); // would leave none; the fake allows it, B stays at 1
    o.poll();
    assert_eq!(o.baseline(), 1);
    // Our own changes are never mistaken for manual ones.
    let w = World::new(2);
    let mut o = started(&w);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    o.poll();
    assert_eq!(o.baseline(), 2);
}

#[test]
fn startup_compaction_and_baseline() {
    // 1 | gap | 3(window) | 4(empty): B = 4 at start; the gap is removed and one created.
    let w = World::new(4);
    put(&w, 1, 1);
    put(&w, 2, 3);
    let o = started(&w);
    assert_eq!((o.baseline(), w.count()), (4, 4));
    assert_eq!(list(&w), vec![1, 3, 4, 5]);
    // Restart after a crash: stored B is used, the re-adopted helper is not counted twice.
    let w = World::new(2);
    let mut o1 = started(&w);
    w.open(1, State::Maximized);
    ev(&w, &mut o1, 1, Loc); // takes the empty desktop 2 (B = 2, count stays 2)
    assert_eq!((w.count(), o1.baseline()), (2, 2));
    let (stored, b) = (o1.snapshot(), o1.baseline());
    let mut o2 = Orchestrator::new(FB(w.clone()), FW(w.clone()), Settings::default());
    o2.adopt(stored.clone(), Some(b));
    assert!(o2.is_managed(h(1)));
    assert_eq!((o2.baseline(), w.count()), (2, 2));
    // Without a stored B (old file): count minus re-adopted helpers.
    let mut o3 = Orchestrator::new(FB(w.clone()), FW(w.clone()), Settings::default());
    o3.adopt(stored, None);
    assert_eq!(o3.baseline(), 1);
    // The window died meanwhile: its desktop is removed, compaction recreates up to B.
    w.set_state(1, State::Destroyed);
    let mut o4 = Orchestrator::new(FB(w.clone()), FW(w.clone()), Settings::default());
    o4.adopt(o2.snapshot(), Some(2));
    assert_eq!((o4.baseline(), w.count()), (2, 2));
}

#[test]
fn excluded_and_foreign_windows_count_as_occupied() {
    let w = World::new(3);
    put(&w, 1, 1);
    put(&w, 2, 2); // e.g. a blocked/excluded process: never moved, but occupies desktop 2
    put(&w, 3, 3);
    w.wins.borrow_mut().get_mut(&3).unwrap().state = State::Minimized;
    let mut o = started(&w);
    assert_eq!((list(&w), w.creates.get()), (vec![1, 2, 3], 0));
    // A pinned window does not occupy anything.
    put(&w, 4, 3);
    w.wins.borrow_mut().get_mut(&4).unwrap().desktop = Guid::ZERO;
    w.set_state(3, State::Destroyed);
    w.tick(1000);
    o.on_event(h(0), Event::Recheck);
    assert_eq!(list(&w), vec![1, 2, 3]); // 3 emptied (trailing, count == B): kept; pinned one ignored
    let _ = o;
}

#[test]
fn enter_and_exit_keep_empty_trailing_desktop_without_remove_create_churn() {
    let w = World::new(2);
    put(&w, 9, 1);
    let mut o = started(&w);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.creates.get(), list(&w), w.current.get().d1), (0, vec![1, 2], 2));
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    // d2 is the trailing empty desktop within B: kept, not removed and re-created.
    assert_eq!((w.creates.get(), list(&w), w.current.get().d1, w.desk_of(1).d1), (0, vec![1, 2], 1, 1));
    // Close in the helper: the user is moved to the origin, the empty desktop stays.
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Destroyed);
    ev(&w, &mut o, 1, Event::Destroy);
    assert_eq!((w.creates.get(), list(&w), w.current.get().d1), (0, vec![1, 2], 1));
}

#[test]
fn compaction_never_counts_the_moved_window_twice_or_not_at_all() {
    // The only window of d1 is maximized: d1 becomes empty but is #1, so it stays; the window
    // is counted on its helper exactly once and nothing else is removed.
    let w = World::new(2);
    let mut o = started(&w);
    w.open(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((list(&w), w.desk_of(1).d1), (vec![1, 2], 2));
    // The only window of d2 is maximized: it already has a desktop to itself -> left alone.
    let w = World::new(2);
    put(&w, 9, 1);
    put(&w, 1, 2);
    let mut o = started(&w);
    w.current.set(g(2));
    w.wins.borrow_mut().get_mut(&1).unwrap().state = State::Maximized;
    ev(&w, &mut o, 1, Loc);
    assert_eq!((list(&w), w.desk_of(1).d1, o.is_managed(h(1)), o.is_rejected(h(1))), (vec![1, 2], 2, false, true));
    let _ = o;
}

// ---------------------------------------------------------------- learned list

use super::learned::LearnedStore;
use std::rc::Rc as R;

fn setup_store(n: u32) -> (Rc<World>, Orchestrator<FB, FW>, Rc<Cell<u64>>) {
    let w = World::new(n);
    fill(&w);
    let mut o = Orchestrator::new(FB(w.clone()), FW(w.clone()), old());
    let t = R::new(Cell::new(1_800_000_000u64));
    let t2 = t.clone();
    o.set_learned(Box::new(LearnedStore::with_clock(None, "fake", 30, Box::new(move || t2.get()))));
    (w, o, t)
}

/// The user maximizes window `id` (a new one when it does not exist yet), which cannot move.
fn user_maximizes_unmovable(w: &World, o: &mut Orchestrator<FB, FW>, id: usize) {
    if !w.wins.borrow().contains_key(&id) {
        w.open(id, State::Normal);
    }
    w.wins.borrow_mut().get_mut(&id).unwrap().can_move = false;
    w.set_state(id, State::Normal);
    ev(w, o, id, Loc);
    w.set_state(id, State::Maximized);
    w.input.set(true);
    ev(w, o, id, Loc);
}

#[test]
fn hard_failure_blocks_process_for_good_and_rejects_window() {
    let (w, mut o, _) = setup_store(2);
    // Plan 6.7: one refusal never blocks; three windows refused right after the user maximized
    // them do (the same window counts once however often it is tried).
    for _ in 0..3 {
        user_maximizes_unmovable(&w, &mut o, 1);
    }
    assert!(o.learned_mut().list_blocked().is_empty(), "one window, one strike");
    user_maximizes_unmovable(&w, &mut o, 2);
    assert!(o.learned_mut().list_blocked().is_empty());
    user_maximizes_unmovable(&w, &mut o, 3);
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
    let calls = w.can_move_calls.get();
    // A new window of the same process is never entered: no can_move, create or move calls.
    w.open(4, State::Maximized);
    ev(&w, &mut o, 4, Loc);
    assert_eq!((w.can_move_calls.get(), w.creates.get(), w.count()), (calls, 0, 2));
    assert!(!o.is_managed(h(4)) && o.is_rejected(h(4)));
    // Forgetting lets the next maximize through.
    o.forget_learned(Some("app.exe"));
    ev(&w, &mut o, 4, Loc);
    assert!(o.is_managed(h(4)));
}

#[test]
fn refusals_that_are_not_real_attempts_are_not_counted() {
    let (w, mut o, _) = setup_store(2);
    // Opened maximized by the app itself (no user input), many windows: nothing is learned.
    for id in 1..=5 {
        w.open(id, State::Maximized);
        w.wins.borrow_mut().get_mut(&id).unwrap().can_move = false;
        ev(&w, &mut o, id, Event::Show);
    }
    assert!(o.learned_mut().list_blocked().is_empty());
    // With user input, but the window was just shown (still starting up).
    w.input.set(true);
    for id in 6..=8 {
        w.open(id, State::Maximized);
        w.wins.borrow_mut().get_mut(&id).unwrap().can_move = false;
        o.on_event(h(id), Event::Show);
        w.tick(100);
        o.on_event(h(id), Loc);
    }
    assert!(o.learned_mut().list_blocked().is_empty());
    // On another desktop (not the one being viewed).
    for id in 9..=11 {
        w.open(id, State::Normal);
        w.wins.borrow_mut().get_mut(&id).unwrap().desktop = g(2);
        w.wins.borrow_mut().get_mut(&id).unwrap().can_move = false;
        w.set_state(id, State::Maximized);
        ev(&w, &mut o, id, Loc);
    }
    assert!(o.learned_mut().list_blocked().is_empty());
    // Lock screen / touch keyboard / shell windows are never even asked.
    let calls = w.can_move_calls.get();
    for (id, p) in [(20, "lockapp.exe"), (21, "textinputhost.exe")] {
        w.open(id, State::Maximized);
        w.procs.borrow_mut().insert(id, p.to_string());
        ev(&w, &mut o, id, Loc);
        assert!(o.is_rejected(h(id)));
    }
    assert_eq!(w.can_move_calls.get(), calls);
}

#[test]
fn window_without_a_view_yet_is_waited_for_not_learned() {
    let (w, mut o, _) = setup_store(2);
    w.input.set(true);
    w.open(1, State::Maximized);
    // Explorer creates the view of a new window only after SHOW.
    w.fail_can_move.set(Some(Error::Permanent(crate::desktops::backend::TYPE_E_ELEMENTNOTFOUND)));
    let sch = ev(&w, &mut o, 1, Event::Show).expect("looked at again soon");
    assert_eq!((sch.hwnd, sch.delay_ms), (h(1), 40));
    assert!(!o.is_rejected(h(1)) && w.count() == 2);
    w.fail_can_move.set(None);
    w.tick(40);
    o.on_event(h(1), Event::Recheck);
    assert!(o.is_managed(h(1)), "entered once the view exists");
    assert!(o.learned_mut().list_blocked().is_empty());
    // A window that never gets a view is left alone after a while, without a strike.
    w.open(2, State::Maximized);
    w.fail_can_move.set(Some(Error::Permanent(crate::desktops::backend::TYPE_E_ELEMENTNOTFOUND)));
    let mut n = 0;
    let mut next = ev(&w, &mut o, 2, Loc);
    while next.is_some() && n < 100 {
        n += 1;
        w.tick(40);
        next = o.on_event(h(2), Event::Recheck);
    }
    assert!(n < 100 && o.is_rejected(h(2)));
    assert!(o.learned_mut().list_blocked().is_empty());
}

#[test]
fn learning_off_records_and_blocks_nothing() {
    let (w, mut o, _) = setup_store(2);
    o.settings.learning = false;
    for id in 1..=3 {
        user_maximizes_unmovable(&w, &mut o, id);
    }
    assert!(o.learned_mut().list_blocked().is_empty());
    // Entry made while learning was on is not applied once it is off.
    o.settings.learning = true;
    for id in 4..=6 {
        user_maximizes_unmovable(&w, &mut o, id);
    }
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
fn move_not_applied_blocks_on_second_window_and_transient_is_not_recorded() {
    let (w, mut o, _) = setup_store(2);
    w.input.set(true);
    w.open(1, State::Maximized);
    w.fail_move.set(Some(TRANSIENT));
    ev(&w, &mut o, 1, Loc);
    w.fail_move.set(None);
    w.move_noop.set(true);
    ev(&w, &mut o, 1, Loc);
    assert!(o.learned_mut().list_blocked().is_empty());
    // The same window again: still one strike.
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.learned_mut().list_blocked().is_empty());
    // A second window of the app fails the same way: blocked.
    w.open(2, State::Normal);
    ev(&w, &mut o, 2, Loc);
    w.set_state(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    assert_eq!(o.learned_mut().list_blocked(), ["app.exe"]);
}

#[test]
fn nothing_is_learned_while_backend_reinitialises() {
    let (w, mut o, _) = setup_store(2);
    o.suspend_learning();
    for id in 1..=3 {
        user_maximizes_unmovable(&w, &mut o, id);
    }
    assert!(o.learned_mut().list_blocked().is_empty());
    o.set_backend(FB(w.clone()));
    for id in 4..=6 {
        user_maximizes_unmovable(&w, &mut o, id);
    }
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

#[test]
fn windows_maximized_at_startup_are_ignored_until_restored() {
    let w = World::new(3);
    put(&w, 9, 1);
    put(&w, 1, 1);
    w.set_state(1, State::Maximized);
    let mut o = started(&w);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.creates.get(), o.is_managed(h(1)), o.is_rejected(h(1))), (0, false, true));
    assert_eq!(w.desk_of(1).d1, 1);
    // The user restores and maximizes again: handled normally (first empty desktop, not #1).
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
    assert_eq!(w.desk_of(1).d1, 2);
    // Tray toggle off -> maximize -> on: the window is left alone.
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    o.set_enabled(false);
    put(&w, 2, 1);
    w.set_state(2, State::Maximized);
    o.set_enabled(true);
    ev(&w, &mut o, 2, Loc);
    assert!(!o.is_managed(h(2)) && o.is_rejected(h(2)));
}

#[test]
fn window_alone_on_its_desktop_is_not_moved_but_alone_on_desktop_one_is() {
    let w = World::new(3);
    put(&w, 1, 2);
    let mut o = started(&w);
    w.set_state(1, State::Maximized);
    w.current.set(g(2));
    ev(&w, &mut o, 1, Loc);
    assert!(!o.is_managed(h(1)) && o.is_rejected(h(1)));
    assert_eq!((list(&w), w.creates.get()), (vec![1, 2, 3], 0));
    // Not a learned-list strike either.
    let (w, mut o, log) = setup(3);
    w.wins.borrow_mut().remove(&902);
    put(&w, 1, 2);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(log.borrow().is_empty() && !o.is_managed(h(1)));
    // Alone on #1: entered.
    let w = World::new(3);
    put(&w, 1, 1);
    let mut o = started(&w);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
}

#[test]
fn desktop_one_is_never_a_helper_even_when_empty() {
    let w = World::new(2);
    put(&w, 9, 2);
    put(&w, 1, 2);
    let mut o = started(&w);
    w.current.set(g(2));
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert_eq!((w.desk_of(1).d1, w.creates.get()), (3, 1)); // d1 is empty but skipped
}

#[test]
fn give_back_all_restores_windows_switches_and_compacts() {
    let w = World::new(2);
    put(&w, 9, 1);
    put(&w, 8, 2);
    let mut o = started(&w);
    w.current.set(g(2));
    for id in [1, 2] {
        put(&w, id, 2);
        w.set_state(id, State::Maximized);
        ev(&w, &mut o, id, Loc);
    }
    assert_eq!(w.count(), 4);
    o.give_back_all();
    assert_eq!((w.desk_of(1).d1, w.desk_of(2).d1, w.current.get().d1), (2, 2, 2));
    assert_eq!(list(&w), vec![1, 2]);
    assert!(o.snapshot().is_empty() && o.take_dirty());
    // Origin vanished: falls back to the previous desktop.
    let w = World::new(3);
    put(&w, 9, 1);
    put(&w, 7, 3);
    put(&w, 8, 2);
    let mut o = started(&w);
    put(&w, 1, 2);
    w.current.set(g(2));
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
    w.set_state(8, State::Destroyed);
    w.user_delete(g(2)); // the user removes the origin desktop; the fake sends windows to #1
    w.wins.borrow_mut().get_mut(&1).unwrap().desktop = w.last();
    o.give_back_all();
    assert_eq!(w.desk_of(1).d1, 1);
    assert_eq!(w.current.get().d1, 1);
}

#[test]
fn settings_exclusion_and_mode_take_effect_without_restart() {
    let w = World::new(3);
    put(&w, 9, 1);
    let mut o = started(&w);
    // Excluded process: its maximized window stays put.
    let mut s = Settings::default();
    s.exclude("App.exe");
    o.apply_settings(s);
    put(&w, 1, 1);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(!o.is_managed(h(1)) && o.is_rejected(h(1)));
    assert_eq!(w.desk_of(1).d1, 1);
    // Exclusion removed: the next maximize (after a restore) is handled.
    o.apply_settings(Settings::default());
    w.set_state(1, State::Normal);
    ev(&w, &mut o, 1, Loc);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
    assert_eq!(w.desk_of(1).d1, 2);
    // Fullscreen-only: a maximized window is ignored (and not remembered as rejected).
    let mut s = Settings::default();
    s.set_fullscreen_only(true);
    o.apply_settings(s);
    put(&w, 2, 1);
    w.set_state(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    assert!(!o.is_managed(h(2)) && !o.is_rejected(h(2)));
    // ... but fullscreen still moves, and switching back to "all" handles maximize again.
    put(&w, 3, 1);
    w.set_state(3, State::Fullscreen);
    ev(&w, &mut o, 3, Loc);
    ev(&w, &mut o, 3, Event::Recheck);
    assert!(o.is_managed(h(3)));
    o.apply_settings(Settings::default());
    ev(&w, &mut o, 2, Loc);
    assert!(o.is_managed(h(2)));
}

#[test]
fn apply_settings_keeps_the_tray_toggle() {
    let w = World::new(2);
    let mut o = started(&w);
    o.set_enabled(false);
    o.apply_settings(Settings::default());
    assert!(!o.settings.enabled);
}

// ---------------------------------------------------------------- layout profile (plan 12.7)

use super::orchestrator::{layout_start_token, layout_tick_token};
use super::settings::{Layout, LayoutRule};

fn profile(count: u32, rules: &[(u32, &str)]) -> Settings {
    let mut s = Settings::default();
    s.layout = Layout {
        enabled: true,
        desktop_count: count,
        start_delay_ms: 1000,
        rules: rules.iter().map(|(d, p)| LayoutRule::new(*d, "", p, "", p).unwrap()).collect(),
    };
    s
}

fn proc_of(w: &World, id: usize, name: &str) {
    w.procs.borrow_mut().insert(id, name.to_string());
}

fn logged(o: &mut Orch, needle: &str) -> bool {
    o.drain_log().iter().any(|l| l.contains(needle))
}

/// Desktop 2 mail.exe (running), desktop 3 trade.exe (missing): applied on a 1-desktop world.
fn layout_world() -> (Rc<World>, Orch) {
    let w = World::new(1);
    put(&w, 1, 1);
    proc_of(&w, 1, "mail.exe");
    let mut o = started(&w);
    o.apply_settings(profile(3, &[(2, "mail.exe"), (3, "trade.exe")]));
    (w, o)
}

#[test]
fn layout_moves_running_app_and_launches_missing_one() {
    let (w, mut o) = layout_world();
    let sch = o.start_layout().expect("trade.exe is awaited: a timer is requested");
    assert_eq!(sch.hwnd, layout_tick_token());
    assert_eq!(list(&w), vec![1, 2, 3]);
    assert_eq!(w.desk_of(1), g(2), "running mail.exe moved");
    assert_eq!(*w.launched.borrow(), vec!["trade.exe".to_string()], "only the missing app is launched");
    assert_eq!(w.current.get(), g(1), "the user stays where they were");
    assert_eq!(o.baseline(), 3);
    assert!(o.layout_running());
    // The launched app shows its window (on the current desktop) -> moved by the event.
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    assert_eq!(w.desk_of(2), g(3));
    assert_eq!(*w.maximized.borrow(), vec![1, 2], "the running and the launched app are both maximized");
    assert_eq!(w.state(2), State::Maximized);
    assert!(o.layout_running(), "the launched app is followed for a while");
    settle(&w, &mut o);
    assert!(!o.layout_running());
    assert_eq!(w.current.get(), g(1));
    assert_eq!(list(&w), vec![1, 2, 3]);
    assert!(!o.is_managed(h(2)), "registered as placed: no helper desktop");
}

#[test]
fn layout_running_app_is_not_started_again_but_moved_and_maximized() {
    let w = World::new(2);
    put(&w, 1, 1);
    proc_of(&w, 1, "a.exe");
    let mut o = started(&w);
    o.apply_settings(profile(3, &[(2, "a.exe")]));
    assert!(o.start_layout().is_none(), "nothing to wait for");
    assert!(w.launched.borrow().is_empty(), "not started again");
    assert_eq!(w.desk_of(1), g(2));
    assert_eq!(*w.maximized.borrow(), vec![1]);
    assert_eq!(w.state(1), State::Maximized);
    assert_eq!(w.current.get(), g(1), "no view switch");
}

#[test]
fn layout_minimized_running_app_on_another_desktop_is_moved_and_maximized() {
    let w = World::new(2);
    put(&w, 1, 2); // on desktop 2, minimized, the user is on desktop 1
    proc_of(&w, 1, "a.exe");
    w.set_state(1, State::Minimized);
    let mut o = started(&w);
    o.apply_settings(profile(3, &[(3, "a.exe")]));
    assert!(o.start_layout().is_none());
    assert!(w.launched.borrow().is_empty());
    assert_eq!(w.desk_of(1), g(3));
    assert_eq!(*w.maximized.borrow(), vec![1]);
    assert_eq!(w.state(1), State::Maximized);
    assert_eq!(w.current.get(), g(1));
    assert!(w.activated.borrow().is_empty(), "no focus taken");
}

#[test]
fn layout_stops_taskbar_flashing_of_placed_windows_while_watched() {
    let (w, mut o) = layout_world();
    o.start_layout();
    assert_eq!(*w.unflashed.borrow(), vec![1], "right after the move");
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    assert!(w.unflashed.borrow().contains(&2));
    w.unflashed.borrow_mut().clear();
    // Later attempts of the app to activate (a flashing button) are cleared by the timer.
    w.tick(500);
    assert!(o.on_event(layout_tick_token(), Event::Recheck).is_some());
    let mut got = w.unflashed.borrow().clone();
    got.sort();
    assert_eq!(got, vec![1, 2]);
    settle(&w, &mut o);
    assert!(!o.layout_running());
    assert!(w.unflashed.borrow().len() > 2, "and once more when the run ends");
}

/// Lets the profile run settle (timer ticks until it ends).
fn settle(w: &World, o: &mut Orch) {
    for _ in 0..100 {
        w.tick(500);
        if o.on_event(layout_tick_token(), Event::Recheck).is_none() {
            return;
        }
    }
    panic!("the profile run never ended");
}

impl World {
    fn state(&self, id: usize) -> State {
        self.wins.borrow()[&id].state
    }
}

#[test]
fn layout_desktops_are_kept_when_empty_or_in_the_middle() {
    let (w, mut o) = layout_world();
    o.start_layout();
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    // mail.exe closes: desktop 2 is empty and desktop 3 is used, yet it is a profile desktop.
    w.set_state(1, State::Destroyed);
    ev(&w, &mut o, 1, Event::Destroy);
    w.tick(1000);
    o.on_event(std::ptr::null_mut(), Event::Recheck);
    o.poll();
    assert_eq!(list(&w), vec![1, 2, 3]);
    // A helper desktop for an ordinary maximized window still comes after them (and goes again).
    put(&w, 5, 1);
    w.set_state(5, State::Maximized);
    ev(&w, &mut o, 5, Loc);
    assert_eq!(list(&w), vec![1, 2, 3, 4]);
    assert_eq!(w.desk_of(5), g(4));
    w.set_state(5, State::Normal);
    ev(&w, &mut o, 5, Loc);
    assert_eq!(list(&w), vec![1, 2, 3]);
}

#[test]
fn layout_gives_up_after_the_limit_and_logs_it() {
    let (w, mut o) = layout_world();
    o.start_layout();
    o.drain_log();
    w.tick(10_000);
    assert!(o.on_event(layout_tick_token(), Event::Recheck).is_some(), "still waiting");
    assert!(o.layout_running());
    w.tick(25_000);
    assert!(o.on_event(layout_tick_token(), Event::Recheck).is_none());
    assert!(!o.layout_running());
    assert!(logged(&mut o, "gave up waiting for a window of trade.exe"));
    assert_eq!(list(&w), vec![1, 2, 3], "desktops stay: the profile still counts for B");
    // A late timer does nothing.
    assert!(o.on_event(layout_tick_token(), Event::Recheck).is_none());
}

#[test]
fn layout_timer_finds_a_window_that_was_not_yet_a_candidate_at_show() {
    let (w, mut o) = layout_world();
    o.start_layout();
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    w.wins.borrow_mut().get_mut(&2).unwrap().candidate = false;
    ev(&w, &mut o, 2, Event::Show);
    assert_eq!(w.desk_of(2), g(1), "not a candidate yet");
    w.wins.borrow_mut().get_mut(&2).unwrap().candidate = true;
    w.tick(500);
    assert!(o.on_event(layout_tick_token(), Event::Recheck).is_some(), "settling");
    assert_eq!(w.desk_of(2), g(3));
    settle(&w, &mut o);
    assert!(!o.layout_running());
}

/// trade.exe launched by the profile (desktop 3 of 3), the user on desktop 1 in window 50.
fn launched_world() -> (Rc<World>, Orch) {
    let w = World::new(1);
    put(&w, 50, 1);
    proc_of(&w, 50, "editor.exe");
    w.fg.set(50);
    let mut o = started(&w);
    o.apply_settings(profile(3, &[(3, "trade.exe")]));
    assert!(o.start_layout().is_some());
    (w, o)
}

#[test]
fn layout_main_window_after_splash_follows_it_no_extra_desktop() {
    let (w, mut o) = launched_world();
    // Splash screen first (no maximize box), then the main window, shown maximized.
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    w.no_max_box.borrow_mut().push(2);
    ev(&w, &mut o, 2, Event::Show);
    assert_eq!(w.desk_of(2), g(3));
    w.set_state(2, State::Destroyed);
    ev(&w, &mut o, 2, Event::Destroy);
    w.open(3, State::Maximized);
    proc_of(&w, 3, "trade.exe");
    ev(&w, &mut o, 3, Event::Show);
    assert_eq!(w.desk_of(3), g(3), "follows the app to its desktop");
    assert!(!o.is_managed(h(3)));
    assert_eq!(list(&w), vec![1, 2, 3], "no helper desktop, no gap between profile desktops");
    assert_eq!(w.current.get(), g(1));
    settle(&w, &mut o);
    // Later the user restores and maximizes it again: it stays on its desktop (plan 12.7).
    w.set_state(3, State::Normal);
    ev(&w, &mut o, 3, Loc);
    w.set_state(3, State::Maximized);
    ev(&w, &mut o, 3, Loc);
    assert_eq!((w.desk_of(3), list(&w)), (g(3), vec![1, 2, 3]));
}

#[test]
fn layout_window_without_a_view_yet_is_placed_as_soon_as_it_has_one() {
    let (w, mut o) = launched_world();
    w.open(2, State::Maximized);
    proc_of(&w, 2, "trade.exe");
    w.fail_can_move.set(Some(Error::Permanent(crate::desktops::backend::TYPE_E_ELEMENTNOTFOUND)));
    let sch = ev(&w, &mut o, 2, Event::Show).expect("fast timer");
    assert_eq!((sch.hwnd, sch.delay_ms), (layout_tick_token(), 40));
    assert_eq!(w.desk_of(2), g(1));
    assert!(!o.is_managed(h(2)), "the ordinary logic did not take it either");
    assert_eq!(w.count(), 3);
    w.fail_can_move.set(None);
    w.tick(40);
    o.on_event(layout_tick_token(), Event::Recheck);
    assert_eq!(w.desk_of(2), g(3));
    assert_eq!(list(&w), vec![1, 2, 3]);
}

#[test]
fn layout_view_and_focus_taken_by_an_app_are_given_back() {
    let (w, mut o) = launched_world();
    // The app takes the focus on the current desktop, then is moved away.
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    w.fg.set(2);
    ev(&w, &mut o, 2, Event::Show);
    assert_eq!(w.desk_of(2), g(3));
    assert_eq!(w.fg.get(), 50, "focus given back to the window the user worked in");
    // Later the app activates its window again: Windows switches the view to desktop 3.
    w.fg.set(2);
    w.current.set(g(3));
    o.poll(); // the desktop registry key changed
    assert_eq!(w.current.get(), g(1), "view returned");
    assert_eq!(w.fg.get(), 50);
    assert!(logged(&mut o, "view returned"));
    // The user goes to desktop 2 on purpose (own window in front): that is respected.
    put(&w, 60, 2);
    proc_of(&w, 60, "notes.exe");
    w.current.set(g(2));
    w.fg.set(60);
    o.poll();
    settle(&w, &mut o);
    assert_eq!(w.current.get(), g(2));
}

#[test]
fn layout_app_restoring_its_own_placement_is_maximized_again() {
    let (w, mut o) = launched_world();
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    assert_eq!(w.state(2), State::Maximized);
    for _ in 0..5 {
        w.set_state(2, State::Normal); // the app applies its saved (normal) placement
        ev(&w, &mut o, 2, Loc);
    }
    assert_eq!(w.maximized.borrow().len(), 3, "a few times, never a fight");
}

/// The same user actions with the profile off (but rules present) and with default settings
/// end in the same desktops and windows (plan 12.7: profile off = unchanged behaviour).
#[test]
fn profile_off_behaves_like_no_profile() {
    let run = |settings: Settings| {
        let w = World::new(2);
        put(&w, 1, 1);
        proc_of(&w, 1, "trade.exe");
        put(&w, 2, 2);
        proc_of(&w, 2, "mail.exe");
        let mut o = started(&w);
        o.apply_settings(settings);
        assert!(o.layout_start_schedule().is_none());
        let mut trace = Vec::new();
        for (id, st) in [(1, State::Maximized), (2, State::Maximized), (1, State::Normal), (2, State::Normal)] {
            w.set_state(id, st);
            ev(&w, &mut o, id, Loc);
            trace.push((list(&w), w.current.get().d1, w.desk_of(1).d1, w.desk_of(2).d1));
        }
        trace
    };
    let mut off = profile(3, &[(2, "mail.exe"), (1, "trade.exe")]);
    off.layout.enabled = false;
    assert_eq!(run(off), run(Settings::default()));
}

#[test]
fn profile_window_restarted_stays_on_its_desktop_only_while_profile_on() {
    let (w, mut o) = layout_world(); // mail.exe on desktop 2, trade.exe launched for desktop 3
    o.start_layout();
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    settle(&w, &mut o);
    // A second mail.exe window on desktop 2 (opened later, not placed by the profile), maximized.
    put(&w, 7, 2);
    proc_of(&w, 7, "mail.exe");
    w.current.set(g(2));
    w.set_state(7, State::Maximized);
    ev(&w, &mut o, 7, Loc);
    assert_eq!((w.desk_of(7), list(&w)), (g(2), vec![1, 2, 3]), "stays on its profile desktop");
    // Profile off: the ordinary rules of plan 12.1 apply again (own helper desktop).
    let mut s = o.settings.clone();
    s.layout.enabled = false;
    o.apply_settings(s);
    w.set_state(7, State::Normal);
    ev(&w, &mut o, 7, Loc);
    w.set_state(7, State::Maximized);
    ev(&w, &mut o, 7, Loc);
    assert!(o.is_managed(h(7)));
}

#[test]
fn layout_launch_failure_is_logged_and_does_not_wait() {
    let (w, mut o) = layout_world();
    w.launch_fails.set(true);
    assert!(o.start_layout().is_none());
    assert!(!o.layout_running());
    assert!(logged(&mut o, "cannot launch trade.exe"));
    assert_eq!(w.desk_of(1), g(2));
}

#[test]
fn layout_window_opening_maximized_goes_to_its_desktop_not_to_a_helper_one() {
    let (w, mut o) = layout_world();
    o.start_layout();
    w.open(2, State::Maximized);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    assert_eq!(w.desk_of(2), g(3));
    assert!(!o.is_managed(h(2)));
    // Maximizing it later (or again) does not take it away either.
    w.set_state(2, State::Normal);
    ev(&w, &mut o, 2, Loc);
    w.set_state(2, State::Maximized);
    ev(&w, &mut o, 2, Loc);
    assert!(!o.is_managed(h(2)));
    assert_eq!(w.desk_of(2), g(3));
    assert_eq!(list(&w), vec![1, 2, 3]);
}

#[test]
fn layout_never_switches_the_view_away() {
    let w = World::new(2);
    put(&w, 1, 2);
    proc_of(&w, 1, "a.exe");
    w.current.set(g(2));
    let mut o = started(&w);
    o.apply_settings(profile(3, &[(3, "a.exe")]));
    assert!(o.start_layout().is_none(), "running app: nothing to wait for");
    assert_eq!(w.desk_of(1), g(3));
    assert_eq!(w.current.get(), g(2));
    assert!(w.launched.borrow().is_empty());
    assert_eq!(list(&w), vec![1, 2, 3]);
}

#[test]
fn layout_failed_move_is_logged_and_not_retried() {
    let (w, mut o) = layout_world();
    w.fail_move.set(Some(PERMANENT));
    o.start_layout();
    assert!(logged(&mut o, "move failed"));
    assert_eq!(w.desk_of(1), g(1));
    // The rule for the missing app is still awaited; nothing else touches mail.exe.
    assert!(o.layout_running());
}

#[test]
fn layout_with_two_rules_for_one_app_uses_two_windows() {
    let w = World::new(1);
    for id in [1, 2] {
        put(&w, id, 1);
        proc_of(&w, id, "n.exe");
    }
    let mut o = started(&w);
    o.apply_settings(profile(3, &[(2, "n.exe"), (3, "n.exe")]));
    assert!(o.start_layout().is_none());
    let mut d: Vec<u32> = [1, 2].iter().map(|i| w.desk_of(*i).d1).collect();
    d.sort();
    assert_eq!(d, vec![2, 3]);
}

#[test]
fn layout_start_schedule_only_when_enabled() {
    let w = World::new(1);
    let mut o = started(&w);
    assert!(o.layout_start_schedule().is_none());
    o.apply_settings(profile(2, &[]));
    let s = o.layout_start_schedule().unwrap();
    assert_eq!((s.hwnd, s.delay_ms), (layout_start_token(), 1000));
    // The start timer applies the profile: an empty rule list just completes the desktops.
    assert!(o.on_event(layout_start_token(), Event::Recheck).is_none());
    assert_eq!(list(&w), vec![1, 2]);
}

#[test]
fn layout_profile_off_releases_the_pinned_desktops() {
    let (w, mut o) = layout_world();
    o.start_layout();
    w.open(2, State::Normal);
    proc_of(&w, 2, "trade.exe");
    ev(&w, &mut o, 2, Event::Show);
    let mut s = o.settings.clone();
    s.layout.enabled = false;
    o.apply_settings(s);
    w.set_state(1, State::Destroyed);
    ev(&w, &mut o, 1, Event::Destroy);
    w.tick(1000);
    o.on_event(std::ptr::null_mut(), Event::Recheck);
    // The gap (desktop 2) is closed again; B = 3 still holds, so a new one is added at the end.
    assert_eq!(list(&w), vec![1, 3, 4]);
}

#[test]
fn capture_layout_reads_desktops_and_windows() {
    let w = World::new(2);
    for (id, d, p) in [(1, 2, "mail.exe"), (2, 2, "mail.exe"), (3, 1, "explorer.exe"), (4, 2, "calc.exe"), (5, 1, "notepad.exe")] {
        put(&w, id, d);
        proc_of(&w, id, p);
    }
    let o = started(&w);
    let (n, rules) = o.capture_layout();
    assert_eq!(n, 3, "two applications need three desktops");
    let got: Vec<(u32, &str)> = rules.iter().map(|r| (r.desktop, r.process.as_str())).collect();
    // Desktop 1 (notepad) stays the user's; the order gives the numbers.
    assert_eq!(got, vec![(2, "calc.exe"), (3, "mail.exe")]);
    assert_eq!(rules[0].launch, r"C:\apps\calc.exe");
    assert_eq!(rules[0].name, "Name 4");
}

#[test]
fn capture_layout_ignores_helper_desktops() {
    let w = World::new(1);
    put(&w, 1, 1);
    let mut o = started(&w);
    w.set_state(1, State::Maximized);
    ev(&w, &mut o, 1, Loc);
    assert!(o.is_managed(h(1)));
    put(&w, 2, 1);
    proc_of(&w, 2, "calc.exe");
    let (n, rules) = o.capture_layout();
    assert_eq!(n, 1, "the helper desktop is not part of the profile");
    assert!(rules.is_empty(), "desktop 1 is not captured");
}
