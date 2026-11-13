//! Settings dialog (plan 12.2): plain Win32 dialog templates from assets/autospaces.rc,
//! created only on request (no cost while idle). Ids below must match the .rc file.

use crate::core::settings::{display_label, file_name_lower, renumber_rules, Layout, LayoutRule, MAX_LAYOUT_DESKTOPS, SYSTEM_HOSTS};
use crate::core::Settings;
use crate::i18n::{self, t, tf, Key, Pref, LANGS};
use crate::sysopts;
use crate::tray::wide;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MapWindowPoints, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    CheckDlgButton, CheckRadioButton, DrawInsert, EnableThemeDialogTexture, IsDlgButtonChecked, LBItemFromPt, MakeDragList,
    BST_CHECKED, BST_UNCHECKED, DL_BEGINDRAG, DL_DRAGGING, DL_DROPPED, DL_MOVECURSOR, DL_STOPCURSOR, DRAGLISTINFO, ETDT_ENABLE, ETDT_USETABTEXTURE,
    InitCommonControlsEx, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, NMHDR, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCN_SELCHANGE,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT, VK_TAB};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, EndDialog, GetDlgItem, GetDlgItemInt, SetDlgItemInt, SetDlgItemTextW, SetWindowTextW,
    LB_RESETCONTENT, LBN_SELCHANGE, LB_SETHORIZONTALEXTENT, GetWindowLongPtrW, GetWindowTextW, LoadImageW, SendDlgItemMessageW,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, GWLP_USERDATA, GWLP_ID,
    ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, IDCANCEL, IDOK, LBN_DBLCLK, LB_ADDSTRING, LB_DELETESTRING, LB_ERR,
    LB_GETCOUNT, LB_GETCURSEL, LB_GETITEMDATA, LB_SETITEMDATA, LB_SETCURSEL, WM_COMMAND, WM_INITDIALOG,
    WM_SETICON, SW_SHOWNORMAL, DWLP_MSGRESULT, MessageBoxW, RegisterWindowMessageW, MB_ICONINFORMATION, MB_OK,
    CallNextHookEx, GetAncestor, GetClientRect, GetParent, GetWindowRect, SetWindowPos, SetWindowsHookExW, ShowWindow,
    UnhookWindowsHookEx, GA_ROOT, MSG, MSGF_DIALOGBOX, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WH_MSGFILTER,
    WM_KEYDOWN, WM_NOTIFY,
};

const IDD_SETTINGS: u16 = 101;
const IDD_PICK: u16 = 102;
const ID_MODE_ALL: i32 = 110;
const ID_MODE_FS: i32 = 111;
const ID_ARRANGE: i32 = 134;
const ID_EXCL_LIST: i32 = 120;
const ID_EXCL_ADD: i32 = 121;
const ID_EXCL_DEL: i32 = 122;
const ID_LEARNED_LIST: i32 = 130;
const ID_LEARNED_DEL: i32 = 131;
const ID_LEARNING: i32 = 132;
const ID_AUTOSTART: i32 = 140;
const ID_OPEN_WINSETTINGS: i32 = 141;
const ID_OPEN_LAYOUT: i32 = 143;
const ID_LANGUAGE: i32 = 144;
const ID_HINT: i32 = 142;
const ID_LOG: i32 = 147;
const IDD_LAYOUT: u16 = 103;
const ID_LAY_ENABLED: i32 = 160;
const ID_LAY_COUNT: i32 = 161;
const ID_LAY_UP: i32 = 169;
const ID_LAY_DOWN: i32 = 170;
const ID_LAY_COUNT_LABEL: i32 = 171;
const ID_LAY_INFO: i32 = 173;
const EN_KILLFOCUS: u32 = 0x0200;
const ID_LAY_LIST: i32 = 163;
const ID_LAY_ADD_RUN: i32 = 164;
const ID_LAY_ADD_FILE: i32 = 165;
const ID_LAY_DEL: i32 = 166;
const ID_LAY_CAPTURE: i32 = 167;
const ID_LAY_APPLY: i32 = 168;
const ID_PICK_LIST: i32 = 150;
/// Tab control of the settings dialog and its pages: child dialogs 106..108 (ids 310.. inside the dialog).
const ID_TAB: i32 = 300;
const ID_PAGE0: i32 = 310;
const IDD_PAGE0: u16 = 106;
const PAGES: usize = 3;

/// Handle of the open dialog (0 = none): a second request just brings it forward.
static OPEN: AtomicUsize = AtomicUsize::new(0);
/// Message hook that gives Ctrl+Tab / Ctrl+Shift+Tab to the tab control while the dialog is open.
static HOOK: AtomicUsize = AtomicUsize::new(0);

/// True while the settings dialog is open.
pub fn is_open() -> bool {
    OPEN.load(Relaxed) != 0
}

