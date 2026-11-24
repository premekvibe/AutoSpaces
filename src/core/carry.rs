//! Carrying a window between desktops (plan 12.13): the pure decisions (target of a step, when
//! the view catches up with the target, which visual method is used). The orchestrator applies
//! them; the host (main.rs) owns the shortcut, the keyboard hook and the row of desktop previews.

use super::settings::CarryStyle;

/// Length of an animated desktop switch; steps pressed faster are merged (no replay of each one).
pub const ANIMATION_MS: u64 = 440;

/// Result of one step from desktop `index` (0-based) of `count`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Go to this desktop (0-based).
    To(usize),
    /// Right from the last desktop with "create a new desktop": a new one at the end.
    Create,
    /// Nothing to do (one desktop only, or left from desktop 1 in the "create" mode).
    Stay,
}

/// Where a step to the right (`right`) or left goes. Without `create_new` the desktops go round
/// (right from the last = desktop 1, left from desktop 1 = the last one); with it, right from the
/// last creates a new desktop and left stops at desktop 1.
pub fn step_target(index: usize, count: usize, right: bool, create_new: bool) -> Step {
    let last = count.saturating_sub(1);
    let index = index.min(last);
    match (right, create_new) {
        (true, _) if index < last => Step::To(index + 1),
        (true, true) => Step::Create,
        (false, _) if index > 0 => Step::To(index - 1),
        (false, true) => Step::Stay,
        _ if count <= 1 => Step::Stay,
        (true, false) => Step::To(0),
        (false, false) => Step::To(last),
    }
}

/// What the carry has to do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// An animation runs: look again after this many ms.
    Wait(u32),
    /// Switch the view (with the window) straight to the current target.
    Switch,
    /// The window was dropped and the view is there: finish.
    Finish,
    /// Nothing to do until the next key.
    Idle,
}

/// Steps faster than the animation are merged: while an animation runs (`now < busy_until`) the
/// target only moves; when it ends, one switch goes straight to the latest target. A dropped
/// carry finishes once the view has reached the target and no animation runs.
pub fn next_action(view_at_target: bool, now: u64, busy_until: u64, dropping: bool) -> Next {
    if now < busy_until {
        return Next::Wait((busy_until - now).min(u32::MAX as u64) as u32);
    }
    if !view_at_target {
        return Next::Switch;
    }
    if dropping {
        Next::Finish
    } else {
        Next::Idle
    }
}

/// The method actually used: the preferred one when available, else the best available
/// (`overlay` → `pin` → `moveSwitch` → `switchMove`, the last works everywhere). Never `Auto`.
pub fn choose_style(pref: CarryStyle, overlay: bool, pin: bool, move_switch: bool) -> CarryStyle {
    match pref {
        CarryStyle::Overlay | CarryStyle::Auto if overlay => CarryStyle::Overlay,
        CarryStyle::Pin if pin => CarryStyle::Pin,
        CarryStyle::MoveSwitch if move_switch => CarryStyle::MoveSwitch,
        CarryStyle::SwitchMove => CarryStyle::SwitchMove,
        _ if overlay => CarryStyle::Overlay,
        _ if pin => CarryStyle::Pin,
        _ if move_switch => CarryStyle::MoveSwitch,
        _ => CarryStyle::SwitchMove,
    }
}

/// Win+Ctrl+Shift held alone this long shows the carry's layers as a preview (no change yet).
/// The layers are built (invisible) as soon as the three are down, so showing them is instant.
pub const PREVIEW_MS: u32 = 70;

/// A key as the preview needs it (from Raw Input; nothing is recorded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordKey {
    Win { right: bool },
    Ctrl { right: bool },
    Shift { right: bool },
    /// Left or right arrow: with the three modifiers it is the carry shortcut.
    Arrow,
    /// Anything else: another shortcut (Win+Ctrl+Shift+B, …) or typing.
    Other,
}

