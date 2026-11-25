//! "Update" dialog (plan 12.15): template 110 in assets/dialogs.inc. Shows the new version, a link to
//! the release page, "Install" / "Skip this version" / "Later" and the "Check for updates" box. Install
//! downloads and verifies the installer on a worker thread inside the same dialog (progress bar,
//! "Cancel"); the caller starts it. A copy that is not installed, or a release without installer or
//! checksum, gets "Open release page" instead of "Install".

use crate::core::update::{progress_text, InstallKind, Release, Version, RELEASES_PAGE};
use crate::i18n::{self, t, tf, Key};
use crate::tray::wide;
use crate::update::{self, DownloadError, Verified};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::Controls::{
    CheckDlgButton, InitCommonControlsEx, IsDlgButtonChecked, BST_CHECKED, BST_UNCHECKED, ICC_LINK_CLASS, ICC_PROGRESS_CLASS,
    INITCOMMONCONTROLSEX, NMHDR, NM_CLICK, NM_RETURN, PBM_SETPOS, PBM_SETRANGE32,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EndDialog, GetDlgItem, GetWindowLongPtrW, PostMessageW, SendDlgItemMessageW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
    GWLP_USERDATA, IDCANCEL, IDOK, SW_HIDE, SW_SHOW, SW_SHOWNORMAL, WM_APP, WM_COMMAND, WM_INITDIALOG, WM_NOTIFY,
};

const IDD_UPDATE: u16 = 110;
const ID_HEAD: i32 = 250;
const ID_LINK: i32 = 251;
const ID_CHECK: i32 = 252;
const ID_NOTE: i32 = 253;
const ID_PROGRESS: i32 = 254;
const ID_SKIP: i32 = 255;
/// From the download thread: wParam = bytes so far, lParam = total (-1 = unknown).
const WM_DL_PROGRESS: u32 = WM_APP + 51;
/// From the download thread: lParam = `Box<Result<Verified, DownloadError>>`.
const WM_DL_DONE: u32 = WM_APP + 52;

/// Handle of the open dialog (0 = none).
static OPEN: AtomicUsize = AtomicUsize::new(0);

pub fn is_open() -> bool {
    OPEN.load(Relaxed) != 0
}

/// Brings the open dialog forward; false when none is open.
pub fn bring_forward() -> bool {
    let d = OPEN.load(Relaxed);
    d != 0 && unsafe { SetForegroundWindow(d as HWND) } != 0
}

pub enum Choice {
    /// "Later", the close box, "Cancel" during the download, or the release page was opened.
    Later,
    /// "Skip this version".
    Skip,
    /// Downloaded and verified: start it.
    Install(Verified),
}

pub struct Outcome {
    pub choice: Choice,
    /// State of "Check for updates" when the dialog closed.
    pub check_updates: bool,
    /// Lines for the activity log (download problems).
    pub log: Vec<String>,
}

struct Dlg {
    release: Release,
    kind: InstallKind,
    check: bool,
    /// "Install" downloads (else the button opens the release page).
    install: bool,
    cancel: Option<Arc<AtomicBool>>,
    choice: Option<Choice>,
    log: Vec<String>,
}

/// Modal; None when the dialog is already open (it is brought forward).
pub fn show(release: &Release, kind: InstallKind, check_updates: bool) -> Option<Outcome> {
    if bring_forward() || is_open() {
        return None;
    }
    let install = kind != InstallKind::Portable && release.installable();
    let mut d = Dlg { release: release.clone(), kind, check: check_updates, install, cancel: None, choice: None, log: Vec::new() };
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LINK_CLASS | ICC_PROGRESS_CLASS,
        });
        i18n::dialog_box(IDD_UPDATE, std::ptr::null_mut(), Some(proc), &mut d as *mut Dlg as LPARAM);
    }
    OPEN.store(0, Relaxed);
    if let Some(c) = &d.cancel {
        c.store(true, Relaxed);
    }
    Some(Outcome { choice: d.choice.take().unwrap_or(Choice::Later), check_updates: d.check, log: d.log })
}