/// What the user confirmed.
pub struct Outcome {
    pub settings: Settings,
    /// Learned processes to forget.
    pub forgotten: Vec<String>,
    /// The user asked to apply the layout profile right now ("Apply now" button).
    pub apply_now: bool,
}

struct Dlg {
    settings: Settings,
    /// (process, readable name); list items carry the index as item data.
    learned: Vec<(String, String)>,
    excl: Vec<(String, String)>,
    forgotten: Vec<String>,
    out: Option<Outcome>,
    /// Source of the "running apps" picker.
    running: fn() -> Vec<(String, String)>,
    /// Current desktops and windows as a layout profile (number of desktops, rules).
    capture: fn() -> (usize, Vec<LayoutRule>),
}

const EXPLORER: &str = "explorer.exe";

/// Running apps for the picker: one row per process, minus excluded ones, sorted by name.
pub fn picker_entries(raw: Vec<(String, String)>, excluded: &[String], own: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for (p, t) in raw {
        if p.is_empty() || p == own || (p != EXPLORER && SYSTEM_HOSTS.contains(&p.as_str())) || excluded.contains(&p) || v.iter().any(|(q, _)| *q == p) {
            continue;
        }
        v.push((p, t));
    }
    v.sort();
    v
}

/// Opens the modal dialog; None = cancelled (or already open).
pub fn show(
    settings: &Settings,
    learned: Vec<(String, String)>,
    running: fn() -> Vec<(String, String)>,
    capture: fn() -> (usize, Vec<LayoutRule>),
) -> Option<Outcome> {
    let prev = OPEN.load(Relaxed);
    if prev != 0 {
        unsafe { SetForegroundWindow(prev as HWND) };
        return None;
    }
    let mut dlg = Dlg {
        settings: settings.clone(),
        learned,
        excl: settings.excluded.iter().map(|p| (p.clone(), settings.names.get(p).cloned().unwrap_or_default())).collect(),
        forgotten: Vec::new(),
        out: None,
        running,
        capture,
    };
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_TAB_CLASSES });
        let hook = SetWindowsHookExW(WH_MSGFILTER, Some(key_hook), std::ptr::null_mut(), GetCurrentThreadId());
        HOOK.store(hook as usize, Relaxed);
        i18n::dialog_box(IDD_SETTINGS, std::ptr::null_mut(), Some(proc), &mut dlg as *mut Dlg as LPARAM);
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
        HOOK.store(0, Relaxed);
    }
    OPEN.store(0, Relaxed);
    dlg.out
}

/// The page (child dialog) that holds control `id`; the control is looked up in it, not in the main dialog.
unsafe fn pg(dlg: HWND, id: i32) -> HWND {
    for i in 0..PAGES as i32 {
        let page = GetDlgItem(dlg, ID_PAGE0 + i);
        if !page.is_null() && !GetDlgItem(page, id).is_null() {
            return page;
        }
    }
    dlg
}

/// Index of the tab reached from `cur` by `step` (wraps around).
fn next_tab(cur: isize, step: isize, count: usize) -> usize {
    (cur + step).rem_euclid(count.max(1) as isize) as usize
}

/// Shows the page of tab `i`, hides the others.
unsafe fn show_page(dlg: HWND, i: usize) {
    for k in 0..PAGES {
        let page = GetDlgItem(dlg, ID_PAGE0 + k as i32);
        ShowWindow(page, if k == i { SW_SHOW } else { SW_HIDE });
    }
}

/// Ctrl+Tab / Ctrl+Shift+Tab: the next / previous tab, keyboard focus on the tab control.
unsafe fn switch_tab(dlg: HWND, step: isize) {
    let tab = GetDlgItem(dlg, ID_TAB);
    let cur = SendMessageW(tab, TCM_GETCURSEL, 0, 0);
    let next = next_tab(cur, step, PAGES);
    SendMessageW(tab, TCM_SETCURSEL, next, 0);
    show_page(dlg, next);
    SetFocus(tab);
}

unsafe extern "system" fn key_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == MSGF_DIALOGBOX as i32 && lp != 0 {
        let m = &*(lp as *const MSG);
        let dlg = OPEN.load(Relaxed) as HWND;
        if m.message == WM_KEYDOWN
            && m.wParam == VK_TAB as usize
            && GetKeyState(VK_CONTROL as i32) < 0
            && !dlg.is_null()
            && GetAncestor(m.hwnd, GA_ROOT) == dlg
        {
            switch_tab(dlg, if GetKeyState(VK_SHIFT as i32) < 0 { -1 } else { 1 });
            return 1;
        }
    }
    CallNextHookEx(HOOK.load(Relaxed) as _, code, wp, lp)
}