/// What the host does after a key or the timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordAction {
    None,
    /// All three modifiers are down: start the timer (`PREVIEW_MS`).
    Arm,
    /// Held long enough: show the preview.
    Show,
    /// A modifier was released: the preview fades away, nothing happens.
    Hide,
    /// Another key: the preview goes at once (the key itself passes untouched).
    HideNow,
}

/// Win+Ctrl+Shift held without another key (plan 12.13): the state of the three modifiers, in
/// any order, left or right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chord {
    win: [bool; 2],
    ctrl: [bool; 2],
    shift: [bool; 2],
    /// Another key came while modifiers were held: no preview until all are released.
    spoiled: bool,
    armed_at: Option<u64>,
    shown: bool,
}

impl Chord {
    pub const NEW: Chord = Chord { win: [false; 2], ctrl: [false; 2], shift: [false; 2], spoiled: false, armed_at: None, shown: false };

    fn all(&self) -> bool {
        self.win.iter().any(|x| *x) && self.ctrl.iter().any(|x| *x) && self.shift.iter().any(|x| *x)
    }

    fn any(&self) -> bool {
        self.win.iter().chain(&self.ctrl).chain(&self.shift).any(|x| *x)
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// Waiting for the delay.
    pub fn armed(&self) -> bool {
        self.armed_at.is_some()
    }

    /// The preview was taken over by a carry (or could not be shown): forget it.
    pub fn reset_preview(&mut self) {
        self.shown = false;
        self.armed_at = None;
    }

    /// A key went down (`down`) or up at `now` (ms).
    pub fn key(&mut self, k: ChordKey, down: bool, now: u64) -> ChordAction {
        let slot = match k {
            ChordKey::Win { right } => Some((&mut self.win, right)),
            ChordKey::Ctrl { right } => Some((&mut self.ctrl, right)),
            ChordKey::Shift { right } => Some((&mut self.shift, right)),
            _ => None,
        };
        if let Some((keys, right)) = slot {
            keys[right as usize] = down;
            if down {
                // Auto-repeat of a held modifier changes nothing.
                if self.all() && !self.spoiled && !self.shown && self.armed_at.is_none() {
                    self.armed_at = Some(now);
                    return ChordAction::Arm;
                }
                return ChordAction::None;
            }
            self.armed_at = None;
            if !self.any() {
                self.spoiled = false;
            }
            if self.shown && !self.all() {
                self.shown = false;
                return ChordAction::Hide;
            }
            return ChordAction::None;
        }
        if !down {
            return ChordAction::None;
        }
        match k {
            // The shortcut itself: the carry starts (from the preview when it is shown).
            ChordKey::Arrow if self.all() => {
                self.armed_at = None;
                ChordAction::None
            }
            _ if self.any() => {
                self.spoiled = true;
                self.armed_at = None;
                if std::mem::replace(&mut self.shown, false) {
                    ChordAction::HideNow
                } else {
                    ChordAction::None
                }
            }
            _ => ChordAction::None,
        }
    }

    /// The timer: show the preview when the three have been held alone for `PREVIEW_MS`.
    pub fn tick(&mut self, now: u64) -> ChordAction {
        match self.armed_at {
            Some(t) if self.all() && !self.spoiled && !self.shown && now >= t + PREVIEW_MS as u64 => {
                self.armed_at = None;
                self.shown = true;
                ChordAction::Show
            }
            _ => ChordAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_both_ways() {
        assert_eq!(step_target(0, 3, true, false), Step::To(1));
        assert_eq!(step_target(1, 3, true, false), Step::To(2));
        assert_eq!(step_target(2, 3, true, false), Step::To(0), "right from the last = desktop 1");
        assert_eq!(step_target(0, 3, false, false), Step::To(2), "left from desktop 1 = the last");
        assert_eq!(step_target(2, 3, false, false), Step::To(1));
    }

    #[test]
    fn create_new_at_the_end_and_stop_at_desktop_one() {
        assert_eq!(step_target(2, 3, true, true), Step::Create);
        assert_eq!(step_target(1, 3, true, true), Step::To(2));
        assert_eq!(step_target(0, 3, false, true), Step::Stay);
        assert_eq!(step_target(1, 3, false, true), Step::To(0));
    }

    #[test]
    fn single_desktop() {
        assert_eq!(step_target(0, 1, true, false), Step::Stay);
        assert_eq!(step_target(0, 1, false, false), Step::Stay);
        assert_eq!(step_target(0, 1, true, true), Step::Create);
        assert_eq!(step_target(0, 1, false, true), Step::Stay);
        // Out-of-range input is clamped, never panics.
        assert_eq!(step_target(9, 2, true, false), Step::To(0));
        assert_eq!(step_target(0, 0, true, false), Step::Stay);
    }

    #[test]
    fn fast_steps_are_merged() {
        // Animation running: only wait.
        assert_eq!(next_action(false, 1000, 1300, false), Next::Wait(300));
        assert_eq!(next_action(false, 1000, 1300, true), Next::Wait(300));
        // Animation over, target moved on meanwhile: one switch straight there.
        assert_eq!(next_action(false, 1300, 1300, false), Next::Switch);
        // At the target, not dropped: idle; dropped: finish.
        assert_eq!(next_action(true, 2000, 1300, false), Next::Idle);
        assert_eq!(next_action(true, 2000, 1300, true), Next::Finish);
        // A drop never finishes during an animation (a pinned window is unpinned after it).
        assert_eq!(next_action(true, 1000, 1300, true), Next::Wait(300));
    }

    #[test]
    fn style_by_availability() {
        use CarryStyle::*;
        assert_eq!(choose_style(Auto, true, true, true), Overlay, "overlay first when available");
        assert_eq!(choose_style(Overlay, true, false, false), Overlay);
        assert_eq!(choose_style(Overlay, false, true, true), Pin, "overlay unavailable: next best");
        assert_eq!(choose_style(Overlay, false, false, false), SwitchMove);
        assert_eq!(choose_style(Pin, true, true, true), Pin, "a chosen method is kept");
        assert_eq!(choose_style(Auto, false, true, true), Pin);
        assert_eq!(choose_style(Auto, false, false, true), MoveSwitch);
        assert_eq!(choose_style(Auto, false, false, false), SwitchMove);
        assert_eq!(choose_style(Pin, false, false, true), MoveSwitch, "unverified pinning: next best");
        assert_eq!(choose_style(Pin, false, false, false), SwitchMove);
        assert_eq!(choose_style(MoveSwitch, true, true, true), MoveSwitch);
        assert_eq!(choose_style(MoveSwitch, false, true, false), Pin, "missing slot: the best available");
        assert_eq!(choose_style(MoveSwitch, true, false, false), Overlay, "missing slot: the best available");
        assert_eq!(choose_style(MoveSwitch, false, false, false), SwitchMove);
        assert_eq!(choose_style(SwitchMove, true, true, true), SwitchMove);
    }

    use ChordAction::*;
    use ChordKey::*;

    fn hold(c: &mut Chord, keys: &[ChordKey], t: u64) -> Vec<ChordAction> {
        keys.iter().map(|k| c.key(*k, true, t)).collect()
    }

    #[test]
    fn holding_the_three_alone_shows_the_preview_after_the_delay() {
        let mut c = Chord::default();
        assert_eq!(hold(&mut c, &[Win { right: false }, Ctrl { right: false }], 0), vec![None, None]);
        assert_eq!(c.key(Shift { right: false }, true, 10), Arm);
        assert_eq!(c.tick(10 + PREVIEW_MS as u64 - 1), None, "not before the delay");
        assert_eq!(c.tick(10 + PREVIEW_MS as u64), Show);
        assert!(c.shown());
        // Auto-repeat of a held modifier: nothing.
        assert_eq!(c.key(Ctrl { right: false }, true, 400), None);
        assert_eq!(c.tick(500), None, "shown once");
        // Releasing any of them: the preview fades away.
        assert_eq!(c.key(Shift { right: false }, false, 600), Hide);
        assert!(!c.shown());
    }

    #[test]
    fn any_order_and_either_side() {
        for order in [[Shift { right: true }, Win { right: true }, Ctrl { right: true }], [Ctrl { right: false }, Shift { right: true }, Win { right: false }]] {
            let mut c = Chord::default();
            let r = hold(&mut c, &order, 0);
            assert_eq!(r, vec![None, None, Arm]);
            assert_eq!(c.tick(PREVIEW_MS as u64), Show);
        }
        // Both shifts down, one released: still held, the preview stays.
        let mut c = Chord::default();
        hold(&mut c, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }, Shift { right: true }], 0);
        c.tick(1000);
        assert_eq!(c.key(Shift { right: false }, false, 1100), None);
        assert!(c.shown());
    }

    #[test]
    fn released_before_the_delay_shows_nothing() {
        let mut c = Chord::default();
        hold(&mut c, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }], 0);
        assert_eq!(c.key(Win { right: false }, false, 50), None);
        assert_eq!(c.tick(PREVIEW_MS as u64 + 10), None);
        assert!(!c.shown());
        // Pressed again: armed afresh.
        assert_eq!(c.key(Win { right: false }, true, 300), Arm);
        assert_eq!(c.tick(300 + PREVIEW_MS as u64), Show);
    }

    #[test]
    fn another_key_cancels_at_once_and_spoils_until_all_released() {
        let mut c = Chord::default();
        hold(&mut c, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }], 0);
        c.tick(200);
        // Win+Ctrl+Shift+B: the preview goes at once (the key passes to Windows untouched).
        assert_eq!(c.key(Other, true, 300), HideNow);
        assert_eq!(c.key(Other, false, 320), None);
        // Still holding the three: no preview again.
        assert_eq!(c.tick(1000), None);
        assert_eq!(c.key(Ctrl { right: false }, true, 1100), None, "repeat does not re-arm");
        // Before the delay: the timer then does nothing.
        let mut d = Chord::default();
        hold(&mut d, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }], 0);
        assert_eq!(d.key(Other, true, 50), None);
        assert_eq!(d.tick(PREVIEW_MS as u64), None);
        // All released: clean again.
        for k in [Win { right: false }, Ctrl { right: false }, Shift { right: false }] {
            c.key(k, false, 2000);
        }
        assert_eq!(hold(&mut c, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }], 3000)[2], Arm);
        // Ctrl+B without Win and Shift: nothing to cancel, but no preview until released.
        let mut e = Chord::default();
        e.key(Ctrl { right: false }, true, 0);
        assert_eq!(e.key(Other, true, 5), None);
        e.key(Win { right: false }, true, 10);
        assert_eq!(e.key(Shift { right: false }, true, 20), None, "spoiled");
    }

    #[test]
    fn arrow_hands_over_to_the_carry() {
        let mut c = Chord::default();
        hold(&mut c, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }], 0);
        c.tick(200);
        // The shortcut: the preview stays (the carry continues with its layers).
        assert_eq!(c.key(Arrow, true, 300), None);
        assert!(c.shown());
        c.reset_preview();
        assert!(!c.shown());
        // An arrow pressed before the delay: the carry starts at once, no preview later.
        let mut d = Chord::default();
        hold(&mut d, &[Win { right: false }, Ctrl { right: false }, Shift { right: false }], 0);
        d.key(Arrow, true, 40);
        assert_eq!(d.tick(PREVIEW_MS as u64 + 50), None);
        // An arrow without the modifiers is ordinary typing.
        let mut e = Chord::default();
        assert_eq!(e.key(Arrow, true, 0), None);
    }
}