//! "Report a problem" dialog (plan 12.12): template 105 in assets/dialogs.inc, created only on request.
//! It creates `report.txt` (the content is built by the owner through `Hooks`), shows it in the
//! folder, opens it in Notepad on request and opens the GitHub issue form. Nothing is sent.

use crate::about_dlg::{hide, item_rect, move_up, set_text, shrink_dialog};
use crate::i18n::{self, t, Key};
use crate::report::ISSUE_URL;
use crate::tray::wide;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EndDialog, GetDlgItem, GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW, GWLP_USERDATA, IDCANCEL, SW_SHOWNORMAL,
    WM_COMMAND, WM_INITDIALOG,
};

const IDD_REPORT: u16 = 105;
const ID_INTRO: i32 = 230;
const ID_CREATE: i32 = 231;
const ID_VIEW: i32 = 232;
const ID_STATUS: i32 = 233;
const ID_GITHUB: i32 = 234;
const ID_LOG_WARN: i32 = 235;
const ID_LOG_ON: i32 = 236;
/// Controls below the logging block that move up when the block disappears.
const BELOW_LOG: [i32; 5] = [ID_CREATE, ID_VIEW, ID_STATUS, ID_GITHUB, IDCANCEL];

/// Handle of the open dialog (0 = none): a second request just brings it forward.
static OPEN: AtomicUsize = AtomicUsize::new(0);

/// True while the dialog is open.
pub fn is_open() -> bool {
    OPEN.load(Relaxed) != 0
}

/// What the dialog asks of the application.
pub struct Hooks {
    /// Writes `report.txt`; its path, or None when it could not be written.
    pub create: fn() -> Option<PathBuf>,
    /// Is the activity log switched on (by the settings)?
    pub logging_on: fn() -> bool,
    /// Switches the activity log on (settings.json and the running app).
    pub enable_logging: fn(),
}

struct Dlg {
    hooks: Hooks,
    report: Option<PathBuf>,
    /// The logging block (warning + button) is still shown.
    log_block: bool,
}

pub fn show(hooks: Hooks) {
    let prev = OPEN.load(Relaxed);
    if prev != 0 {
        unsafe { SetForegroundWindow(prev as HWND) };
        return;
    }
    let mut dlg = Dlg { hooks, report: None, log_block: true };
    unsafe {
        i18n::dialog_box(IDD_REPORT, std::ptr::null_mut(), Some(proc), &mut dlg as *mut Dlg as LPARAM);
    }
    OPEN.store(0, Relaxed);
}

unsafe fn state<'a>(dlg: HWND) -> Option<&'a mut Dlg> {
    (GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut Dlg).as_mut()
}

/// Removes the logging block and the room it took.
unsafe fn drop_log_block(dlg: HWND, d: &mut Dlg) {
    if !d.log_block {
        return;
    }
    d.log_block = false;
    let dy = item_rect(dlg, ID_CREATE).top - item_rect(dlg, ID_LOG_WARN).top;
    hide(dlg, ID_LOG_WARN);
    hide(dlg, ID_LOG_ON);
    for id in BELOW_LOG {
        move_up(dlg, id, dy);
    }
    shrink_dialog(dlg, dy);
}

unsafe fn open(dlg: HWND, file: &str, params: Option<&str>) {
    let p = params.map(wide);
    ShellExecuteW(
        dlg,
        wide("open").as_ptr(),
        wide(file).as_ptr(),
        p.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
        std::ptr::null(),
        SW_SHOWNORMAL,
    );
}

unsafe fn init(dlg: HWND, d: &mut Dlg) {
    crate::settings_dlg::set_icons(dlg);
    set_text(dlg, ID_INTRO, t(Key::ReportIntro));
    set_text(dlg, ID_LOG_WARN, t(Key::ReportLogWarn));
    EnableWindow(GetDlgItem(dlg, ID_VIEW), 0);
    if (d.hooks.logging_on)() {
        drop_log_block(dlg, d);
    }
}

unsafe extern "system" fn proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
            OPEN.store(dlg as usize, Relaxed);
            if let Some(d) = state(dlg) {
                init(dlg, d);
            }
            1
        }
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            let Some(d) = state(dlg) else { return 0 };
            match id {
                x if x == IDCANCEL => {
                    EndDialog(dlg, IDCANCEL as isize);
                }
                ID_LOG_ON => {
                    (d.hooks.enable_logging)();
                    drop_log_block(dlg, d);
                    set_text(dlg, ID_STATUS, t(Key::ReportLogNowOn));
                }
                ID_CREATE => match (d.hooks.create)() {
                    Some(p) => {
                        // Selected in Explorer, ready to be dragged into the report.
                        open(dlg, "explorer.exe", Some(&format!("/select,\"{}\"", p.display())));
                        d.report = Some(p);
                        EnableWindow(GetDlgItem(dlg, ID_VIEW), 1);
                        set_text(dlg, ID_STATUS, t(Key::ReportCreated));
                    }
                    None => set_text(dlg, ID_STATUS, t(Key::ReportFailed)),
                },
                ID_VIEW => {
                    if let Some(p) = &d.report {
                        open(dlg, "notepad.exe", Some(&format!("\"{}\"", p.display())));
                    }
                }
                ID_GITHUB => open(dlg, ISSUE_URL, None),
                _ => return 0,
            }
            1
        }
        _ => 0,
    }
}
