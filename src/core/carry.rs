//! Carrying a window between desktops (plan 12.13): the pure decisions (target of a step, when
//! the view catches up with the target, which visual method is used). The orchestrator applies
//! them; the host (main.rs) owns the shortcut, the keyboard hook and the indicator.

use super::settings::CarryStyle;

/// Length of an animated desktop switch; steps pressed faster are merged (no replay of each one).
pub const ANIMATION_MS: u64 = 440;
/// The indicator stays this long after the window was dropped.
pub const OSD_LINGER_MS: u32 = 800;

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
/// (`pin` → `moveSwitch` → `switchMove`, the last works everywhere). Never `Auto`.
pub fn choose_style(pref: CarryStyle, pin: bool, move_switch: bool) -> CarryStyle {
    match pref {
        CarryStyle::Pin if pin => CarryStyle::Pin,
        CarryStyle::MoveSwitch if move_switch => CarryStyle::MoveSwitch,
        CarryStyle::SwitchMove => CarryStyle::SwitchMove,
        _ if pin => CarryStyle::Pin,
        _ if move_switch => CarryStyle::MoveSwitch,
        _ => CarryStyle::SwitchMove,
    }
}

/// Mark at an end of the indicator's row of desktops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    None,
    /// The next step in this direction goes round to the other end.
    Wrap,
    /// The next step to the right creates a new desktop.
    New,
}

/// What the indicator shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Osd {
    /// Number of desktops.
    pub count: usize,
    /// Highlighted desktop (0-based): where the window goes.
    pub target: usize,
    pub left: Edge,
    pub right: Edge,
    /// "This window cannot be moved" instead of the desktops.
    pub cannot_move: bool,
}

impl Osd {
    pub fn new(count: usize, target: usize, create_new: bool) -> Osd {
        let last = count.saturating_sub(1);
        let right = if target < last {
            Edge::None
        } else if create_new {
            Edge::New
        } else if count > 1 {
            Edge::Wrap
        } else {
            Edge::None
        };
        let left = if target > 0 || create_new || count <= 1 { Edge::None } else { Edge::Wrap };
        Osd { count, target: target.min(last), left, right, cannot_move: false }
    }

    pub fn cannot_move() -> Osd {
        Osd { count: 0, target: 0, left: Edge::None, right: Edge::None, cannot_move: true }
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
        assert_eq!(choose_style(Auto, true, true), Pin);
        assert_eq!(choose_style(Auto, false, true), MoveSwitch);
        assert_eq!(choose_style(Auto, false, false), SwitchMove);
        assert_eq!(choose_style(Pin, false, true), MoveSwitch, "unverified pinning: next best");
        assert_eq!(choose_style(Pin, false, false), SwitchMove);
        assert_eq!(choose_style(MoveSwitch, true, true), MoveSwitch);
        assert_eq!(choose_style(MoveSwitch, true, false), Pin, "missing slot: the best available");
        assert_eq!(choose_style(MoveSwitch, false, false), SwitchMove);
        assert_eq!(choose_style(SwitchMove, true, true), SwitchMove);
    }

    #[test]
    fn indicator_edges() {
        let o = Osd::new(3, 2, false);
        assert_eq!((o.left, o.right), (Edge::None, Edge::Wrap));
        let o = Osd::new(3, 0, false);
        assert_eq!((o.left, o.right), (Edge::Wrap, Edge::None));
        let o = Osd::new(3, 2, true);
        assert_eq!((o.left, o.right), (Edge::None, Edge::New));
        let o = Osd::new(3, 0, true);
        assert_eq!((o.left, o.right), (Edge::None, Edge::None));
        let o = Osd::new(1, 0, false);
        assert_eq!((o.left, o.right), (Edge::None, Edge::None));
        assert!(Osd::cannot_move().cannot_move);
    }
}
