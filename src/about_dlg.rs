//! "O aplikaci" dialog and the support reminder (plan 12.4/12.6/12.9): template 104 in assets/autospaces.rc, created only on
//! request (the reminder is the same template, modeless and shown without activation). The donate QR code is an RLE bitmap resource that is decoded and scaled when the
//! dialog opens and freed when it closes (nothing stays in memory while idle).

use crate::i18n::{self, t, tf, Key};
use crate::tray::wide;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{GlobalFree, HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, CreateSolidBrush, DeleteDC, DeleteObject, FillRect,
    GetDC, GetObjectW, GetStockObject, GetSysColorBrush, Polygon, Ellipse, ReleaseDC, ScreenToClient, COLOR_BTNFACE, NULL_PEN, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt, BITMAP, COLORONCOLOR, HALFTONE, LOGFONTW,
    SRCCOPY,
};
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ};
use windows_sys::Win32::UI::Controls::{CheckRadioButton, DRAWITEMSTRUCT, IsDlgButtonChecked, BST_CHECKED, InitCommonControlsEx, ICC_LINK_CLASS, INITCOMMONCONTROLSEX, NMHDR, NM_CLICK, NM_RETURN};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, ShellExecuteW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, EndDialog, GetClientRect, GetDlgItem, GetWindowRect,
    IsDialogMessageW, LoadImageW, PostMessageW, SendDlgItemMessageW, SendMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow, MSG, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_APP, WM_DRAWITEM, IDCANCEL, IDOK, IMAGE_BITMAP, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED,
    STM_SETICON, STM_SETIMAGE, SW_SHOWNORMAL, WM_COMMAND, WM_DESTROY, WM_GETFONT, WM_INITDIALOG, WM_NOTIFY, WM_SETFONT,
};

const IDD_ABOUT: u16 = 104;
const IDB_QR_ACCOUNT: u16 = 20;
const IDB_QR_REVOLUT: u16 = 21;
const ID_PAY_REVOLUT: i32 = 215;
const ID_PAY_ACCOUNT: i32 = 216;
const ID_CAPTION: i32 = 209;
const ID_SUMMARY: i32 = 203;
const ID_AUTHOR: i32 = 204;
const ID_NEVER: i32 = 217;
const ID_NOTE: i32 = 218;
const ID_GROUP: i32 = 219;
const ID_LOGO: i32 = 220;
/// Controls below the author lines that move up when those lines are hidden (reminder mode).
const PAY_BLOCK: [i32; 14] = [219, 215, 216, 208, 209, 210, 211, 212, 213, 214, 220, 217, 218, 1];
/// Posted to the owner when the support reminder closes; wParam = "never again" was ticked.
pub const WM_SUPPORT_CLOSED: u32 = WM_APP + 20;
const ID_ICON: i32 = 200;
const ID_TITLE: i32 = 201;
const ID_VERSION: i32 = 202;
const ID_LINK: i32 = 205;
const ID_VARIANT: i32 = 206;
const ID_QR: i32 = 208;
const ID_ADDRESS: i32 = 211;
const ID_COPY: i32 = 212;
const ID_WALLET: i32 = 213;
const ID_STATUS: i32 = 214;
const BN_CLICKED: u32 = 0;
const CF_UNICODETEXT: u32 = 13;

/// Revolut user name exactly as printed under the QR code in assets/donate-revolut-source.jpg.
pub const REVOLUT_NAME: &str = "@pemyslih2";
pub const AUTHOR_URL: &str = "https://www.premysl-vavrousek.cz";
pub const LIGHTNING_ADDRESS: &str = "premek.v@bitlifi.com";

/// Handle of the open dialog (0 = none): a second request just brings it forward.
static OPEN: AtomicUsize = AtomicUsize::new(0);
/// Resources owned by the open dialog, released in WM_DESTROY.
static TITLE_FONT: AtomicUsize = AtomicUsize::new(0);
static QR_BITMAP: AtomicUsize = AtomicUsize::new(0);
/// The reminder (modeless): its window and owner.
static SUPPORT_HWND: AtomicUsize = AtomicUsize::new(0);
static SUPPORT_OWNER: AtomicUsize = AtomicUsize::new(0);