/// Where a window of `win` (left, top, right, bottom) must go to lie inside `work`, and how many pixels
/// of height must be cut because it is taller than `work`: (x, y, cut).
fn fit_in_work(win: [i32; 4], work: [i32; 4]) -> (i32, i32, i32) {
    let (w, h) = (win[2] - win[0], win[3] - win[1]);
    let cut = (h - (work[3] - work[1])).max(0);
    let h = h - cut;
    let x = if w >= work[2] - work[0] { work[0] } else { win[0].clamp(work[0], work[2] - w) };
    let y = win[1].clamp(work[1], (work[3] - h).max(work[1]));
    (x, y, cut)
}

unsafe fn rect_in(dlg: HWND, child: HWND) -> RECT {
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(child, &mut r);
    MapWindowPoints(std::ptr::null_mut(), dlg, &mut r as *mut RECT as *mut POINT, 2);
    r
}

/// Puts the pages exactly into the display area of the tab control.
unsafe fn layout_pages(dlg: HWND) {
    let tab = GetDlgItem(dlg, ID_TAB);
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(tab, &mut r);
    SendMessageW(tab, TCM_ADJUSTRECT, 0, &mut r as *mut RECT as LPARAM);
    let mut o = POINT { x: 0, y: 0 };
    MapWindowPoints(tab, dlg, &mut o, 1);
    for k in 0..PAGES as i32 {
        let page = GetDlgItem(dlg, ID_PAGE0 + k);
        // Right after the tab control in the z-order: the tab key goes tab -> page -> OK -> Cancel.
        SetWindowPos(page, tab, o.x + r.left, o.y + r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE);
    }
}

/// Safety net: the whole dialog inside the work area of its monitor; when it is taller than the work area, the
/// tab control gets shorter so that the top edge and the OK / Cancel buttons stay visible.
unsafe fn fit_to_work_area(dlg: HWND) {
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(MonitorFromWindow(dlg, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
        return;
    }
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(dlg, &mut r);
    let wk = mi.rcWork;
    let (x, y, cut) = fit_in_work([r.left, r.top, r.right, r.bottom], [wk.left, wk.top, wk.right, wk.bottom]);
    if cut > 0 {
        let tab = GetDlgItem(dlg, ID_TAB);
        let t = rect_in(dlg, tab);
        let cut = cut.min((t.bottom - t.top - 80).max(0));
        SetWindowPos(tab, std::ptr::null_mut(), t.left, t.top, t.right - t.left, t.bottom - t.top - cut, SWP_NOZORDER | SWP_NOACTIVATE);
        for id in [IDOK, IDCANCEL] {
            let b = GetDlgItem(dlg, id);
            let br = rect_in(dlg, b);
            SetWindowPos(b, std::ptr::null_mut(), br.left, br.top - cut, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE);
        }
        layout_pages(dlg);
        SetWindowPos(dlg, std::ptr::null_mut(), x, y, r.right - r.left, r.bottom - r.top - cut, SWP_NOZORDER | SWP_NOACTIVATE);
    } else {
        SetWindowPos(dlg, std::ptr::null_mut(), x, y, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE);
    }
}

unsafe extern "system" fn page_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            // The tab page background of the visual styles (no grey rectangles behind the labels).
            EnableThemeDialogTexture(dlg, ETDT_ENABLE | ETDT_USETABTEXTURE);
            0
        }
        WM_COMMAND => {
            // Everything is handled by the settings dialog itself.
            SendMessageW(GetParent(dlg), WM_COMMAND, wp, lp);
            1
        }
        _ => 0,
    }
}

/// Adds a row labelled `text`; `data` (index into the dialog's own vectors) travels with it.
unsafe fn list_add(dlg: HWND, id: i32, text: &str, data: usize) {
    let idx = SendDlgItemMessageW(dlg, id, LB_ADDSTRING, 0, wide(text).as_ptr() as LPARAM);
    if idx >= 0 {
        SendDlgItemMessageW(dlg, id, LB_SETITEMDATA, idx as usize, data as LPARAM);
    }
}

unsafe fn list_data(dlg: HWND, id: i32) -> Vec<usize> {
    let n = SendDlgItemMessageW(dlg, id, LB_GETCOUNT, 0, 0).max(0) as usize;
    (0..n).map(|i| SendDlgItemMessageW(dlg, id, LB_GETITEMDATA, i, 0) as usize).collect()
}

/// Removes the selected row; returns its item data and keeps a neighbour selected.
unsafe fn list_remove_selected(dlg: HWND, id: i32) -> Option<usize> {
    let sel = SendDlgItemMessageW(dlg, id, LB_GETCURSEL, 0, 0);
    if sel == LB_ERR as isize {
        return None;
    }
    let data = SendDlgItemMessageW(dlg, id, LB_GETITEMDATA, sel as usize, 0) as usize;
    SendDlgItemMessageW(dlg, id, LB_DELETESTRING, sel as usize, 0);
    let n = SendDlgItemMessageW(dlg, id, LB_GETCOUNT, 0, 0);
    if n > 0 {
        SendDlgItemMessageW(dlg, id, LB_SETCURSEL, (sel as usize).min(n as usize - 1), 0);
    }
    Some(data)
}

