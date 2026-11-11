//! Settings dialog (plan 12.2): plain Win32 dialog templates from assets/autospaces.rc,
//! created only on request (no cost while idle). Ids below must match the .rc file.

use crate::core::Settings;
use crate::sysopts;
use crate::tray::wide;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{CheckDlgButton, CheckRadioButton, IsDlgButtonChecked, BST_CHECKED, BST_UNCHECKED};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DialogBoxParamW, EndDialog, GetDlgItem, GetWindowLongPtrW, GetWindowTextW, LoadImageW, SendDlgItemMessageW,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, GWLP_USERDATA,
    ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, LB_SETHORIZONTALEXTENT, IDCANCEL, IDOK, LBN_DBLCLK, LB_ADDSTRING, LB_DELETESTRING, LB_ERR,
    LB_GETCOUNT, LB_GETCURSEL, LB_GETTEXT, LB_GETTEXTLEN, LB_SETCURSEL, WM_COMMAND, WM_INITDIALOG,
    WM_SETICON, SW_SHOWNORMAL,
};

const IDD_SETTINGS: u16 = 101;
const IDD_PICK: u16 = 102;
const ID_MODE_ALL: i32 = 110;
const ID_MODE_FS: i32 = 111;
const ID_EXCL_LIST: i32 = 120;
const ID_EXCL_ADD: i32 = 121;
const ID_EXCL_DEL: i32 = 122;
const ID_LEARNED_LIST: i32 = 130;
const ID_LEARNED_DEL: i32 = 131;
const ID_LEARNING: i32 = 132;
const ID_AUTOSTART: i32 = 140;
const ID_OPEN_WINSETTINGS: i32 = 141;
const ID_PICK_LIST: i32 = 150;

/// Handle of the open dialog (0 = none): a second request just brings it forward.
static OPEN: AtomicUsize = AtomicUsize::new(0);

/// What the user confirmed.
pub struct Outcome {
    pub settings: Settings,
    /// Learned processes to forget.
    pub forgotten: Vec<String>,
}

struct Dlg {
    settings: Settings,
    learned: Vec<String>,
    forgotten: Vec<String>,
    out: Option<Outcome>,
    /// Source of the "running apps" picker.
    running: fn() -> Vec<(String, String)>,
}

/// System hosts that never make sense to exclude.
const SYSTEM_HOSTS: [&str; 8] = [
    "applicationframehost.exe",
    "systemsettings.exe",
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "explorer.exe",
    "autospaces.exe",
];

/// Running apps for the picker: one row per process, minus excluded ones, sorted by name.
pub fn picker_entries(raw: Vec<(String, String)>, excluded: &[String], own: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for (p, t) in raw {
        if p.is_empty() || p == own || SYSTEM_HOSTS.contains(&p.as_str()) || excluded.contains(&p) || v.iter().any(|(q, _)| *q == p) {
            continue;
        }
        v.push((p, t));
    }
    v.sort();
    v
}

/// Opens the modal dialog; None = cancelled (or already open).
pub fn show(settings: &Settings, learned: Vec<String>, running: fn() -> Vec<(String, String)>) -> Option<Outcome> {
    let prev = OPEN.load(Relaxed);
    if prev != 0 {
        unsafe { SetForegroundWindow(prev as HWND) };
        return None;
    }
    let mut dlg = Dlg {
        settings: settings.clone(),
        learned,
        forgotten: Vec::new(),
        out: None,
        running,
    };
    unsafe {
        DialogBoxParamW(
            GetModuleHandleW(std::ptr::null()),
            std::ptr::without_provenance::<u16>(IDD_SETTINGS as usize),
            std::ptr::null_mut(),
            Some(proc),
            &mut dlg as *mut Dlg as LPARAM,
        );
    }
    OPEN.store(0, Relaxed);
    dlg.out
}

unsafe fn list_add(dlg: HWND, id: i32, text: &str) {
    SendDlgItemMessageW(dlg, id, LB_ADDSTRING, 0, wide(text).as_ptr() as LPARAM);
}