struct Ctx {
    variant: String,
    support: bool,
}

fn is_support(dlg: HWND) -> bool {
    SUPPORT_HWND.load(Relaxed) == dlg as usize
}

/// True while any of the two dialogs is open.
pub fn is_open() -> bool {
    OPEN.load(Relaxed) != 0
}

/// For the main message loop: lets Tab/Enter/Esc work in the modeless reminder.
pub fn handle_message(msg: &MSG) -> bool {
    let d = SUPPORT_HWND.load(Relaxed);
    d != 0 && unsafe { IsDialogMessageW(d as HWND, msg) } != 0
}

/// Which payment QR code is shown.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pay {
    Revolut,
    Account,
}

impl Pay {
    fn bitmap_id(self) -> u16 {
        match self {
            Pay::Revolut => IDB_QR_REVOLUT,
            Pay::Account => IDB_QR_ACCOUNT,
        }
    }
    fn caption(self) -> String {
        match self {
            Pay::Revolut => tf(Key::PayRevolutCaption, &[REVOLUT_NAME]),
            Pay::Account => t(Key::PayAccountCaption).to_string(),
        }
    }
}

pub fn lightning_uri() -> String {
    format!("lightning:{LIGHTNING_ADDRESS}")
}

/// "Version 0.1.0" (version from Cargo.toml, the single source of truth).
pub fn version_label() -> String {
    tf(Key::AboutVersion, &[env!("CARGO_PKG_VERSION")])
}

/// `backend` is the name reported by the desktop backend (e.g. "internal:win11-24h2"), if any.
pub fn variant_label(backend: Option<&str>) -> String {
    match backend {
        Some(b) => tf(Key::AboutVariant, &[b.strip_prefix("internal:").unwrap_or(b)]),
        None => t(Key::AboutVariantNone).to_string(),
    }
}

/// Opens the modal dialog (no-op, except bringing it forward, when already open).
pub fn show(backend: Option<&str>) {
    let prev = OPEN.load(Relaxed);
    if prev != 0 {
        unsafe { SetForegroundWindow(prev as HWND) };
        return;
    }
    let ctx = Ctx { variant: variant_label(backend), support: false };
    unsafe {
        init_controls();
        i18n::dialog_box(IDD_ABOUT, std::ptr::null_mut(), Some(proc), &ctx as *const Ctx as LPARAM);
    }
    OPEN.store(0, Relaxed);
}

unsafe fn init_controls() {
    InitCommonControlsEx(&INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LINK_CLASS,
    });
}

/// Shows the support reminder on the current desktop without taking the focus (modeless).
/// `owner` receives WM_SUPPORT_CLOSED. False when a dialog is already open or creation failed.
pub fn show_support(owner: HWND) -> bool {
    if is_open() {
        return false;
    }
    let ctx = Ctx { variant: String::new(), support: true };
    unsafe {
        init_controls();
        SUPPORT_OWNER.store(owner as usize, Relaxed);
        let dlg = i18n::create_dialog(IDD_ABOUT, std::ptr::null_mut(), Some(proc), &ctx as *const Ctx as LPARAM);
        if dlg.is_null() {
            OPEN.store(0, Relaxed);
            SUPPORT_HWND.store(0, Relaxed);
            return false;
        }
        ShowWindow(dlg, SW_SHOWNOACTIVATE);
    }
    true
}

unsafe fn set_text(dlg: HWND, id: i32, text: &str) {
    SetDlgItemTextW(dlg, id, wide(text).as_ptr());
}

unsafe fn item_size(dlg: HWND, id: i32) -> (i32, i32) {
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(GetDlgItem(dlg, id), &mut r);
    (r.right - r.left, r.bottom - r.top)
}

