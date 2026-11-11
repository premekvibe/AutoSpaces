//! Data model and the window-system abstraction used by the orchestrator.

use crate::desktops::Guid;
use windows_sys::Win32::Foundation::HWND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Maximized,
    Fullscreen,
    Minimized,
    Hidden,
    Destroyed,
}

/// Window events routed from the watcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    LocationChange,
    Show,
    Hide,
    Destroy,
    MinimizeStart,
    MoveSizeStart,
    MoveSizeEnd,
    /// A timer requested by the orchestrator (fullscreen settle, end of transition lock, retry).
    Recheck,
}

/// Request to the host: deliver `Event::Recheck` for `hwnd` after `delay_ms`
/// (a repeated request for the same hwnd replaces the earlier one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub hwnd: HWND,
    pub delay_ms: u32,
}

/// A window we moved to a helper desktop of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedSpace {
    pub desktop: Guid,
    pub hwnd: HWND,
    pub pid: u32,
    pub origin: Guid,
    /// `WindowSystem::now()` at entry, ms.
    pub entered_at: u64,
    pub process: String,
    pub class: String,
}

pub trait WindowSystem {
    fn state(&self, hwnd: HWND) -> State;
    /// Real app window that may get its own desktop (plan 6.2).
    fn is_candidate(&self, hwnd: HWND) -> bool;
    /// Lower-case process image name (`foo.exe`, empty when unknown) and process id.
    fn process(&self, hwnd: HWND) -> (String, u32);
    fn class(&self, hwnd: HWND) -> String;
    /// All top-level windows passing `is_candidate`; the orchestrator filters them by
    /// desktop (via `DesktopBackend::window_desktop`) to count windows on a desktop.
    fn app_windows(&self) -> Vec<HWND>;
    fn activate(&self, hwnd: HWND);
    /// Monotonic milliseconds.
    fn now(&self) -> u64;
    /// The window still exists and belongs to process `pid`.
    fn is_alive(&self, hwnd: HWND, pid: u32) -> bool;
    /// The user pressed a key / moved the mouse within the last `ms`.
    fn recent_user_input(&self, ms: u32) -> bool;
}