unsafe fn list_text(dlg: HWND, id: i32, idx: usize) -> String {
    let n = SendDlgItemMessageW(dlg, id, LB_GETTEXTLEN, idx, 0);
    if n < 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let n = SendDlgItemMessageW(dlg, id, LB_GETTEXT, idx, buf.as_mut_ptr() as LPARAM);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

unsafe fn list_all(dlg: HWND, id: i32) -> Vec<String> {
    let n = SendDlgItemMessageW(dlg, id, LB_GETCOUNT, 0, 0).max(0) as usize;
    (0..n).map(|i| list_text(dlg, id, i)).collect()
}

/// Removes the selected row; returns its text and keeps a neighbour selected.
unsafe fn list_remove_selected(dlg: HWND, id: i32) -> Option<String> {
    let sel = SendDlgItemMessageW(dlg, id, LB_GETCURSEL, 0, 0);
    if sel == LB_ERR as isize {
        return None;
    }
    let text = list_text(dlg, id, sel as usize);
    SendDlgItemMessageW(dlg, id, LB_DELETESTRING, sel as usize, 0);
    let n = SendDlgItemMessageW(dlg, id, LB_GETCOUNT, 0, 0);
    if n > 0 {
        SendDlgItemMessageW(dlg, id, LB_SETCURSEL, (sel as usize).min(n as usize - 1), 0);
    }
    Some(text)
}

unsafe fn update_buttons(dlg: HWND) {
    for (list, btn) in [(ID_EXCL_LIST, ID_EXCL_DEL), (ID_LEARNED_LIST, ID_LEARNED_DEL)] {
        let has = SendDlgItemMessageW(dlg, list, LB_GETCURSEL, 0, 0) != LB_ERR as isize;
        EnableWindow(GetDlgItem(dlg, btn), has as i32);
    }
}

unsafe fn state<'a>(dlg: HWND) -> Option<&'a mut Dlg> {
    (GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut Dlg).as_mut()
}

fn check(on: bool) -> u32 {
    if on {
        BST_CHECKED
    } else {
        BST_UNCHECKED
    }
}

unsafe fn init(dlg: HWND, d: &Dlg) {
    // Icons at the size of the window's own DPI (shared handles: nothing to free).
    let dpi = GetDpiForWindow(dlg).max(96);
    for (kind, cx, cy) in [(ICON_BIG, SM_CXICON, SM_CYICON), (ICON_SMALL, SM_CXSMICON, SM_CYSMICON)] {
        let icon = LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            std::ptr::without_provenance::<u16>(1),
            IMAGE_ICON,
            GetSystemMetricsForDpi(cx, dpi),
            GetSystemMetricsForDpi(cy, dpi),
            LR_SHARED,
        );
        if !icon.is_null() {
            SendMessageW(dlg, WM_SETICON, kind as usize, icon as LPARAM);
        }
    }
    let fs = d.settings.fullscreen_only();
    CheckRadioButton(dlg, ID_MODE_ALL, ID_MODE_FS, if fs { ID_MODE_FS } else { ID_MODE_ALL });
    for p in &d.settings.excluded {
        list_add(dlg, ID_EXCL_LIST, p);
    }
    for p in &d.learned {
        list_add(dlg, ID_LEARNED_LIST, p);
    }
    CheckDlgButton(dlg, ID_LEARNING, check(d.settings.learning));
    // Autostart is read from the system, not from settings.json.
    CheckDlgButton(dlg, ID_AUTOSTART, check(sysopts::autostart_enabled()));
    update_buttons(dlg);
}

unsafe fn collect(dlg: HWND, d: &mut Dlg) {
    let mut s = d.settings.clone();
    s.set_fullscreen_only(IsDlgButtonChecked(dlg, ID_MODE_FS) == BST_CHECKED);
    s.excluded.clear();
    for p in list_all(dlg, ID_EXCL_LIST) {
        s.exclude(&p);
    }
    s.learning = IsDlgButtonChecked(dlg, ID_LEARNING) == BST_CHECKED;
    s.autostart = IsDlgButtonChecked(dlg, ID_AUTOSTART) == BST_CHECKED;
    d.out = Some(Outcome {
        settings: s,
        forgotten: std::mem::take(&mut d.forgotten),
    });
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
                x if x == IDOK => {
                    collect(dlg, d);
                    EndDialog(dlg, IDOK as isize);
                }
                x if x == IDCANCEL => {
                    EndDialog(dlg, IDCANCEL as isize);
                }
                ID_EXCL_ADD => {
                    if let Some(p) = pick(dlg, d) {
                        if !list_all(dlg, ID_EXCL_LIST).contains(&p) {
                            list_add(dlg, ID_EXCL_LIST, &p);
                        }
                        update_buttons(dlg);
                    }
                }
                ID_EXCL_DEL => {
                    list_remove_selected(dlg, ID_EXCL_LIST);
                    update_buttons(dlg);
                }
                ID_LEARNED_DEL => {
                    if let Some(p) = list_remove_selected(dlg, ID_LEARNED_LIST) {
                        d.forgotten.push(p);
                    }
                    update_buttons(dlg);
                }
                ID_OPEN_WINSETTINGS => {
                    ShellExecuteW(
                        dlg,
                        wide("open").as_ptr(),
                        wide("ms-settings:multitasking").as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        SW_SHOWNORMAL,
                    );
                }
                ID_EXCL_LIST | ID_LEARNED_LIST => update_buttons(dlg),
                _ => return 0,
            }
            1
        }
        _ => 0,
    }
}