/// Decodes the QR resource and scales it into a square of `side` pixels (HALFTONE when
/// shrinking, nearest neighbour when enlarging); the full-size copy is freed at once.
unsafe fn scaled_qr(dlg: HWND, side: i32, res: u16) -> usize {
    let src = LoadImageW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance::<u16>(res as usize),
        IMAGE_BITMAP,
        0,
        0,
        LR_DEFAULTCOLOR,
    );
    if src.is_null() {
        return 0;
    }
    let mut bm: BITMAP = std::mem::zeroed();
    GetObjectW(src, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut _);
    let screen = GetDC(dlg);
    let (dc_src, dc_dst) = (CreateCompatibleDC(screen), CreateCompatibleDC(screen));
    let dst = CreateCompatibleBitmap(screen, side, side);
    let (old_src, old_dst) = (SelectObject(dc_src, src), SelectObject(dc_dst, dst));
    SetStretchBltMode(dc_dst, if side < bm.bmWidth { HALFTONE } else { COLORONCOLOR });
    SetBrushOrgEx(dc_dst, 0, 0, std::ptr::null_mut());
    StretchBlt(dc_dst, 0, 0, side, side, dc_src, 0, 0, bm.bmWidth, bm.bmHeight, SRCCOPY);
    SelectObject(dc_src, old_src);
    SelectObject(dc_dst, old_dst);
    DeleteDC(dc_src);
    DeleteDC(dc_dst);
    ReleaseDC(dlg, screen);
    DeleteObject(src);
    dst as usize
}

unsafe fn init(dlg: HWND, ctx: &Ctx) {
    OPEN.store(dlg as usize, Relaxed);
    if ctx.support {
        SUPPORT_HWND.store(dlg as usize, Relaxed);
    }
    // Application icon scaled to its control (so it follows the DPI).
    let (iw, ih) = item_size(dlg, ID_ICON);
    let side = iw.min(ih).max(16);
    let icon = LoadImageW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance::<u16>(1),
        IMAGE_ICON,
        side,
        side,
        LR_SHARED,
    );
    if !icon.is_null() {
        SendDlgItemMessageW(dlg, ID_ICON, STM_SETICON, icon as WPARAM, 0);
    }
    // Title in a larger bold font derived from the dialog font.
    let font = SendMessageW(dlg, WM_GETFONT, 0, 0);
    let mut lf: LOGFONTW = std::mem::zeroed();
    if font != 0 && GetObjectW(font as _, std::mem::size_of::<LOGFONTW>() as i32, &mut lf as *mut LOGFONTW as *mut _) != 0 {
        lf.lfHeight = lf.lfHeight * 3 / 2;
        lf.lfWeight = 700;
        let big = CreateFontIndirectW(&lf);
        if !big.is_null() {
            TITLE_FONT.store(big as usize, Relaxed);
            SendDlgItemMessageW(dlg, ID_TITLE, WM_SETFONT, big as WPARAM, 1);
        }
    }
    set_text(dlg, ID_VERSION, &version_label());
    set_text(dlg, ID_LINK, &tf(Key::AboutWeb, &[AUTHOR_URL, AUTHOR_URL.trim_start_matches("https://")]));
    set_text(dlg, ID_VARIANT, &ctx.variant);
    set_text(dlg, ID_ADDRESS, LIGHTNING_ADDRESS);
    CheckRadioButton(dlg, ID_PAY_REVOLUT, ID_PAY_ACCOUNT, ID_PAY_REVOLUT);
    show_pay(dlg, Pay::Revolut);
    if ctx.support {
        layout_support(dlg);
    } else {
        layout_about(dlg);
        SetFocus(GetDlgItem(dlg, IDOK));
    }
}

unsafe fn item_rect(dlg: HWND, id: i32) -> RECT {
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(GetDlgItem(dlg, id), &mut r);
    r
}

unsafe fn hide(dlg: HWND, id: i32) {
    ShowWindow(GetDlgItem(dlg, id), SW_HIDE);
}