unsafe fn state<'a>(dlg: HWND) -> Option<&'a mut Dlg> {
    (GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut Dlg).as_mut()
}

/// Text under the link: what "Install" will do, or why it is not offered.
fn note(d: &Dlg) -> String {
    if d.kind == InstallKind::Portable {
        t(Key::UpdatePortableNote).to_string()
    } else if !d.release.installable() {
        t(Key::UpdateNoHash).to_string()
    } else if d.kind == InstallKind::AllUsers {
        format!("{} {}", t(Key::UpdateInstallNote), t(Key::UpdateAdminNote))
    } else {
        t(Key::UpdateInstallNote).to_string()
    }
}

unsafe fn set(dlg: HWND, id: i32, text: &str) {
    crate::about_dlg::set_text(dlg, id, text);
}

/// The release page (only below this repository's releases; checked when the JSON was read).
unsafe fn open_page(dlg: HWND, d: &Dlg) {
    if d.release.html_url.starts_with(RELEASES_PAGE) {
        ShellExecuteW(dlg, wide("open").as_ptr(), wide(&d.release.html_url).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

unsafe fn init(dlg: HWND, d: &Dlg) {
    crate::settings_dlg::set_icons(dlg);
    let v = d.release.version.to_string();
    set(dlg, ID_HEAD, &tf(Key::UpdateAvailable, &[&v, &Version::current().to_string()]));
    set(dlg, ID_LINK, &tf(Key::UpdateWhatsNew, &[&v]));
    set(dlg, ID_NOTE, &note(d));
    ShowWindow(GetDlgItem(dlg, ID_PROGRESS), SW_HIDE);
    SendDlgItemMessageW(dlg, ID_PROGRESS, PBM_SETRANGE32, 0, 1000);
    CheckDlgButton(dlg, ID_CHECK, if d.check { BST_CHECKED } else { BST_UNCHECKED });
    if !d.install {
        set(dlg, IDOK, t(Key::UpdateOpenPage));
    }
    SetFocus(GetDlgItem(dlg, IDOK));
    SetForegroundWindow(dlg);
}

unsafe fn finish(dlg: HWND, d: &mut Dlg, choice: Choice) {
    d.check = IsDlgButtonChecked(dlg, ID_CHECK) == BST_CHECKED;
    if let Some(c) = &d.cancel {
        c.store(true, Relaxed);
    }
    d.choice = Some(choice);
    EndDialog(dlg, 0);
}

/// "Install": the download runs on its own thread; the dialog shows its progress.
unsafe fn start_download(dlg: HWND, d: &mut Dlg) {
    let cancel = Arc::new(AtomicBool::new(false));
    d.cancel = Some(cancel.clone());
    EnableWindow(GetDlgItem(dlg, IDOK), 0);
    EnableWindow(GetDlgItem(dlg, ID_SKIP), 0);
    set(dlg, IDCANCEL, t(Key::UpdateCancel));
    SendDlgItemMessageW(dlg, ID_PROGRESS, PBM_SETPOS, 0, 0);
    ShowWindow(GetDlgItem(dlg, ID_PROGRESS), SW_SHOW);
    set(dlg, ID_NOTE, &tf(Key::UpdateDownloading, &[&progress_text(0, None)]));
    let release = d.release.clone();
    let target = dlg as usize;
    let spawned = std::thread::Builder::new().name("update-download".into()).spawn(move || {
        let mut last = 0u64;
        let mut progress = |done: u64, total: Option<u64>| {
            // About every 64 kB is plenty for a bar.
            if done - last >= 64 * 1024 || total == Some(done) {
                last = done;
                PostMessageW(target as HWND, WM_DL_PROGRESS, done as WPARAM, total.map_or(-1, |t| t as isize));
            }
        };
        let res = update::download_verified(&release, &cancel, &mut progress);
        let msg = Box::into_raw(Box::new(res));
        if PostMessageW(target as HWND, WM_DL_DONE, 0, msg as LPARAM) == 0 {
            drop(Box::from_raw(msg)); // the dialog is gone (cancelled)
        }
    });
    if spawned.is_err() {
        download_failed(dlg, d, "no thread for the download".into());
    }
}

/// Back to the state before "Install", with the reason shown.
unsafe fn download_failed(dlg: HWND, d: &mut Dlg, why: String) {
    d.log.push(format!("update: download of {} failed: {why}", d.release.version));
    d.cancel = None;
    ShowWindow(GetDlgItem(dlg, ID_PROGRESS), SW_HIDE);
    set(dlg, ID_NOTE, t(Key::UpdateDownloadFailed));
    set(dlg, IDCANCEL, t(Key::UpdateLater));
    EnableWindow(GetDlgItem(dlg, IDOK), 1);
    EnableWindow(GetDlgItem(dlg, ID_SKIP), 1);
}

unsafe fn on_done(dlg: HWND, d: &mut Dlg, res: Result<Verified, DownloadError>) {
    match res {
        Ok(v) => {
            d.log.push(format!("update: {} downloaded and verified (SHA-256)", d.release.version));
            set(dlg, ID_NOTE, t(Key::UpdateStarting));
            finish(dlg, d, Choice::Install(v));
        }
        Err(DownloadError::Cancelled) => {}
        Err(DownloadError::Mismatch) => {
            d.log.push(format!("update: the downloaded installer of {} does not match the SHA-256 of the release; deleted", d.release.version));
            d.cancel = None;
            d.install = false;
            ShowWindow(GetDlgItem(dlg, ID_PROGRESS), SW_HIDE);
            set(dlg, ID_NOTE, t(Key::UpdateHashMismatch));
            set(dlg, IDOK, t(Key::UpdateOpenPage));
            set(dlg, IDCANCEL, t(Key::UpdateLater));
            EnableWindow(GetDlgItem(dlg, IDOK), 1);
            EnableWindow(GetDlgItem(dlg, ID_SKIP), 1);
        }
        Err(DownloadError::Other(e)) => download_failed(dlg, d, e),
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
            0
        }
        WM_DL_PROGRESS => {
            if state(dlg).is_some_and(|d| d.cancel.is_some()) {
                let total = (lp >= 0).then_some(lp as u64);
                let done = wp as u64;
                if let Some(t) = total.filter(|t| *t > 0) {
                    SendDlgItemMessageW(dlg, ID_PROGRESS, PBM_SETPOS, (done.min(t) * 1000 / t) as WPARAM, 0);
                }
                set(dlg, ID_NOTE, &tf(Key::UpdateDownloading, &[&progress_text(done, total)]));
            }
            1
        }
        WM_DL_DONE => {
            let res = *Box::from_raw(lp as *mut Result<Verified, DownloadError>);
            if let Some(d) = state(dlg) {
                if d.cancel.is_some() {
                    on_done(dlg, d, res);
                }
            }
            1
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            if nm.idFrom == ID_LINK as usize && (nm.code == NM_CLICK || nm.code == NM_RETURN) {
                if let Some(d) = state(dlg) {
                    open_page(dlg, d);
                }
            }
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            let Some(d) = state(dlg) else { return 0 };
            match id {
                x if x == IDOK => {
                    if d.cancel.is_some() {
                        return 1; // already downloading
                    }
                    if d.install {
                        start_download(dlg, d);
                    } else {
                        open_page(dlg, d);
                        finish(dlg, d, Choice::Later);
                    }
                }
                ID_SKIP => finish(dlg, d, Choice::Skip),
                x if x == IDCANCEL => finish(dlg, d, Choice::Later),
                _ => return 0,
            }
            1
        }
        _ => 0,
    }
}