struct Pick {
    entries: Vec<(String, String)>,
    chosen: Option<String>,
}

/// Second dialog: choose from running apps. Returns the process name.
unsafe fn pick(parent: HWND, d: &Dlg) -> Option<String> {
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let excl = list_all(parent, ID_EXCL_LIST);
    let mut st = Pick { entries: picker_entries((d.running)(), &excl, &own), chosen: None };
    DialogBoxParamW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance::<u16>(IDD_PICK as usize),
        parent,
        Some(pick_proc),
        &mut st as *mut Pick as LPARAM,
    );
    st.chosen
}

unsafe extern "system" fn pick_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
            if let Some(st) = (lp as *mut Pick).as_ref() {
                for (p, t) in &st.entries {
                    list_add(dlg, ID_PICK_LIST, &format!("{p} – {t}"));
                }
                // Horizontal scroll range for long titles: 700 px at 96 DPI, scaled to the window.
                let ext = 700 * GetDpiForWindow(dlg).max(96) as usize / 96;
                SendDlgItemMessageW(dlg, ID_PICK_LIST, LB_SETHORIZONTALEXTENT, ext, 0);
                if !st.entries.is_empty() {
                    SendDlgItemMessageW(dlg, ID_PICK_LIST, LB_SETCURSEL, 0, 0);
                }
            }
            1
        }
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            let notify = ((wp >> 16) & 0xffff) as u32;
            let accept = id == IDOK || (id == ID_PICK_LIST && notify == LBN_DBLCLK);
            if accept {
                let sel = SendDlgItemMessageW(dlg, ID_PICK_LIST, LB_GETCURSEL, 0, 0);
                if let Some(st) = (GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut Pick).as_mut() {
                    if sel >= 0 {
                        st.chosen = st.entries.get(sel as usize).map(|e| e.0.clone());
                    }
                }
                EndDialog(dlg, IDOK as isize);
                return 1;
            }
            if id == IDCANCEL {
                EndDialog(dlg, IDCANCEL as isize);
                return 1;
            }
            0
        }
        _ => 0,
    }
}

/// Process name + title of every running app window (candidates only).
pub fn running_apps() -> Vec<(String, String)> {
    use crate::core::WindowSystem;
    let ws = crate::windowing::system::Win32Windows::new();
    ws.app_windows()
        .into_iter()
        .map(|w| {
            let mut buf = [0u16; 200];
            let n = unsafe { GetWindowTextW(w, buf.as_mut_ptr(), buf.len() as i32) };
            (ws.process(w).0, String::from_utf16_lossy(&buf[..n.max(0) as usize]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_dedupes_filters_and_sorts() {
        let raw = vec![
            ("b.exe".to_string(), "B1".to_string()),
            ("a.exe".to_string(), "A".to_string()),
            ("b.exe".to_string(), "B2".to_string()),
            ("".to_string(), "unknown".to_string()),
            ("autospaces.exe".to_string(), "me".to_string()),
            ("x.exe".to_string(), "excluded".to_string()),
            ("explorer.exe".to_string(), "e".to_string()),
            ("systemsettings.exe".to_string(), "s".to_string()),
            ("autospaces.exe".to_string(), "a".to_string()),
        ];
        let v = picker_entries(raw, &["x.exe".to_string()], "autospaces.exe");
        assert_eq!(v, vec![("a.exe".to_string(), "A".to_string()), ("b.exe".to_string(), "B1".to_string())]);
    }
}
