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
    /// Entered as a (borderless) fullscreen window, not just maximized.
    pub fullscreen: bool,
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
    /// `"size:mtime"` of the exe of process `pid`; empty when unknown (e.g. process gone).
    fn exe_stamp(&self, pid: u32) -> String;
    /// Human-readable app name from the exe's version info (FileDescription, else ProductName);
    /// empty when unknown. Reads files: never call it on the event hot path.
    fn app_name(&self, _pid: u32) -> String {
        String::new()
    }
    /// Full path of the exe of process `pid`; empty when unknown.
    fn exe_path(&self, _pid: u32) -> String {
        String::new()
    }
    /// How to start the app of process `pid` again: `shell:AppsFolder\<AUMID>` for packaged apps
    /// (their exe path contains the version), else the exe path; empty when unknown.
    fn launch_command(&self, pid: u32) -> String {
        self.exe_path(pid)
    }
    /// Starts an application (`ShellExecute`, without activation where the app allows it);
    /// false when it could not be started.
    /// Must return quickly: the window is awaited by events, not by blocking.
    fn launch(&self, _command: &str, _args: &str) -> bool {
        false
    }
    /// While the profile launches apps: keeps other processes from taking the foreground
    /// (`LockSetForegroundWindow`; best effort, lifted by any user input).
    fn hold_foreground(&self, _on: bool) {}
    /// The window is cloaked (e.g. it lives on another virtual desktop).
    fn is_cloaked(&self, _hwnd: HWND) -> bool {
        false
    }
    /// The foreground window (null when none).
    fn foreground(&self) -> HWND {
        std::ptr::null_mut()
    }
    /// Stops the taskbar button flashing (`FlashWindowEx(FLASHW_STOP)`). An app that asks for the
    /// foreground while `hold_foreground` is on gets no activation, Windows flashes it instead.
    fn stop_flash(&self, _hwnd: HWND) {}
    /// Maximizes without activating (asynchronous; the window's thread may be busy).
    /// False when the window cannot be maximized (no maximize box, e.g. a splash screen).
    fn maximize_inactive(&self, _hwnd: HWND) -> bool {
        false
    }
}
