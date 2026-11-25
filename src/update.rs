//! Update check and installation (plan 12.15), the Windows side: HTTPS through WinHTTP (system DLL),
//! SHA-256 through BCrypt, how the app is installed (exe folder + the installer's uninstall entries),
//! starting the installer, `update.json`. The decisions are pure functions in `core::update`.
//!
//! Only GitHub is contacted: `API_LATEST` with the header `User-Agent: AutoSpaces/<version>` (no
//! cookies, no query, nothing about the user or the system), and, after the user confirmed, the
//! installer asset with its redirect to `*.githubusercontent.com`. Every hop must be https on an
//! allowed host; redirects are followed by this code, not by WinHTTP.

use crate::core::update::{self as up, InstallKind, Release};
use crate::tray::wide;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM};
use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
    WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_CONTENT_LENGTH,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData, BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE,
};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL, WM_APP};

/// Posted to the main window when a check is over; lParam = `Box<Checked>` (see `take`).
pub const WM_UPDATE_CHECKED: u32 = WM_APP + 50;
/// The installer's AppId (installer/autospaces.iss) as Inno Setup names its uninstall key.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{30AB1B41-D258-4DAA-A410-484D902466A8}_is1";

/// A check is running on its thread.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Outcome of one check, handed to the UI thread.
pub struct Checked {
    pub manual: bool,
    /// Ok(None) = no usable release (draft, pre-release, odd tag); Err = network or format problem.
    pub result: Result<Option<Release>, String>,
}

/// Starts a check on its own thread; false when one is already running (or no thread could start).
pub fn start_check(main: HWND, manual: bool) -> bool {
    if RUNNING.swap(true, Relaxed) {
        return false;
    }
    let target = main as usize;
    let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
        let msg = Box::into_raw(Box::new(Checked { manual, result: fetch_latest() }));
        if unsafe { PostMessageW(target as HWND, WM_UPDATE_CHECKED, 0, msg as LPARAM) } == 0 {
            // The app is gone (or its queue is full): nobody will take the result.
            drop(unsafe { Box::from_raw(msg) });
            RUNNING.store(false, Relaxed);
        }
    });
    if spawned.is_err() {
        RUNNING.store(false, Relaxed);
        return false;
    }
    true
}

/// Takes the result posted with `WM_UPDATE_CHECKED`.
///
/// # Safety
/// `lp` must be the lParam of a `WM_UPDATE_CHECKED` message, taken once.
pub unsafe fn take(lp: LPARAM) -> Checked {
    RUNNING.store(false, Relaxed);
    *Box::from_raw(lp as *mut Checked)
}

