//! Abstraction over the virtual desktop implementation (plan 4.2).

use super::guid::Guid;
use std::fmt;
use windows_sys::Win32::Foundation::HWND;

/// Backend error: `Transient` = Explorer / COM server unavailable or restarting
/// (re-initialise and retry later), `Permanent` = the operation itself failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transient(i32),
    Permanent(i32),
}

pub type Result<T> = std::result::Result<T, Error>;

/// `IApplicationViewCollection::GetViewForHwnd`: no view for this window.
pub const TYPE_E_ELEMENTNOTFOUND: i32 = 0x8002_802B_u32 as i32;

/// HRESULTs meaning "the server (explorer.exe) is gone / not reachable right now".
const TRANSIENT_HRESULTS: [u32; 9] = [
    0x800706BA, // RPC_S_SERVER_UNAVAILABLE
    0x800706BE, // RPC_S_CALL_FAILED
    0x800706BF, // RPC_S_CALL_FAILED_DNE
    0x80010108, // RPC_E_DISCONNECTED
    0x80010012, // RPC_E_SERVER_DIED_DNE
    0x80010007, // RPC_E_SERVER_DIED
    0x80010001, // RPC_E_CALL_REJECTED
    0x8001010A, // RPC_E_SERVERCALL_RETRYLATER
    0x800401FD, // CO_E_OBJNOTCONNECTED
];

impl Error {
    pub fn from_hr(hr: i32) -> Error {
        let u = hr as u32;
        if TRANSIENT_HRESULTS.contains(&u) {
            Error::Transient(hr)
        } else {
            Error::Permanent(hr)
        }
    }

    pub fn hr(&self) -> i32 {
        match *self {
            Error::Transient(h) | Error::Permanent(h) => h,
        }
    }

    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Transient(_))
    }

    /// The window has no application view (yet): Explorer creates it shortly after the window is
    /// shown, and windows outside the task switcher never get one. Not a failure of the window.
    pub fn is_no_view(&self) -> bool {
        self.hr() == TYPE_E_ELEMENTNOTFOUND
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transient(h) => write!(f, "transient 0x{:08X}", *h as u32),
            Error::Permanent(h) => write!(f, "permanent 0x{:08X}", *h as u32),
        }
    }
}

/// Converts an HRESULT into `Result<()>`.
pub fn check(hr: i32) -> Result<()> {
    if hr >= 0 {
        Ok(())
    } else {
        Err(Error::from_hr(hr))
    }
}

pub trait DesktopBackend {
    /// e.g. "internal:win11-24h2".
    fn name(&self) -> &str;
    /// All desktops in display order.
    fn desktops(&self) -> Result<Vec<Guid>>;
    fn current(&self) -> Result<Guid>;
    /// Cheap check, done BEFORE creating a desktop. A window without an application view yields
    /// an error with `is_no_view()` (it may get one a moment later), not `Ok(false)`.
    fn can_move(&self, hwnd: HWND) -> Result<bool>;
    /// Creates a desktop (always appended at the end).
    fn create(&self) -> Result<Guid>;
    fn move_window(&self, hwnd: HWND, desk: Guid) -> Result<()>;
    fn switch(&self, desk: Guid, animated: bool) -> Result<()>;
    /// Moves remaining windows to `fallback` and switches there if `desk` is visible.
    fn remove(&self, desk: Guid, fallback: Guid) -> Result<()>;
    /// Public API; `None` when unknown (hidden window, TYPE_E_ELEMENTNOTFOUND, ...).
    fn window_desktop(&self, hwnd: HWND) -> Option<Guid>;

    // ---- carrying a window (plan 12.13); the defaults mean "not available".

    /// `switch(_, true)` really animates (the variant has `SwitchDesktopWithAnimation`).
    fn animates(&self) -> bool {
        false
    }
    /// The variant has `SwitchDesktopAndMoveForegroundView`.
    fn has_move_switch(&self) -> bool {
        false
    }
    /// Moves the foreground window to `desk` and switches there (one call).
    fn move_switch(&self, _desk: Guid) -> Result<()> {
        Err(Error::Permanent(E_NOTIMPL))
    }
    /// The variant describes `IVirtualDesktopPinnedApps` (sources agree); nothing is called yet.
    fn has_pin(&self) -> bool {
        false
    }
    /// Read-only check before the first pin of a run: the interface is obtained by `QueryService`
    /// and `IsViewPinned` of `hwnd` (a window the public API places on a desktop, so it is not
    /// pinned) must answer false. Err = description of what failed.
    fn pin_probe(&self, _hwnd: HWND) -> std::result::Result<(), String> {
        Err("no pinning interface".into())
    }
    /// Pins (`true`) or unpins the window's view to / from all desktops.
    fn set_pinned(&self, _hwnd: HWND, _on: bool) -> Result<()> {
        Err(Error::Permanent(E_NOTIMPL))
    }
    /// `IsViewPinned` of the window.
    fn is_pinned(&self, _hwnd: HWND) -> Result<bool> {
        Err(Error::Permanent(E_NOTIMPL))
    }

    // ---- wallpapers (plan 12.17); the defaults mean "not available".

    /// (`SetDesktopWallpaper` described, `UpdateWallpaperPathForAllDesktops` described).
    fn wallpaper_slots(&self) -> (bool, bool) {
        (false, false)
    }
    /// Sets the wallpaper of one desktop (undocumented; only on a variant that passed the self-test).
    fn set_desktop_wallpaper(&self, _desk: Guid, _path: &str) -> Result<()> {
        Err(Error::Permanent(E_NOTIMPL))
    }
    /// Sets one wallpaper on every desktop (undocumented, as above).
    fn set_all_wallpapers(&self, _path: &str) -> Result<()> {
        Err(Error::Permanent(E_NOTIMPL))
    }
}

pub const E_NOTIMPL: i32 = 0x8000_4001_u32 as i32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_rpc_failures_as_transient() {
        for hr in [0x800706BAu32, 0x800706BE, 0x80010108, 0x80010012, 0x800401FD] {
            assert!(Error::from_hr(hr as i32).is_transient(), "{hr:#x}");
        }
    }

    #[test]
    fn classifies_others_as_permanent() {
        for hr in [0x80004005u32, 0x80070057, 0x8002802B, 0x80004002, 0] {
            assert_eq!(Error::from_hr(hr as i32), Error::Permanent(hr as i32));
        }
    }

    #[test]
    fn check_maps_success_codes() {
        assert_eq!(check(0), Ok(()));
        assert_eq!(check(1), Ok(())); // S_FALSE
        assert_eq!(check(0x800706BAu32 as i32), Err(Error::Transient(0x800706BAu32 as i32)));
    }
}
