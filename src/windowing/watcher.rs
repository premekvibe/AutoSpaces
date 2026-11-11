//! WinEvent hooks. The callback is minimal: filter, then PostMessageW.

use crate::core::Event;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering::Relaxed};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetAncestor, PostMessageW, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_MINIMIZESTART,
    EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, GA_ROOT, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
};

/// Private message posted to the hidden window: wParam = hwnd, lParam = event.
pub const WM_WINEVENT: u32 = WM_APP + 2;

/// Total hook callbacks.
pub static TOTAL: AtomicU64 = AtomicU64::new(0);
/// Callbacks that passed the filter (and were posted).
pub static PASSED: AtomicU64 = AtomicU64::new(0);

static TARGET: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

pub struct Watcher {
    hooks: [HWINEVENTHOOK; 4],
}

unsafe extern "system" fn callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    TOTAL.fetch_add(1, Relaxed);
    if id_object != OBJID_WINDOW || id_child != 0 || hwnd.is_null() {
        return;
    }
    let root = GetAncestor(hwnd, GA_ROOT);
    // A window being destroyed may no longer be queryable (root is null).
    if root != hwnd && !(event == EVENT_OBJECT_DESTROY && root.is_null()) {
        return;
    }
    PASSED.fetch_add(1, Relaxed);
    PostMessageW(TARGET.load(Relaxed), WM_WINEVENT, hwnd as usize, event as isize);
}

impl Watcher {
    pub fn start(target: HWND) -> Watcher {
        TARGET.store(target, Relaxed);
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let ranges = [
            (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZESTART),
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
        ];
        let mut hooks = [std::ptr::null_mut(); 4];
        for (h, (lo, hi)) in hooks.iter_mut().zip(ranges) {
            *h = unsafe {
                SetWinEventHook(lo, hi, std::ptr::null_mut(), Some(callback), 0, 0, flags)
            };
        }
        Watcher { hooks }
    }

    pub fn stop(&mut self) {
        for h in self.hooks.iter_mut() {
            if !h.is_null() {
                unsafe { UnhookWinEvent(*h) };
                *h = std::ptr::null_mut();
            }
        }
    }
}

/// Maps a WinEvent id to the core event type.
pub fn to_event(event: u32) -> Option<Event> {
    Some(match event {
        EVENT_OBJECT_LOCATIONCHANGE => Event::LocationChange,
        EVENT_OBJECT_SHOW => Event::Show,
        EVENT_OBJECT_HIDE => Event::Hide,
        EVENT_OBJECT_DESTROY => Event::Destroy,
        EVENT_SYSTEM_MINIMIZESTART => Event::MinimizeStart,
        EVENT_SYSTEM_MOVESIZESTART => Event::MoveSizeStart,
        EVENT_SYSTEM_MOVESIZEEND => Event::MoveSizeEnd,
        _ => return None,
    })
}