unsafe fn move_up(dlg: HWND, id: i32, dy: i32) {
    let r = item_rect(dlg, id);
    let mut pt = POINT { x: r.left, y: r.top };
    ScreenToClient(dlg, &mut pt);
    SetWindowPos(GetDlgItem(dlg, id), std::ptr::null_mut(), pt.x, pt.y - dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
}

/// Makes the dialog `shrink` pixels shorter, keeping it centred.
unsafe fn shrink_dialog(dlg: HWND, shrink: i32) {
    if shrink <= 0 {
        return;
    }
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(dlg, &mut r);
    SetWindowPos(dlg, std::ptr::null_mut(), r.left, r.top + shrink / 2, r.right - r.left, r.bottom - r.top - shrink, SWP_NOZORDER | SWP_NOACTIVATE);
}

/// About: the reminder-only controls (tick box and note) are hidden and their room removed.
unsafe fn layout_about(dlg: HWND) {
    let shrink = item_rect(dlg, ID_NOTE).bottom - item_rect(dlg, IDOK).bottom;
    hide(dlg, ID_NEVER);
    hide(dlg, ID_NOTE);
    shrink_dialog(dlg, shrink);
}

/// Reminder: the invitation replaces the description and author lines; the payment block moves up.
unsafe fn layout_support(dlg: HWND) {
    SetWindowTextW(dlg, wide(t(Key::SupportTitle)).as_ptr());
    set_text(dlg, ID_SUMMARY, t(Key::SupportSummary));
    let dy = item_rect(dlg, ID_GROUP).top - item_rect(dlg, ID_AUTHOR).top;
    for id in [ID_AUTHOR, ID_LINK, ID_VARIANT] {
        hide(dlg, id);
    }
    for id in PAY_BLOCK {
        move_up(dlg, id, dy);
    }
    shrink_dialog(dlg, dy);
}

/// Bitcoin / Lightning mark drawn in code (own drawing, see assets/LICENSES.md): an orange disc
/// with a white bolt, painted 4x larger and shrunk with HALFTONE so the edges are smooth at any DPI.
unsafe fn draw_logo(dis: &DRAWITEMSTRUCT) {
    let dc = dis.hDC;
    let (w, h) = (dis.rcItem.right - dis.rcItem.left, dis.rcItem.bottom - dis.rcItem.top);
    let s = w.min(h);
    if s <= 0 {
        return;
    }
    let big = s * 4;
    let mem = CreateCompatibleDC(dc);
    let bmp = CreateCompatibleBitmap(dc, big, big);
    let old = SelectObject(mem, bmp);
    FillRect(mem, &RECT { left: 0, top: 0, right: big, bottom: big }, GetSysColorBrush(COLOR_BTNFACE));
    let orange = CreateSolidBrush(0x001A_93F7); // #F7931A as COLORREF (0x00BBGGRR)
    let white = CreateSolidBrush(0x00FF_FFFF);
    let old_pen = SelectObject(mem, GetStockObject(NULL_PEN));
    let old_brush = SelectObject(mem, orange);
    Ellipse(mem, 0, 0, big + 1, big + 1);
    SelectObject(mem, white);
    const BOLT: [(i32, i32); 7] = [(60, 10), (28, 55), (46, 55), (37, 90), (72, 43), (54, 43), (66, 10)];
    let pts = BOLT.map(|(x, y)| POINT { x: x * big / 100, y: y * big / 100 });
    Polygon(mem, pts.as_ptr(), pts.len() as i32);
    SelectObject(mem, old_pen);
    SelectObject(mem, old_brush);
    SetStretchBltMode(dc, HALFTONE);
    SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
    StretchBlt(dc, dis.rcItem.left + (w - s) / 2, dis.rcItem.top + (h - s) / 2, s, s, mem, 0, 0, big, big, SRCCOPY);
    SelectObject(mem, old);
    DeleteObject(bmp);
    DeleteObject(orange);
    DeleteObject(white);
    DeleteDC(mem);
}

/// Shows the chosen QR code and caption; the previous bitmap is freed at once.
unsafe fn show_pay(dlg: HWND, pay: Pay) {
    set_text(dlg, ID_CAPTION, &pay.caption());
    let (qw, qh) = item_size(dlg, ID_QR);
    let bmp = scaled_qr(dlg, qw.min(qh).max(1), pay.bitmap_id());
    // The static control does not own its bitmap: detach (null) and delete the old one ourselves.
    SendDlgItemMessageW(dlg, ID_QR, STM_SETIMAGE, IMAGE_BITMAP as WPARAM, bmp as LPARAM);
    let old = QR_BITMAP.swap(bmp, Relaxed);
    if old != 0 {
        DeleteObject(old as _);
    }
}

/// Copies `text` to the clipboard as Unicode text; false when the clipboard is busy.
unsafe fn copy_to_clipboard(dlg: HWND, text: &str) -> bool {
    let w = wide(text);
    if OpenClipboard(dlg) == 0 {
        return false;
    }
    EmptyClipboard();
    let h = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
    let mut ok = false;
    if !h.is_null() {
        let p = GlobalLock(h) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            GlobalUnlock(h);
            ok = !SetClipboardData(CF_UNICODETEXT, h as _).is_null();
        }
        if !ok {
            GlobalFree(h); // ownership passes to the system only on success
        }
    }
    CloseClipboard();
    ok
}

