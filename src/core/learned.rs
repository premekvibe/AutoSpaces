//! Learned list of unmovable apps (plan 6.7). The real store is a later phase.

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

pub trait Learned {
    fn blocked(&self, process: &str, class: &str) -> bool;
    fn fail(&mut self, process: &str, class: &str, reason: FailReason, hard: bool);
}

/// Placeholder: blocks nothing, remembers nothing.
pub struct NoLearned;

impl Learned for NoLearned {
    fn blocked(&self, _process: &str, _class: &str) -> bool {
        false
    }
    fn fail(&mut self, _process: &str, _class: &str, _reason: FailReason, _hard: bool) {}
}