unsafe fn update_buttons(dlg: HWND) {
    for (list, btn) in [(ID_EXCL_LIST, ID_EXCL_DEL), (ID_LEARNED_LIST, ID_LEARNED_DEL)] {
        let has = SendDlgItemMessageW(pg(dlg, list), list, LB_GETCURSEL, 0, 0) != LB_ERR as isize;
        EnableWindow(GetDlgItem(pg(dlg, btn), btn), has as i32);
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

/// Gives the dialog the application icon at the size of its own DPI (shared handles: nothing to free).
pub(crate) unsafe fn set_icons(dlg: HWND) {
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
}

/// Tabs, pages (child dialogs) and their initial layout; the first tab is shown.
unsafe fn create_pages(dlg: HWND) {
    let tab = GetDlgItem(dlg, ID_TAB);
    for (i, key) in [Key::TabGeneral, Key::TabApps, Key::TabAdvanced].into_iter().enumerate() {
        let text = wide(t(key));
        let mut item: TCITEMW = std::mem::zeroed();
        item.mask = TCIF_TEXT;
        item.pszText = text.as_ptr() as *mut u16;
        SendMessageW(tab, TCM_INSERTITEMW, i, &item as *const TCITEMW as LPARAM);
    }
    for k in 0..PAGES as i32 {
        let page = i18n::create_dialog(IDD_PAGE0 + k as u16, dlg, Some(page_proc), 0);
        SetWindowLongPtrW(page, GWLP_ID, (ID_PAGE0 + k) as isize);
    }
    layout_pages(dlg);
    show_page(dlg, 0);
}

unsafe fn init(dlg: HWND, d: &Dlg) {
    set_icons(dlg);
    create_pages(dlg);
    SetDlgItemTextW(pg(dlg, ID_HINT), ID_HINT, wide(t(Key::SettingsHint)).as_ptr());
    let fs = d.settings.fullscreen_only();
    CheckRadioButton(pg(dlg, ID_MODE_ALL), ID_MODE_ALL, ID_MODE_FS, if fs { ID_MODE_FS } else { ID_MODE_ALL });
    for (i, (p, n)) in d.excl.iter().enumerate() {
        list_add(pg(dlg, ID_EXCL_LIST), ID_EXCL_LIST, &display_label(p, n), i);
    }
    for (i, (p, n)) in d.learned.iter().enumerate() {
        list_add(pg(dlg, ID_LEARNED_LIST), ID_LEARNED_LIST, &display_label(p, n), i);
    }
    CheckDlgButton(pg(dlg, ID_LEARNING), ID_LEARNING, check(d.settings.learning));
    CheckDlgButton(pg(dlg, ID_ARRANGE), ID_ARRANGE, check(d.settings.arrange_at_start));
    CheckDlgButton(pg(dlg, ID_LOG), ID_LOG, check(d.settings.log));
    // Autostart is read from the system, not from settings.json.
    CheckDlgButton(pg(dlg, ID_AUTOSTART), ID_AUTOSTART, check(sysopts::autostart_enabled()));
    // Language: automatic, then every language under its own name.
    let names = std::iter::once(t(Key::LangAuto)).chain(LANGS.iter().map(|l| l.native_name()));
    for n in names {
        SendDlgItemMessageW(pg(dlg, ID_LANGUAGE), ID_LANGUAGE, CB_ADDSTRING, 0, wide(n).as_ptr() as LPARAM);
    }
    SendDlgItemMessageW(pg(dlg, ID_LANGUAGE), ID_LANGUAGE, CB_SETCURSEL, d.settings.language.index(), 0);
    update_buttons(dlg);
}

/// Checked state of a button of any page.
unsafe fn checked(dlg: HWND, id: i32) -> bool {
    IsDlgButtonChecked(pg(dlg, id), id) == BST_CHECKED
}

/// Reads every page, also those the user never opened (their controls exist all the time).
unsafe fn collect(dlg: HWND, d: &mut Dlg) {
    let mut s = d.settings.clone();
    s.set_fullscreen_only(checked(dlg, ID_MODE_FS));
    s.excluded.clear();
    s.names.clear();
    for i in list_data(pg(dlg, ID_EXCL_LIST), ID_EXCL_LIST) {
        if let Some((p, n)) = d.excl.get(i) {
            s.exclude(p);
            s.set_name(p, n);
        }
    }
    s.learning = checked(dlg, ID_LEARNING);
    s.arrange_at_start = checked(dlg, ID_ARRANGE);
    s.autostart = checked(dlg, ID_AUTOSTART);
    s.log = checked(dlg, ID_LOG);
    let sel = SendDlgItemMessageW(pg(dlg, ID_LANGUAGE), ID_LANGUAGE, CB_GETCURSEL, 0, 0);
    s.language = if sel < 0 { d.settings.language } else { Pref::from_index(sel as usize) };
    d.out = Some(Outcome {
        settings: s,
        forgotten: std::mem::take(&mut d.forgotten),
        apply_now: false,
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
            fit_to_work_area(dlg);
            // The focus starts on the tab control (arrow keys and Ctrl+Tab change the tab).
            SetFocus(GetDlgItem(dlg, ID_TAB));
            0
        }
        WM_NOTIFY => {
            let h = &*(lp as *const NMHDR);
            if h.idFrom == ID_TAB as usize && h.code == TCN_SELCHANGE {
                show_page(dlg, SendMessageW(h.hwndFrom, TCM_GETCURSEL, 0, 0).max(0) as usize);
            }
            0
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
                    if let Some((p, n)) = pick(dlg, d) {
                        let have = list_data(pg(dlg, ID_EXCL_LIST), ID_EXCL_LIST);
                        if !have.iter().any(|&i| d.excl.get(i).is_some_and(|e| e.0 == p)) {
                            d.excl.push((p.clone(), n.clone()));
                            list_add(pg(dlg, ID_EXCL_LIST), ID_EXCL_LIST, &display_label(&p, &n), d.excl.len() - 1);
                        }
                        update_buttons(dlg);
                    }
                }
                ID_EXCL_DEL => {
                    list_remove_selected(pg(dlg, ID_EXCL_LIST), ID_EXCL_LIST);
                    update_buttons(dlg);
                }
                ID_LEARNED_DEL => {
                    if let Some(i) = list_remove_selected(pg(dlg, ID_LEARNED_LIST), ID_LEARNED_LIST) {
                        if let Some((p, _)) = d.learned.get(i) {
                            d.forgotten.push(p.clone());
                        }
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
                ID_OPEN_LAYOUT => {
                    if layout_dialog(dlg, d) {
                        // "Apply now": confirm the whole settings dialog and apply the profile.
                        collect(dlg, d);
                        if let Some(o) = d.out.as_mut() {
                            o.apply_now = true;
                        }
                        EndDialog(dlg, IDOK as isize);
                    }
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
    chosen: Option<(String, String)>,
    title: String,
    button: String,
}

/// Second dialog: choose from running apps. Returns the process name.
unsafe fn pick(parent: HWND, d: &Dlg) -> Option<(String, String)> {
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let excl: Vec<String> =
        list_data(pg(parent, ID_EXCL_LIST), ID_EXCL_LIST).into_iter().filter_map(|i| d.excl.get(i).map(|e| e.0.clone())).collect();
    pick_from(parent, picker_entries((d.running)(), &excl, &own), t(Key::PickExcludeTitle), t(Key::PickExcludeButton))
}

/// Shows the picker dialog with its own caption and confirm button; the chosen entry or None.
unsafe fn pick_from(parent: HWND, entries: Vec<(String, String)>, title: &str, button: &str) -> Option<(String, String)> {
    let mut st = Pick { entries, chosen: None, title: title.into(), button: button.into() };
    i18n::dialog_box(IDD_PICK, parent, Some(pick_proc), &mut st as *mut Pick as LPARAM);
    st.chosen
}

unsafe extern "system" fn pick_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
            if let Some(st) = (lp as *mut Pick).as_ref() {
                SetWindowTextW(dlg, wide(&st.title).as_ptr());
                SetDlgItemTextW(dlg, IDOK, wide(&st.button).as_ptr());
                for (p, t) in &st.entries {
                    list_add(dlg, ID_PICK_LIST, &display_label(p, t), 0);
                }
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
                        st.chosen = st.entries.get(sel as usize).cloned();
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

/// One row per running app process: (process, readable name). The name comes from the exe's
/// version info, else the title of its first window, else it is empty.
pub fn running_apps() -> Vec<(String, String)> {
    running_apps_full().into_iter().map(|(p, n, _)| (p, n)).collect()
}

/// Like `running_apps`, plus how to launch it (`shell:AppsFolder\<AUMID>` or the exe path).
pub fn running_apps_full() -> Vec<(String, String, String)> {
    use crate::core::WindowSystem;
    let ws = crate::windowing::system::Win32Windows::new();
    let mut seen: Vec<(String, u32, String)> = Vec::new();
    for w in ws.app_windows() {
        let (p, pid) = ws.process(w);
        if p.is_empty() || seen.iter().any(|s| s.0 == p) {
            continue;
        }
        let mut buf = [0u16; 200];
        let n = unsafe { GetWindowTextW(w, buf.as_mut_ptr(), buf.len() as i32) };
        seen.push((p, pid, String::from_utf16_lossy(&buf[..n.max(0) as usize])));
    }
    seen.into_iter()
        .map(|(p, pid, title)| {
            let name = if p == EXPLORER { t(Key::FileExplorer).to_string() } else { ws.app_name(pid) };
            (p, if name.is_empty() { title } else { name }, ws.launch_command(pid))
        })
        .collect()
}

// ---------------------------------------------------------------- layout profile (plan 12.7)

struct LayDlg {
    layout: Layout,
    capture: fn() -> (usize, Vec<LayoutRule>),
    apply: bool,
    /// Registered message of the drag list box ("commctrl_DragListMsg").
    drag_msg: u32,
    /// Row being dragged.
    drag_src: Option<usize>,
}

/// The layout dialog; true when the user chose "Apply now" (the profile is stored in `d`).
unsafe fn layout_dialog(parent: HWND, d: &mut Dlg) -> bool {
    let mut st = LayDlg { layout: d.settings.layout.clone(), capture: d.capture, apply: false, drag_msg: 0, drag_src: None };
    let r = i18n::dialog_box(IDD_LAYOUT, parent, Some(layout_proc), &mut st as *mut LayDlg as LPARAM);
    if r != IDOK as isize {
        return false;
    }
    st.layout.renumber();
    d.settings.layout = st.layout;
    st.apply
}

fn read_num(dlg: HWND, id: i32, default: u32) -> u32 {
    let mut ok = 0;
    let v = unsafe { GetDlgItemInt(dlg, id, &mut ok, 0) };
    if ok == 0 {
        default
    } else {
        v.clamp(1, MAX_LAYOUT_DESKTOPS)
    }
}

/// The desktop count may exceed the minimum (empty desktops on top) but never fall below it.
unsafe fn lay_fix_count(dlg: HWND, l: &Layout) {
    let min = l.min_count();
    SetDlgItemTextW(dlg, ID_LAY_COUNT_LABEL, wide(&tf(Key::LayoutCountLabel, &[&min.to_string()])).as_ptr());
    if read_num(dlg, ID_LAY_COUNT, 1) < min {
        SetDlgItemInt(dlg, ID_LAY_COUNT, min, 0);
    }
}

/// Fills the list in the order of the rules (the order gives the desktops: the first application
/// is desktop 2, ...); item data = index into the rules. `sel` is selected afterwards.
unsafe fn lay_refill(dlg: HWND, l: &mut Layout, sel: Option<usize>) {
    renumber_rules(&mut l.rules);
    SendDlgItemMessageW(dlg, ID_LAY_LIST, LB_RESETCONTENT, 0, 0);
    let mut longest = 0;
    for (i, r) in l.rules.iter().enumerate() {
        let text = tf(Key::LayoutRow, &[&r.desktop.to_string(), &r.label()]);
        longest = longest.max(text.chars().count());
        list_add(dlg, ID_LAY_LIST, &text, i);
    }
    // Long names scroll sideways instead of being cut off (extent in pixels, generous per char).
    let dpi = GetDpiForWindow(dlg).max(96) as usize;
    SendDlgItemMessageW(dlg, ID_LAY_LIST, LB_SETHORIZONTALEXTENT, longest * 8 * dpi / 96 + 16, 0);
    if let Some(s) = sel.filter(|s| *s < l.rules.len()) {
        SendDlgItemMessageW(dlg, ID_LAY_LIST, LB_SETCURSEL, s, 0);
    }
    lay_fix_count(dlg, l);
    lay_buttons(dlg, l);
}

unsafe fn lay_buttons(dlg: HWND, l: &Layout) {
    let sel = SendDlgItemMessageW(dlg, ID_LAY_LIST, LB_GETCURSEL, 0, 0);
    let has = sel != LB_ERR as isize;
    let i = sel.max(0) as usize;
    EnableWindow(GetDlgItem(dlg, ID_LAY_DEL), has as i32);
    EnableWindow(GetDlgItem(dlg, ID_LAY_UP), (has && i > 0) as i32);
    EnableWindow(GetDlgItem(dlg, ID_LAY_DOWN), (has && i + 1 < l.rules.len()) as i32);
}

/// Moves a row (drag or the up / down buttons): the desktops follow the new order at once.
unsafe fn lay_move(dlg: HWND, l: &mut Layout, from: usize, to: usize) {
    if from >= l.rules.len() || to >= l.rules.len() || from == to {
        return;
    }
    let r = l.rules.remove(from);
    l.rules.insert(to, r);
    lay_refill(dlg, l, Some(to));
}

/// New applications go to the end of the list: the next desktop.
unsafe fn lay_add(dlg: HWND, l: &mut Layout, name: &str, launch: &str, process: &str) {
    if l.rules.len() + 1 >= MAX_LAYOUT_DESKTOPS as usize {
        let text = tf(Key::LayoutFull, &[&(MAX_LAYOUT_DESKTOPS - 1).to_string()]);
        MessageBoxW(dlg, wide(&text).as_ptr(), wide(t(Key::AppName)).as_ptr(), MB_OK | MB_ICONINFORMATION);
        return;
    }
    if let Some(r) = LayoutRule::new(l.rules.len() as u32 + 2, name, launch, "", process) {
        if !l.rules.iter().any(|x| x.process == r.process && x.launch == r.launch) {
            l.rules.push(r);
            lay_refill(dlg, l, Some(l.rules.len() - 1));
        }
    }
}

/// Standard file dialog for an exe; the chosen path.
unsafe fn choose_exe(parent: HWND) -> Option<String> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    let mut file = [0u16; 1040];
    let filter: Vec<u16> =
        format!("{}\0*.exe\0{}\0*.*\0\0", t(Key::FileFilterPrograms), t(Key::FileFilterAll)).encode_utf16().collect();
    let title = wide(t(Key::FileChooseApp));
    let mut ofn: OPENFILENAMEW = std::mem::zeroed();
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = parent;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file.as_mut_ptr();
    ofn.nMaxFile = file.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY;
    if GetOpenFileNameW(&mut ofn) == 0 {
        return None;
    }
    let n = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(String::from_utf16_lossy(&file[..n]))
}

unsafe fn lay_collect(dlg: HWND, st: &mut LayDlg) {
    st.layout.enabled = IsDlgButtonChecked(dlg, ID_LAY_ENABLED) == BST_CHECKED;
    st.layout.desktop_count = read_num(dlg, ID_LAY_COUNT, 1).max(st.layout.min_count());
    st.layout.renumber();
}

/// Drag list box notification (`MakeDragList`): the row follows the mouse, the insert mark shows
/// where it lands. Returns the dialog message result.
unsafe fn lay_drag(dlg: HWND, st: &mut LayDlg, info: &DRAGLISTINFO) -> isize {
    let lb = info.hWnd;
    let result = match info.uNotification {
        DL_BEGINDRAG => {
            let item = LBItemFromPt(lb, info.ptCursor, 0);
            if item < 0 {
                0
            } else {
                st.drag_src = Some(item as usize);
                SendMessageW(lb, LB_SETCURSEL, item as usize, 0);
                lay_buttons(dlg, &st.layout);
                1
            }
        }
        DL_DRAGGING => {
            let item = LBItemFromPt(lb, info.ptCursor, 1);
            DrawInsert(dlg, lb, item);
            if item >= 0 {
                DL_MOVECURSOR as isize
            } else {
                DL_STOPCURSOR as isize
            }
        }
        DL_DROPPED => {
            DrawInsert(dlg, lb, -1);
            let item = LBItemFromPt(lb, info.ptCursor, 0);
            if let (Some(from), true) = (st.drag_src.take(), item >= 0) {
                lay_move(dlg, &mut st.layout, from, item as usize);
            }
            0
        }
        _ => {
            // DL_CANCELDRAG
            DrawInsert(dlg, lb, -1);
            st.drag_src = None;
            0
        }
    };
    SetWindowLongPtrW(dlg, DWLP_MSGRESULT as i32, result);
    1
}

unsafe extern "system" fn layout_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    if let Some(st) = (GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut LayDlg).as_mut() {
        if st.drag_msg != 0 && msg == st.drag_msg && lp != 0 {
            return lay_drag(dlg, st, &*(lp as *const DRAGLISTINFO));
        }
    }
    match msg {
        WM_INITDIALOG => {
            SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
            if let Some(st) = (lp as *mut LayDlg).as_mut() {
                SetDlgItemTextW(dlg, ID_LAY_INFO, wide(t(Key::LayoutInfo)).as_ptr());
                CheckDlgButton(dlg, ID_LAY_ENABLED, check(st.layout.enabled));
                SetDlgItemInt(dlg, ID_LAY_COUNT, st.layout.count() as u32, 0);
                MakeDragList(GetDlgItem(dlg, ID_LAY_LIST));
                st.drag_msg = RegisterWindowMessageW(wide("commctrl_DragListMsg").as_ptr());
                lay_refill(dlg, &mut st.layout, None);
            }
            1
        }
        WM_COMMAND => {
            let id = (wp & 0xffff) as i32;
            let notify = ((wp >> 16) & 0xffff) as u32;
            let Some(st) = (GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut LayDlg).as_mut() else { return 0 };
            let sel = SendDlgItemMessageW(dlg, ID_LAY_LIST, LB_GETCURSEL, 0, 0);
            let sel = (sel != LB_ERR as isize).then_some(sel as usize);
            match id {
                x if x == IDOK || x == ID_LAY_APPLY => {
                    lay_collect(dlg, st);
                    st.apply = x == ID_LAY_APPLY;
                    EndDialog(dlg, IDOK as isize);
                }
                x if x == IDCANCEL => {
                    EndDialog(dlg, IDCANCEL as isize);
                }
                ID_LAY_LIST => {
                    if notify == LBN_SELCHANGE {
                        lay_buttons(dlg, &st.layout);
                    }
                }
                ID_LAY_COUNT => {
                    if notify == EN_KILLFOCUS {
                        lay_fix_count(dlg, &st.layout);
                    }
                }
                ID_LAY_ADD_RUN => {
                    let full = running_apps_full();
                    let own = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
                        .unwrap_or_default();
                    let raw: Vec<(String, String)> = full.iter().map(|(p, n, _)| (p.clone(), n.clone())).collect();
                    let entries = picker_entries(raw, &[], &own);
                    if let Some((p, n)) = pick_from(dlg, entries, t(Key::PickLayoutTitle), t(Key::PickLayoutButton)) {
                        let path = full.iter().find(|f| f.0 == p).map(|f| f.2.clone()).unwrap_or_default();
                        lay_add(dlg, &mut st.layout, &n, &path, &p);
                    }
                }
                ID_LAY_ADD_FILE => {
                    if let Some(path) = choose_exe(dlg) {
                        let name = crate::windowing::system::version_name(&path);
                        let launch = crate::launch_for_path(&path);
                        lay_add(dlg, &mut st.layout, &name, &launch, &file_name_lower(&path));
                    }
                }
                ID_LAY_DEL => {
                    if let Some(i) = sel.filter(|i| *i < st.layout.rules.len()) {
                        st.layout.rules.remove(i);
                        let next = i.min(st.layout.rules.len().saturating_sub(1));
                        lay_refill(dlg, &mut st.layout, Some(next));
                    }
                }
                ID_LAY_UP | ID_LAY_DOWN => {
                    if let Some(i) = sel {
                        let to = if id == ID_LAY_UP { i.saturating_sub(1) } else { i + 1 };
                        lay_move(dlg, &mut st.layout, i, to);
                        // A button that just got disabled must not keep the keyboard focus.
                        SetFocus(GetDlgItem(dlg, ID_LAY_LIST));
                    }
                }
                ID_LAY_CAPTURE => {
                    let (n, rules) = (st.capture)();
                    if n > 0 {
                        st.layout.rules = rules;
                        st.layout.desktop_count = n as u32;
                        SetDlgItemInt(dlg, ID_LAY_COUNT, n as u32, 0);
                        lay_refill(dlg, &mut st.layout, None);
                    }
                }
                _ => return 0,
            }
            1
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_wrap_around() {
        assert_eq!(next_tab(0, 1, 3), 1);
        assert_eq!(next_tab(2, 1, 3), 0);
        assert_eq!(next_tab(0, -1, 3), 2);
        assert_eq!(next_tab(1, -1, 3), 0);
        assert_eq!(next_tab(0, 1, 0), 0);
    }

    #[test]
    fn dialog_is_moved_into_the_work_area() {
        let work = [0, 0, 1366, 728];
        // Fits and is inside: untouched.
        assert_eq!(fit_in_work([100, 100, 600, 500], work), (100, 100, 0));
        // Sticks out at the bottom and on the right: moved up and left.
        assert_eq!(fit_in_work([1000, 500, 1500, 800], work), (866, 428, 0));
        // Sticks out at the top and on the left.
        assert_eq!(fit_in_work([-50, -20, 450, 380], work), (0, 0, 0));
        // Taller than the work area: top edge at the top, the rest is cut.
        assert_eq!(fit_in_work([300, 10, 800, 900], work), (300, 0, 162));
        // Work area not at the origin (second monitor).
        assert_eq!(fit_in_work([2000, 0, 2400, 300], [1920, 0, 3840, 1040]), (2000, 0, 0));
        assert_eq!(fit_in_work([1800, 900, 2200, 1200], [1920, 0, 3840, 1040]), (1920, 740, 0));
    }

    #[test]
    fn picker_dedupes_filters_and_sorts() {
        let raw = vec![
            ("b.exe".to_string(), "B1".to_string()),
            ("a.exe".to_string(), "A".to_string()),
            ("b.exe".to_string(), "B2".to_string()),
            ("".to_string(), "unknown".to_string()),
            ("autospaces.exe".to_string(), "me".to_string()),
            ("x.exe".to_string(), "excluded".to_string()),
            ("explorer.exe".to_string(), "File Explorer".to_string()),
            ("systemsettings.exe".to_string(), "s".to_string()),
            ("autospaces.exe".to_string(), "a".to_string()),
        ];
        let v = picker_entries(raw, &["x.exe".to_string()], "autospaces.exe");
        assert_eq!(v, vec![
                ("a.exe".to_string(), "A".to_string()),
                ("b.exe".to_string(), "B1".to_string()),
                ("explorer.exe".to_string(), "File Explorer".to_string())
            ]);
    }
}