/// True when some program is registered for the lightning: scheme.
fn lightning_handler_registered() -> bool {
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let ok = RegOpenKeyExW(HKEY_CLASSES_ROOT, wide("lightning\\shell\\open\\command").as_ptr(), 0, KEY_READ, &mut key) == 0;
        if ok {
            RegCloseKey(key);
        }
        ok
    }
}

/// ShellExecuteEx without any system error window (SEE_MASK_FLAG_NO_UI); true on success.
unsafe fn open_uri(dlg: HWND, uri: &str) -> bool {
    let verb = wide("open");
    let file = wide(uri);
    let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
    sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = SEE_MASK_FLAG_NO_UI;
    sei.hwnd = dlg;
    sei.lpVerb = verb.as_ptr();
    sei.lpFile = file.as_ptr();
    sei.nShow = SW_SHOWNORMAL;
    ShellExecuteExW(&mut sei) != 0
}

unsafe fn on_command(dlg: HWND, id: i32, code: u32) {
    match id {
        ID_PAY_REVOLUT | ID_PAY_ACCOUNT if code == BN_CLICKED => {
            show_pay(dlg, if id == ID_PAY_REVOLUT { Pay::Revolut } else { Pay::Account });
        }
        x if x == IDOK || x == IDCANCEL => {
            if is_support(dlg) {
                let never = IsDlgButtonChecked(dlg, ID_NEVER) == BST_CHECKED;
                PostMessageW(SUPPORT_OWNER.load(Relaxed) as HWND, WM_SUPPORT_CLOSED, never as WPARAM, 0);
                DestroyWindow(dlg);
            } else {
                EndDialog(dlg, x as isize);
            }
        }
        ID_COPY => {
            let msg = if copy_to_clipboard(dlg, LIGHTNING_ADDRESS) {
                t(Key::CopyDone)
            } else {
                t(Key::CopyFailed)
            };
            set_text(dlg, ID_STATUS, msg);
        }
        ID_WALLET => {
            // The address is copied first, so the user can always paste it into any wallet.
            let copied = copy_to_clipboard(dlg, LIGHTNING_ADDRESS);
            let opened = lightning_handler_registered() && open_uri(dlg, &lightning_uri());
            let msg = match (opened, copied) {
                (true, true) => t(Key::WalletOpenedCopied),
                (true, false) => t(Key::WalletOpened),
                (false, true) => t(Key::WalletMissingCopied),
                (false, false) => t(Key::WalletMissing),
            };
            set_text(dlg, ID_STATUS, msg);
        }
        _ => {}
    }
}