/// Asks GitHub for the latest release.
fn fetch_latest() -> Result<Option<Release>, String> {
    // Debug builds: `AUTOSPACES_UPDATE_JSON=<file>` answers from a file (no network), for trying the UI.
    #[cfg(debug_assertions)]
    if let Some(f) = std::env::var_os("AUTOSPACES_UPDATE_JSON") {
        let text = std::fs::read_to_string(f).map_err(|e| format!("AUTOSPACES_UPDATE_JSON: {e}"))?;
        return up::parse_release(&text);
    }
    let mut body = Vec::new();
    get(up::API_LATEST, true, &mut |chunk, _| {
        if body.len() + chunk.len() > up::MAX_API_BYTES {
            return Err("answer too large".into());
        }
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    let text = String::from_utf8(body).map_err(|_| "answer is not UTF-8".to_string())?;
    up::parse_release(&text)
}

fn user_agent() -> String {
    format!("AutoSpaces/{}", env!("CARGO_PKG_VERSION"))
}

/// A WinHTTP handle closed on drop.
struct Inet(*mut core::ffi::c_void);

impl Drop for Inet {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> String {
    format!("{what} failed (error {})", unsafe { GetLastError() })
}

unsafe fn set_u32(h: &Inet, option: u32, value: u32) -> bool {
    WinHttpSetOption(h.0, option, &value as *const u32 as *const _, 4) != 0
}

unsafe fn query_number(req: &Inet, what: u32) -> Option<u32> {
    let mut v: u32 = 0;
    let mut len: u32 = 4;
    let ok = WinHttpQueryHeaders(req.0, what | WINHTTP_QUERY_FLAG_NUMBER, std::ptr::null(), &mut v as *mut u32 as *mut _, &mut len, std::ptr::null_mut());
    (ok != 0).then_some(v)
}

unsafe fn query_text(req: &Inet, what: u32) -> Option<String> {
    let mut len: u32 = 0;
    WinHttpQueryHeaders(req.0, what, std::ptr::null(), std::ptr::null_mut(), &mut len, std::ptr::null_mut());
    if len == 0 || len > 64 * 1024 {
        return None;
    }
    let mut buf = vec![0u16; len as usize / 2 + 1];
    if WinHttpQueryHeaders(req.0, what, std::ptr::null(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut()) == 0 {
        return None;
    }
    buf.truncate(len as usize / 2);
    Some(String::from_utf16_lossy(&buf))
}

/// HTTPS GET of `url`; the body goes to `sink` in pieces (with the announced length, if any).
/// `api` adds the GitHub API headers. `sink` may stop the transfer by returning Err.
/// Receives the body in pieces with the announced length; Err stops the transfer.
type Sink<'a> = &'a mut dyn FnMut(&[u8], Option<u64>) -> Result<(), String>;

fn get(url: &str, api: bool, sink: Sink) -> Result<(), String> {
    let (mut host, mut path) = up::parse_https(url).ok_or("not an https address")?;
    if !up::allowed_host(&host) {
        return Err(format!("host {host} is not allowed"));
    }
    unsafe {
        let session = Inet(WinHttpOpen(wide(&user_agent()).as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        // TLS 1.2 or 1.3 only (1.3 is unknown to older Windows 10: then 1.2 alone).
        if !set_u32(&session, WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3) {
            set_u32(&session, WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2);
        }
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 30_000);
        let headers = wide(if api { "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n" } else { "" });
        for _ in 0..=up::MAX_REDIRECTS {
            let connect = Inet(WinHttpConnect(session.0, wide(&host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0));
            if connect.0.is_null() {
                return Err(last_error("WinHttpConnect"));
            }
            let req = Inet(WinHttpOpenRequest(
                connect.0,
                wide("GET").as_ptr(),
                wide(&path).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ));
            if req.0.is_null() {
                return Err(last_error("WinHttpOpenRequest"));
            }
            // Redirects are checked here (allowed hosts), never followed blindly; no cookies are kept.
            set_u32(&req, WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER);
            set_u32(&req, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_COOKIES);
            let (hp, hl) = if headers.len() > 1 { (headers.as_ptr(), (headers.len() - 1) as u32) } else { (std::ptr::null(), 0) };
            if WinHttpSendRequest(req.0, hp, hl, std::ptr::null(), 0, 0, 0) == 0 {
                return Err(last_error(&format!("request to {host}")));
            }
            if WinHttpReceiveResponse(req.0, std::ptr::null_mut()) == 0 {
                return Err(last_error(&format!("answer from {host}")));
            }
            let status = query_number(&req, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
            match status {
                301 | 302 | 303 | 307 | 308 => {
                    let loc = query_text(&req, WINHTTP_QUERY_LOCATION).ok_or("redirect without a location")?;
                    let (h, p) = up::redirect_target(&host, &loc).ok_or_else(|| format!("redirect from {host} to a host that is not allowed"))?;
                    host = h;
                    path = p;
                }
                200 => {
                    let total = query_number(&req, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        let mut got: u32 = 0;
                        if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got) == 0 {
                            return Err(last_error(&format!("reading from {host}")));
                        }
                        if got == 0 {
                            return Ok(());
                        }
                        sink(&buf[..got as usize], total)?;
                    }
                }
                s => return Err(format!("HTTP {s} from {host}")),
            }
        }
    }
    Err("too many redirects".into())
}

/// `%TEMP%\AutoSpaces-update`.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("AutoSpaces-update")
}

/// Removes what an earlier update left in `download_dir` (files still in use stay).
pub fn cleanup() {
    let _ = std::fs::remove_dir_all(download_dir());
}

/// Why a download did not give a verified installer.
#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    /// The file was complete but its SHA-256 differs from the release text (it has been deleted).
    Mismatch,
    Other(String),
}

/// A downloaded installer whose SHA-256 matched. The file stays open without write sharing until it
/// is dropped (after the installer has been started), so it cannot be swapped in between.
pub struct Verified {
    pub path: PathBuf,
    _lock: std::fs::File,
}

/// Downloads the installer of `release` into `download_dir` and checks it. `progress(done, total)`
/// is called while the bytes come; `cancel` stops the transfer.
pub fn download_verified(release: &Release, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Verified, DownloadError> {
    let (Some(url), Some(expected)) = (&release.installer_url, &release.sha256) else {
        return Err(DownloadError::Other("the release has no installer or no checksum".into()));
    };
    let dir = download_dir();
    cleanup();
    std::fs::create_dir_all(&dir).map_err(|e| DownloadError::Other(format!("{}: {e}", dir.display())))?;
    let path = dir.join(up::installer_name(release.version));
    let res = download_to(url, &path, cancel, progress);
    if let Err(e) = res {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    let lock = open_locked(&path).map_err(|e| DownloadError::Other(format!("{}: {e}", path.display())))?;
    let actual = sha256_hex(&mut &lock).map_err(DownloadError::Other)?;
    if !actual.eq_ignore_ascii_case(expected) {
        drop(lock);
        let _ = std::fs::remove_file(&path);
        return Err(DownloadError::Mismatch);
    }
    Ok(Verified { path, _lock: lock })
}

fn download_to(url: &str, path: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), DownloadError> {
    let mut file = std::fs::File::create(path).map_err(|e| DownloadError::Other(format!("{}: {e}", path.display())))?;
    let mut done: u64 = 0;
    let res = get(url, false, &mut |chunk, total| {
        if cancel.load(Relaxed) {
            return Err("cancelled".into());
        }
        done += chunk.len() as u64;
        if done > up::MAX_INSTALLER_BYTES || total.is_some_and(|t| t > up::MAX_INSTALLER_BYTES) {
            return Err("installer too large".into());
        }
        file.write_all(chunk).map_err(|e| e.to_string())?;
        progress(done, total);
        Ok(())
    });
    if cancel.load(Relaxed) {
        return Err(DownloadError::Cancelled);
    }
    res.map_err(DownloadError::Other)?;
    file.sync_all().map_err(|e| DownloadError::Other(e.to_string()))
}

/// Opens `path` for reading; others may read (and run) it, nobody may write, rename or delete it.
fn open_locked(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path)
}

/// SHA-256 (BCrypt, Windows' own implementation) of everything `r` gives, as lower-case hex.
pub fn sha256_hex(r: &mut dyn Read) -> Result<String, String> {
    unsafe {
        let mut h: BCRYPT_HASH_HANDLE = std::ptr::null_mut();
        if BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut h, std::ptr::null_mut(), 0, std::ptr::null(), 0, 0) != 0 {
            return Err("BCryptCreateHash failed".into());
        }
        let mut buf = vec![0u8; 64 * 1024];
        let mut out = [0u8; 32];
        let res = loop {
            match r.read(&mut buf) {
                Ok(0) => {
                    break if BCryptFinishHash(h, out.as_mut_ptr(), 32, 0) == 0 { Ok(()) } else { Err("BCryptFinishHash failed".to_string()) };
                }
                Ok(n) => {
                    if BCryptHashData(h, buf.as_ptr(), n as u32, 0) != 0 {
                        break Err("BCryptHashData failed".into());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => break Err(e.to_string()),
            }
        };
        BCryptDestroyHash(h);
        res.map(|()| out.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// `InstallLocation` of the installer's uninstall entry under `root`, if any.
fn install_location(root: HKEY) -> Option<String> {
    let key = wide(UNINSTALL_KEY);
    let name = wide("InstallLocation");
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
    let r = unsafe { RegGetValueW(root, key.as_ptr(), name.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut len) };
    if r != 0 {
        return None;
    }
    buf.truncate((len as usize / 2).saturating_sub(1));
    Some(String::from_utf16_lossy(&buf))
}

/// How this copy is installed (see `core::update::install_kind`).
pub fn install_kind() -> InstallKind {
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned())) else {
        return InstallKind::Portable;
    };
    let local_programs = std::env::var("LOCALAPPDATA").ok().map(|l| format!(r"{l}\Programs"));
    let pf: Vec<String> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|v| std::env::var(v).ok()).collect();
    let pf: Vec<&str> = pf.iter().map(String::as_str).collect();
    let cu = install_location(HKEY_CURRENT_USER);
    let lm = install_location(HKEY_LOCAL_MACHINE);
    up::install_kind(&dir, cu.as_deref(), lm.as_deref(), local_programs.as_deref(), &pf)
}

/// Starts the verified installer silently in the mode of the current installation. Per user it runs
/// without elevation; for all users the installer asks for elevation itself (UAC) – started from an
/// already elevated process it could not start AutoSpaces again as the original user.
pub fn launch_installer(v: &Verified, kind: InstallKind) -> Result<(), String> {
    let args = up::installer_args(kind).ok_or("this copy is not installed")?;
    let verb = wide("open");
    let file = wide(&v.path.to_string_lossy());
    let params = wide(args);
    let dir = wide(&v.path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default());
    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.lpDirectory = dir.as_ptr();
        sei.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut sei) == 0 {
            return Err(last_error("ShellExecuteEx"));
        }
    }
    Ok(())
}

/// `update.json` in the app's data folder (the skipped version).
pub fn load_notices(dir: Option<&Path>) -> up::Notices {
    dir.and_then(|d| std::fs::read_to_string(d.join("update.json")).ok()).map_or_else(Default::default, |t| up::Notices::from_json(&t))
}

pub fn save_notices(dir: Option<&Path>, n: &up::Notices) {
    if let Some(d) = dir {
        let tmp = d.join("update.json.tmp");
        if std::fs::write(&tmp, n.to_json()).is_ok() {
            let _ = std::fs::rename(&tmp, d.join("update.json"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BCrypt gives the standard SHA-256 test vectors (local computation, no network).
    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(sha256_hex(&mut &b""[..]).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(&mut &b"abc"[..]).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // Longer than one read buffer.
        let big = vec![b'a'; 1_000_000];
        assert_eq!(sha256_hex(&mut &big[..]).unwrap(), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn user_agent_names_only_the_app_and_its_version() {
        assert_eq!(user_agent(), format!("AutoSpaces/{}", env!("CARGO_PKG_VERSION")));
    }
}
