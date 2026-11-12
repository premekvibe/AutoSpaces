//! "O aplikaci" dialog (plan 12.4/12.6): template 104 in assets/autospaces.rc, created only on
//! request. The donate QR code is an RLE bitmap resource that is decoded and scaled when the
//! dialog opens and freed when it closes (nothing stays in memory while idle).

use crate::tray::wide;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{GlobalFree, HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, DeleteDC, DeleteObject, GetDC, GetObjectW,
    ReleaseDC, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt, BITMAP, COLORONCOLOR, HALFTONE, LOGFONTW,
    SRCCOPY,
};
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ};
use windows_sys::Win32::UI::Controls::{InitCommonControlsEx, ICC_LINK_CLASS, INITCOMMONCONTROLSEX, NMHDR, NM_CLICK, NM_RETURN};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, ShellExecuteW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DialogBoxParamW, EndDialog, GetClientRect, GetDlgItem, LoadImageW, SendDlgItemMessageW, SendMessageW,
    SetDlgItemTextW, SetForegroundWindow, IDCANCEL, IDOK, IMAGE_BITMAP, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED,
    STM_SETICON, STM_SETIMAGE, SW_SHOWNORMAL, WM_COMMAND, WM_DESTROY, WM_GETFONT, WM_INITDIALOG, WM_NOTIFY, WM_SETFONT,
};

const IDD_ABOUT: u16 = 104;
const IDB_QR: u16 = 20;
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
const CF_UNICODETEXT: u32 = 13;

pub const AUTHOR_URL: &str = "https://www.premysl-vavrousek.cz";
pub const LIGHTNING_ADDRESS: &str = "premek.v@bitlifi.com";

/// Handle of the open dialog (0 = none): a second request just brings it forward.
static OPEN: AtomicUsize = AtomicUsize::new(0);
/// Resources owned by the open dialog, released in WM_DESTROY.
static TITLE_FONT: AtomicUsize = AtomicUsize::new(0);
static QR_BITMAP: AtomicUsize = AtomicUsize::new(0);

pub fn lightning_uri() -> String {
    format!("lightning:{LIGHTNING_ADDRESS}")
}

/// "Verze 0.1.0" (version from Cargo.toml, the single source of truth).
pub fn version_label() -> String {
    format!("Verze {}", env!("CARGO_PKG_VERSION"))
}

/// `backend` is the name reported by the desktop backend (e.g. "internal:win11-24h2"), if any.
pub fn variant_label(backend: Option<&str>) -> String {
    match backend {
        Some(b) => format!("Varianta Windows: {}", b.strip_prefix("internal:").unwrap_or(b)),
        None => "Varianta Windows: nebyla rozpoznána, aplikace je neaktivní".to_string(),
    }
}

/// Opens the modal dialog (no-op, except bringing it forward, when already open).
pub fn show(backend: Option<&str>) {
    let prev = OPEN.load(Relaxed);
    if prev != 0 {
        unsafe { SetForegroundWindow(prev as HWND) };
        return;
    }
    let variant = variant_label(backend);
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LINK_CLASS,
        });
        DialogBoxParamW(
            GetModuleHandleW(std::ptr::null()),
            std::ptr::without_provenance::<u16>(IDD_ABOUT as usize),
            std::ptr::null_mut(),
            Some(proc),
            &variant as *const String as LPARAM,
        );
    }
    OPEN.store(0, Relaxed);
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
unsafe fn scaled_qr(dlg: HWND, side: i32) -> usize {
    let src = LoadImageW(
        GetModuleHandleW(std::ptr::null()),
        std::ptr::without_provenance::<u16>(IDB_QR as usize),
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

unsafe fn init(dlg: HWND, variant: &str) {
    OPEN.store(dlg as usize, Relaxed);
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
    set_text(dlg, ID_LINK, &format!("Web: <a href=\"{AUTHOR_URL}\">{}</a>", AUTHOR_URL.trim_start_matches("https://")));
    set_text(dlg, ID_VARIANT, variant);
    set_text(dlg, ID_ADDRESS, LIGHTNING_ADDRESS);
    let (qw, qh) = item_size(dlg, ID_QR);
    let bmp = scaled_qr(dlg, qw.min(qh).max(1));
    if bmp != 0 {
        QR_BITMAP.store(bmp, Relaxed);
        SendDlgItemMessageW(dlg, ID_QR, STM_SETIMAGE, IMAGE_BITMAP as WPARAM, bmp as LPARAM);
    }
    SetFocus(GetDlgItem(dlg, IDOK));
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

unsafe fn on_command(dlg: HWND, id: i32) {
    match id {
        x if x == IDOK || x == IDCANCEL => {
            EndDialog(dlg, x as isize);
        }
        ID_COPY => {
            let msg = if copy_to_clipboard(dlg, LIGHTNING_ADDRESS) {
                "Adresa je zkopírována do schránky."
            } else {
                "Schránku se nepodařilo otevřít."
            };
            set_text(dlg, ID_STATUS, msg);
        }
        ID_WALLET => {
            // The address is copied first, so the user can always paste it into any wallet.
            let copied = copy_to_clipboard(dlg, LIGHTNING_ADDRESS);
            let opened = lightning_handler_registered() && open_uri(dlg, &lightning_uri());
            let msg = match (opened, copied) {
                (true, true) => "Adresa je zkopírována do schránky, peněženka se otevírá.",
                (true, false) => "Peněženka se otevírá.",
                (false, true) => "Žádná peněženka není nastavená pro odkazy lightning:. Adresa je zkopírována do schránky.",
                (false, false) => "Žádná peněženka není nastavená pro odkazy lightning:.",
            };
            set_text(dlg, ID_STATUS, msg);
        }
        _ => {}
    }
}

unsafe extern "system" fn proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let variant = &*(lp as *const String);
            init(dlg, variant);
            0 // focus already set to the close button
        }
        WM_COMMAND => {
            on_command(dlg, (wp & 0xffff) as i32);
            0
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
        assert_eq!(version_label(), format!("Verze {v}"));
        assert!(v.split('.').count() >= 3 && v.split('.').all(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit())));
    }

    #[test]
    fn variant_label_strips_backend_prefix() {
        assert_eq!(variant_label(Some("internal:win11-24h2")), "Varianta Windows: win11-24h2");
        assert_eq!(variant_label(Some("other")), "Varianta Windows: other");
        assert!(variant_label(None).contains("neaktivní"));
    }

    #[test]
    fn author_url_is_https() {
        assert!(AUTHOR_URL.starts_with("https://"));
    }

    #[test]
    fn qr_bitmap_asset_is_a_valid_rle8_bmp() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/donate-qr.bmp");
        let b = std::fs::read(p).expect("assets/donate-qr.bmp missing (run tools/make-qr-bmp.py)");
        assert_eq!(&b[..2], b"BM");
        let le = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(le(18), le(22), "square image"); // width == height
        assert_eq!(u16::from_le_bytes([b[28], b[29]]), 8); // bits per pixel
        assert_eq!(le(30), 1); // BI_RLE8
        assert!(b.len() < 120_000, "keep the exe small");
        assert_eq!(le(2) as usize, b.len());
    }
}