unsafe extern "system" fn proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            init(dlg, &*(lp as *const Ctx));
            0 // the About dialog already focused the close button; the reminder takes no focus
        }
        WM_COMMAND => {
            on_command(dlg, (wp & 0xffff) as i32, ((wp >> 16) & 0xffff) as u32);
            0
        }
        WM_DRAWITEM => {
            let dis = &*(lp as *const DRAWITEMSTRUCT);
            if dis.CtlID == ID_LOGO as u32 {
                draw_logo(dis);
            }
            1
        }
        WM_NOTIFY => {
            let nm = &*(lp as *const NMHDR);
            if nm.idFrom == ID_LINK as usize && (nm.code == NM_CLICK || nm.code == NM_RETURN) {
                // Fixed address (never the href of the notification); a failure is silently ignored.
                ShellExecuteW(
                    dlg,
                    wide("open").as_ptr(),
                    wide(AUTHOR_URL).as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                );
            }
            0
        }
        WM_DESTROY => {
            if is_support(dlg) {
                SUPPORT_HWND.store(0, Relaxed);
                OPEN.store(0, Relaxed);
            }
            // The QR control does not own its bitmap, the title font is ours.
            let bmp = QR_BITMAP.swap(0, Relaxed);
            if bmp != 0 {
                SendDlgItemMessageW(dlg, ID_QR, STM_SETIMAGE, IMAGE_BITMAP as WPARAM, 0);
                DeleteObject(bmp as _);
            }
            let font = TITLE_FONT.swap(0, Relaxed);
            if font != 0 {
                SendDlgItemMessageW(dlg, ID_TITLE, WM_SETFONT, 0, 0);
                DeleteObject(font as _);
            }
            0
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightning_uri_uses_the_address() {
        assert_eq!(lightning_uri(), "lightning:premek.v@bitlifi.com");
    }

    #[test]
    fn version_comes_from_cargo() {
        let v = env!("CARGO_PKG_VERSION");
        assert!(version_label().contains(v));
        assert!(!version_label().contains('{'));
        assert!(v.split('.').count() >= 3 && v.split('.').all(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit())));
    }

    #[test]
    fn variant_label_strips_backend_prefix() {
        assert!(variant_label(Some("internal:win11-24h2")).ends_with("win11-24h2"));
        assert!(!variant_label(Some("internal:win11-24h2")).contains("internal:"));
        assert!(variant_label(Some("other")).ends_with("other"));
        assert_ne!(variant_label(None), variant_label(Some("other")));
    }

    #[test]
    fn pay_choices_map_to_distinct_bitmaps_and_captions() {
        assert_ne!(Pay::Revolut.bitmap_id(), Pay::Account.bitmap_id());
        assert!(Pay::Revolut.caption().contains(REVOLUT_NAME));
        assert!(!Pay::Account.caption().contains(REVOLUT_NAME));
        assert!(REVOLUT_NAME.starts_with('@'));
    }

    #[test]
    fn author_url_is_https() {
        assert!(AUTHOR_URL.starts_with("https://"));
    }

    #[test]
    fn qr_bitmap_assets_are_valid_rle8_bmps() {
        for name in ["donate-qr.bmp", "donate-revolut.bmp"] {
            check_bmp(name);
        }
    }

    fn check_bmp(name: &str) {
        let p = format!("{}/assets/{name}", env!("CARGO_MANIFEST_DIR"));
        let b = std::fs::read(&p).expect("QR bitmap missing (run tools/make-qr-bmp.py)");
        assert_eq!(&b[..2], b"BM");
        let le = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(le(18), le(22), "square image"); // width == height
        assert_eq!(u16::from_le_bytes([b[28], b[29]]), 8); // bits per pixel
        assert_eq!(le(30), 1); // BI_RLE8
        assert!(b.len() < 120_000, "keep the exe small");
        assert_eq!(le(2) as usize, b.len());
    }
}
