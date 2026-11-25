//! Update check (plan 12.15), the pure parts: versions, the GitHub release JSON, the SHA-256 line of
//! the release text, the hosts a download may come from, how the app is installed, what to tell the
//! user and when to check. Network, hashing, files and dialogs live in `src/update.rs` and
//! `src/update_dlg.rs`.

use crate::json::{parse_json, quote, Json};

/// The only address asked by the check (no query, no cookies, no data about the user).
pub const API_LATEST: &str = "https://api.github.com/repos/premekvibe/AutoSpaces/releases/latest";
/// Release pages of this repository; `html_url` must lie below it (it is opened in the browser).
pub const RELEASES_PAGE: &str = "https://github.com/premekvibe/AutoSpaces/releases/";
/// Installer assets of this repository; `browser_download_url` must lie below it.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/premekvibe/AutoSpaces/releases/download/";
/// Largest accepted API answer and installer.
pub const MAX_API_BYTES: usize = 1024 * 1024;
pub const MAX_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;
/// Redirects followed per request (GitHub needs one: github.com -> release-assets.githubusercontent.com).
pub const MAX_REDIRECTS: usize = 5;

/// `major.minor.patch`, compared numerically.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// `0.2.1` or `v0.2.1`. Anything else (pre-release `-beta`, build `+x`, four parts, leading
    /// zeros like `01`) is not a release version: None.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let mut parts = s.split('.');
        let mut num = || -> Option<u32> {
            let p = parts.next()?;
            if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) || (p.len() > 1 && p.starts_with('0')) {
                return None;
            }
            p.parse().ok()
        };
        let v = Version(num()?, num()?, num()?);
        if parts.next().is_some() {
            return None;
        }
        Some(v)
    }

    /// The version of this build (Cargo.toml).
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// File name of the installer of `v` (the asset the update downloads).
pub fn installer_name(v: Version) -> String {
    format!("AutoSpaces-Setup-{v}.exe")
}

/// What the check found about the latest release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// Release page ("What's new"), below `RELEASES_PAGE`.
    pub html_url: String,
    /// `browser_download_url` of `AutoSpaces-Setup-X.Y.Z.exe`, below `DOWNLOAD_PREFIX`; None = no such asset.
    pub installer_url: Option<String>,
    /// Lower-case hex SHA-256 of the installer from the release text; None = missing or ambiguous.
    pub sha256: Option<String>,
}

impl Release {
    /// Can it be downloaded and verified (asset and checksum present)?
    pub fn installable(&self) -> bool {
        self.installer_url.is_some() && self.sha256.is_some()
    }
}

/// Reads the answer of `API_LATEST`. Ok(None) = nothing to offer (draft, pre-release, a tag that is
/// not `vX.Y.Z`); Err = not the expected JSON or a page outside this repository.
pub fn parse_release(text: &str) -> Result<Option<Release>, String> {
    let j = parse_json(text)?;
    if !matches!(j, Json::Obj(_)) {
        return Err("release: not a JSON object".into());
    }
    let flag = |k: &str| matches!(j.get(k), Some(Json::Bool(true)));
    if flag("draft") || flag("prerelease") {
        return Ok(None);
    }
    let Some(version) = j.get("tag_name").and_then(Json::as_str).and_then(Version::parse) else {
        return Ok(None);
    };
    let html_url = j.get("html_url").and_then(Json::as_str).unwrap_or_default();
    if !html_url.starts_with(RELEASES_PAGE) || parse_https(html_url).is_none() {
        return Err("release: html_url is not a release page of this repository".into());
    }
    let name = installer_name(version);
    let installer_url = match j.get("assets") {
        Some(Json::Arr(assets)) => assets
            .iter()
            .filter(|a| a.get("name").and_then(Json::as_str) == Some(name.as_str()))
            .filter_map(|a| a.get("browser_download_url").and_then(Json::as_str))
            .find(|u| u.starts_with(DOWNLOAD_PREFIX) && u.ends_with(&format!("/{name}")) && parse_https(u).is_some())
            .map(str::to_string),
        _ => None,
    };
    let body = j.get("body").and_then(Json::as_str).unwrap_or_default();
    Ok(Some(Release { version, html_url: html_url.to_string(), installer_url, sha256: sha256_from_body(body, &name) }))
}

/// The checksum line of the release text (plan 12.15, required for every release):
/// ``SHA-256 (AutoSpaces-Setup-X.Y.Z.exe): `<64 hex digits>` `` (backticks optional, case of the hex
/// and of "SHA-256" free). None when no line names `file`, or when lines for it disagree.
pub fn sha256_from_body(body: &str, file: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in body.lines() {
        let l = line.trim().trim_start_matches(['-', '*', ' ']);
        let Some(rest) = l.get(..9).filter(|p| p.eq_ignore_ascii_case("SHA-256 (")).map(|_| &l[9..]) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(file).and_then(|r| r.strip_prefix("):")) else {
            continue;
        };
        let hex = rest.trim().trim_matches('`').trim();
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let hex = hex.to_ascii_lowercase();
        match &found {
            Some(h) if *h != hex => return None,
            _ => found = Some(hex),
        }
    }
    found
}

/// `https://host[:443]/path?query` -> (lower-case host, path with query, without the fragment).
/// Only https, no user info, no other port.
pub fn parse_https(url: &str) -> Option<(String, String)> {
    let rest = url.get(..8).filter(|p| p.eq_ignore_ascii_case("https://")).map(|_| &url[8..])?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((h, "443")) => h,
        Some(_) => return None,
        None => authority,
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
        return None;
    }
    let tail = tail.split('#').next().unwrap_or("");
    if tail.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let path = if tail.starts_with('/') { tail.to_string() } else { format!("/{tail}") };
    Some((host, path))
}

/// Hosts a request may go to: GitHub itself, its API and its download servers.
pub fn allowed_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "github.com" || h == "api.github.com" || (h.ends_with(".githubusercontent.com") && h.len() > ".githubusercontent.com".len())
}

/// Where a redirect (`Location` of a 3xx answer from `host`) leads, when it may be followed:
/// an absolute https address on an allowed host, or a path on the same host.
pub fn redirect_target(host: &str, location: &str) -> Option<(String, String)> {
    let loc = location.trim();
    let target = if loc.starts_with('/') && !loc.starts_with("//") {
        parse_https(&format!("https://{host}{loc}"))?
    } else {
        parse_https(loc)?
    };
    allowed_host(&target.0).then_some(target)
}

/// How this copy of AutoSpaces was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// By the installer for the current user (`%LOCALAPPDATA%\Programs`, HKCU): no administrator rights.
    PerUser,
    /// By the installer for all users (Program Files, HKLM): the installer asks for elevation itself.
    AllUsers,
    /// Not installed (a build in `target\`, a copied exe): the update only opens the release page.
    Portable,
}

/// `C:/x/Y\` -> `c:\x\y` for comparing folders.
fn norm_dir(p: &str) -> String {
    p.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// `dir` is `root` or lies below it.
fn under(dir: &str, root: &str) -> bool {
    let (d, r) = (norm_dir(dir), norm_dir(root));
    !r.is_empty() && (d == r || d.starts_with(&format!("{r}\\")))
}

/// Decides from the folder of the running exe. The installer's uninstall entries (`InstallLocation`
/// of `{30AB1B41-…}_is1` in HKCU / HKLM) win when they name exactly this folder; otherwise the
/// folder decides: below `%LOCALAPPDATA%\Programs` = per user, below Program Files = all users.
pub fn install_kind(exe_dir: &str, hkcu_location: Option<&str>, hklm_location: Option<&str>, local_programs: Option<&str>, program_files: &[&str]) -> InstallKind {
    let same = |loc: Option<&str>| loc.is_some_and(|l| !norm_dir(l).is_empty() && norm_dir(l) == norm_dir(exe_dir));
    if same(hkcu_location) {
        return InstallKind::PerUser;
    }
    if same(hklm_location) {
        return InstallKind::AllUsers;
    }
    if local_programs.is_some_and(|r| under(exe_dir, r)) {
        return InstallKind::PerUser;
    }
    if program_files.iter().any(|r| under(exe_dir, r)) {
        return InstallKind::AllUsers;
    }
    InstallKind::Portable
}

/// Command line of the Inno Setup installer for an update (None = not installed: no installer).
/// `/UPDATE` makes the silent installer start AutoSpaces again when it is done (installer/autospaces.iss).
pub fn installer_args(kind: InstallKind) -> Option<&'static str> {
    match kind {
        InstallKind::PerUser => Some("/SILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /UPDATE"),
        InstallKind::AllUsers => Some("/SILENT /SUPPRESSMSGBOXES /NORESTART /ALLUSERS /UPDATE"),
        InstallKind::Portable => None,
    }
}

/// What to do with the result of a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing to say (automatic check: up to date, skipped or already announced in this run).
    Nothing,
    /// Manual check, nothing newer: "You have the latest version".
    UpToDate,
    /// Automatic check, a newer version: the notification.
    Notify,
    /// Manual check, a newer version: the update dialog at once.
    Dialog,
}

/// Remembered across runs (`%LOCALAPPDATA%\AutoSpaces\update.json`): the skipped version; and, for
/// this run only, the version already announced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notices {
    pub skipped: Option<Version>,
    announced: Option<Version>,
}

impl Notices {
    /// `found` = version of the latest release (None = no usable release).
    pub fn verdict(&mut self, current: Version, found: Option<Version>, manual: bool) -> Verdict {
        let newer = found.filter(|v| *v > current);
        match (newer, manual) {
            (None, true) => Verdict::UpToDate,
            (None, false) => Verdict::Nothing,
            (Some(v), true) => {
                self.announced = Some(v);
                Verdict::Dialog
            }
            (Some(v), false) if self.skipped == Some(v) || self.announced == Some(v) => Verdict::Nothing,
            (Some(v), false) => {
                self.announced = Some(v);
                Verdict::Notify
            }
        }
    }

    /// "Skip this version": no automatic notice for `v` any more (a later version is announced).
    pub fn skip(&mut self, v: Version) {
        self.skipped = Some(v);
    }

    pub fn to_json(&self) -> String {
        match self.skipped {
            Some(v) => format!("{{\"skipVersion\":{}}}\n", quote(&v.to_string())),
            None => "{}\n".to_string(),
        }
    }

    /// Tolerant: anything unreadable = nothing skipped.
    pub fn from_json(text: &str) -> Notices {
        let skipped = parse_json(text).ok().and_then(|j| j.get("skipVersion").and_then(Json::as_str).and_then(Version::parse));
        Notices { skipped, announced: None }
    }
}

/// First automatic check this long after start (logon is not slowed down).
pub const FIRST_CHECK_SECS: u64 = 10 * 60;
/// Then once a day while the app runs.
pub const INTERVAL_SECS: u64 = 24 * 60 * 60;
/// A failed check (offline, GitHub not reachable) is tried again after an hour.
pub const RETRY_SECS: u64 = 60 * 60;
/// The timer never waits longer than this, so sleep and clock changes are noticed.
pub const MAX_TIMER_SECS: u64 = 60 * 60;

/// When the next automatic check is due (wall clock), for this run only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timing {
    next_at: Option<u64>,
}

impl Timing {
    pub const fn new() -> Timing {
        Timing { next_at: None }
    }

    /// Seconds until the next check (0 = now). `uptime` = seconds since the app started, `now` = unix time.
    pub fn wait_secs(&self, uptime: u64, now: u64) -> u64 {
        if uptime < FIRST_CHECK_SECS {
            return FIRST_CHECK_SECS - uptime;
        }
        match self.next_at {
            None => 0,
            // The clock was set back by more than an interval: check now rather than never.
            Some(t) if t > now + INTERVAL_SECS => 0,
            Some(t) => t.saturating_sub(now),
        }
    }

    /// A check finished (`ok` = GitHub answered, whatever the answer was).
    pub fn done(&mut self, now: u64, ok: bool) {
        self.next_at = Some(now + if ok { INTERVAL_SECS } else { RETRY_SECS });
    }

    /// Delay of the one-shot timer for `wait` seconds (at least 1 s, at most `MAX_TIMER_SECS`).
    pub fn timer_ms(wait: u64) -> u32 {
        (wait.clamp(1, MAX_TIMER_SECS) * 1000) as u32
    }
}

/// "45 %" when the size is known, else "1234 kB".
pub fn progress_text(done: u64, total: Option<u64>) -> String {
    match total {
        Some(t) if t > 0 => format!("{} %", (done.min(t) * 100 / t)),
        _ => format!("{} kB", done / 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed copy of the real answer for v0.2.0 (2026-11-25), with its checksum line.
    const SAMPLE: &str = r#"{"url":"https://api.github.com/repos/premekvibe/AutoSpaces/releases/457278471",
"html_url":"https://github.com/premekvibe/AutoSpaces/releases/tag/v0.2.0","id":457278471,
"author":{"login":"premekvibe","id":285717291,"site_admin":false},
"tag_name":"v0.2.0","target_commitish":"main","name":"AutoSpaces 0.2.0","draft":false,"immutable":false,"prerelease":false,
"created_at":"2026-11-24T20:05:00Z","assets":[{"url":"https://api.github.com/repos/premekvibe/AutoSpaces/releases/assets/1",
"id":1,"name":"AutoSpaces-Setup-0.2.0.exe","label":null,"content_type":"application/x-msdownload","state":"uploaded",
"size":2863486,"digest":"sha256:b212f76804de048b0ada8a4641be16fd211843f0b75aa420eb67450fa25dce35","download_count":7,
"browser_download_url":"https://github.com/premekvibe/AutoSpaces/releases/download/v0.2.0/AutoSpaces-Setup-0.2.0.exe"}],
"body":"**English**\r\n\r\nNew in 0.2.0: **carry a window between desktops.**\r\n- Press **\u2190/\u2192** \ud83d\ude00\r\n\r\n**\u010cesky**\r\n\r\nSHA-256 (AutoSpaces-Setup-0.2.0.exe): `b212f76804de048b0ada8a4641be16fd211843f0b75aa420eb67450fa25dce35`\r\n"}"#;

    #[test]
    fn versions_parse_strictly_and_compare_numerically() {
        assert_eq!(Version::parse("v0.2.1"), Some(Version(0, 2, 1)));
        assert_eq!(Version::parse("0.10.0"), Some(Version(0, 10, 0)));
        assert_eq!(Version::parse(" V1.2.3 "), Some(Version(1, 2, 3)));
        for bad in ["", "v", "1.2", "1.2.3.4", "1.2.3-beta", "1.2.3+build", "1.02.3", "a.b.c", "1..3", "-1.2.3", "1.2.x"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert!(Version(0, 2, 10) > Version(0, 2, 9));
        assert_eq!(Version(0, 3, 0).to_string(), "0.3.0");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn real_release_answer_is_read() {
        let r = parse_release(SAMPLE).unwrap().unwrap();
        assert_eq!(r.version, Version(0, 2, 0));
        assert_eq!(r.html_url, "https://github.com/premekvibe/AutoSpaces/releases/tag/v0.2.0");
        assert_eq!(
            r.installer_url.as_deref(),
            Some("https://github.com/premekvibe/AutoSpaces/releases/download/v0.2.0/AutoSpaces-Setup-0.2.0.exe")
        );
        assert_eq!(r.sha256.as_deref(), Some("b212f76804de048b0ada8a4641be16fd211843f0b75aa420eb67450fa25dce35"));
        assert!(r.installable());
    }

    #[test]
    fn drafts_prereleases_and_odd_tags_offer_nothing() {
        let set = |k: &str, v: &str| SAMPLE.replace(&format!("\"{k}\":false"), &format!("\"{k}\":{v}"));
        assert_eq!(parse_release(&set("draft", "true")), Ok(None));
        assert_eq!(parse_release(&set("prerelease", "true")), Ok(None));
        assert_eq!(parse_release(&SAMPLE.replace("\"v0.2.0\"", "\"v0.3.0-rc1\"")), Ok(None));
        assert_eq!(parse_release(&SAMPLE.replace("\"tag_name\":\"v0.2.0\",", "")), Ok(None));
        assert!(parse_release("not json").is_err());
        assert!(parse_release("[1]").is_err());
        // The page that would be opened must belong to this repository.
        assert!(parse_release(&SAMPLE.replace("github.com/premekvibe/AutoSpaces/releases/tag", "evil.example/releases/tag")).is_err());
    }

    #[test]
    fn installer_asset_must_match_the_version_and_the_repository() {
        // The asset of another version is not taken.
        let r = parse_release(&SAMPLE.replace("\"v0.2.0\"", "\"v0.2.5\"")).unwrap().unwrap();
        assert_eq!(r.installer_url, None);
        assert_eq!(r.sha256, None);
        assert!(!r.installable());
        // A download address outside the repository is ignored.
        let other = SAMPLE.replace("https://github.com/premekvibe/AutoSpaces/releases/download/", "https://example.com/x/");
        assert_eq!(parse_release(&other).unwrap().unwrap().installer_url, None);
        let http = SAMPLE.replace("\"browser_download_url\":\"https:", "\"browser_download_url\":\"http:");
        assert_eq!(parse_release(&http).unwrap().unwrap().installer_url, None);
        // No assets at all.
        let none = SAMPLE.replace("\"assets\":[", "\"assets_old\":[");
        assert_eq!(parse_release(&none).unwrap().unwrap().installer_url, None);
    }

    #[test]
    fn checksum_line_is_found_only_for_the_named_file() {
        let h = "b212f76804de048b0ada8a4641be16fd211843f0b75aa420eb67450fa25dce35";
        let f = "AutoSpaces-Setup-0.2.0.exe";
        assert_eq!(sha256_from_body(&format!("x\nSHA-256 ({f}): `{h}`\n"), f).as_deref(), Some(h));
        assert_eq!(sha256_from_body(&format!("- sha-256 ({f}): {}\r\n", h.to_uppercase()), f).as_deref(), Some(h));
        assert_eq!(sha256_from_body(&format!("SHA-256 ({f}):`{h}`"), f).as_deref(), Some(h));
        // Another file, a short or broken hash, no line at all.
        assert_eq!(sha256_from_body(&format!("SHA-256 (AutoSpaces-Setup-0.1.0.exe): `{h}`"), f), None);
        assert_eq!(sha256_from_body(&format!("SHA-256 ({f}): `{}`", &h[..63]), f), None);
        assert_eq!(sha256_from_body(&format!("SHA-256 ({f}): `{}g`", &h[..63]), f), None);
        assert_eq!(sha256_from_body("SHA-1 (x): abc", f), None);
        assert_eq!(sha256_from_body("", f), None);
        // Two different values for the same file: ambiguous, nothing is trusted.
        let other = h.replace('b', "c");
        assert_eq!(sha256_from_body(&format!("SHA-256 ({f}): `{h}`\nSHA-256 ({f}): `{other}`"), f), None);
        // The same value twice is fine.
        assert_eq!(sha256_from_body(&format!("SHA-256 ({f}): `{h}`\nSHA-256 ({f}): `{h}`"), f).as_deref(), Some(h));
    }

    #[test]
    fn only_https_on_github_hosts_is_followed() {
        assert_eq!(
            parse_https("https://api.github.com/repos/x?y=1#frag"),
            Some(("api.github.com".into(), "/repos/x?y=1".into()))
        );
        assert_eq!(parse_https("HTTPS://GitHub.com:443"), Some(("github.com".into(), "/".into())));
        for bad in ["http://github.com/x", "https://user@github.com/x", "https://github.com:8443/x", "ftp://github.com", "https:///x", "https://git hub.com/", "https://github.com/a b"] {
            assert_eq!(parse_https(bad), None, "{bad}");
        }
        for ok in ["github.com", "api.github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"] {
            assert!(allowed_host(ok), "{ok}");
        }
        for bad in ["githubusercontent.com", "evilgithub.com", "github.com.evil.example", "githubusercontent.com.evil.example", "example.com", "gist.github.io"] {
            assert!(!allowed_host(bad), "{bad}");
        }
        let gh = "github.com";
        assert_eq!(
            redirect_target(gh, "https://release-assets.githubusercontent.com/github-production-release-asset/1/2?sp=r&sig=x"),
            Some(("release-assets.githubusercontent.com".into(), "/github-production-release-asset/1/2?sp=r&sig=x".into()))
        );
        assert_eq!(redirect_target(gh, "/premekvibe/AutoSpaces/x"), Some((gh.into(), "/premekvibe/AutoSpaces/x".into())));
        assert_eq!(redirect_target(gh, "http://objects.githubusercontent.com/x"), None);
        assert_eq!(redirect_target(gh, "https://evil.example/x"), None);
        assert_eq!(redirect_target(gh, "//evil.example/x"), None);
        assert_eq!(redirect_target(gh, ""), None);
    }

    #[test]
    fn install_kind_from_registry_and_folder() {
        let local = r"C:\Users\Jan\AppData\Local\Programs";
        let pf = [r"C:\Program Files", r"C:\Program Files (x86)"];
        let k = |dir: &str, cu: Option<&str>, lm: Option<&str>| install_kind(dir, cu, lm, Some(local), &pf);
        // The installer's own records win when they name this folder.
        assert_eq!(k(r"D:\Apps\AutoSpaces", Some(r"D:\Apps\AutoSpaces\"), None), InstallKind::PerUser);
        assert_eq!(k(r"D:\Apps\AutoSpaces", None, Some(r"d:/apps/autospaces")), InstallKind::AllUsers);
        // Records of another folder do not count; the folder decides.
        assert_eq!(k(r"C:\Users\Jan\AppData\Local\Programs\AutoSpaces", None, Some(r"C:\Program Files\AutoSpaces\")), InstallKind::PerUser);
        assert_eq!(k(r"C:\Program Files\AutoSpaces", Some(r"C:\Other\"), None), InstallKind::AllUsers);
        assert_eq!(k(r"c:\program files (x86)\AutoSpaces", None, None), InstallKind::AllUsers);
        // Builds and copies elsewhere are not installed.
        assert_eq!(k(r"C:\Users\Jan\Documents\GitHub\AutoSpaces\target\release", None, None), InstallKind::Portable);
        assert_eq!(k(r"C:\Program FilesX\AutoSpaces", None, None), InstallKind::Portable);
        assert_eq!(k(r"C:\Users\Jan\AppData\Local\ProgramsOld\AutoSpaces", None, None), InstallKind::Portable);
        assert_eq!(install_kind(r"C:\x", Some(""), Some(""), None, &[]), InstallKind::Portable);
        // Command lines: silent, matching mode, start again afterwards; none for a copy.
        assert_eq!(installer_args(InstallKind::PerUser), Some("/SILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /UPDATE"));
        assert!(installer_args(InstallKind::AllUsers).unwrap().contains("/ALLUSERS"));
        assert_eq!(installer_args(InstallKind::Portable), None);
    }

    #[test]
    fn a_version_is_announced_once_per_run_and_never_when_skipped() {
        let cur = Version(0, 2, 1);
        let mut n = Notices::default();
        assert_eq!(n.verdict(cur, Some(Version(0, 2, 1)), false), Verdict::Nothing);
        assert_eq!(n.verdict(cur, Some(Version(0, 2, 0)), false), Verdict::Nothing);
        assert_eq!(n.verdict(cur, None, false), Verdict::Nothing);
        assert_eq!(n.verdict(cur, Some(Version(0, 2, 1)), true), Verdict::UpToDate);
        assert_eq!(n.verdict(cur, None, true), Verdict::UpToDate);
        // Newer: announced once, the next daily check is quiet.
        assert_eq!(n.verdict(cur, Some(Version(0, 3, 0)), false), Verdict::Notify);
        assert_eq!(n.verdict(cur, Some(Version(0, 3, 0)), false), Verdict::Nothing);
        // A manual check always shows the dialog.
        assert_eq!(n.verdict(cur, Some(Version(0, 3, 0)), true), Verdict::Dialog);
        // An even newer one is announced again.
        assert_eq!(n.verdict(cur, Some(Version(0, 3, 1)), false), Verdict::Notify);
        // Skipped: never announced automatically, also in later runs (update.json), but shown on request.
        n.skip(Version(0, 4, 0));
        let mut later = Notices::from_json(&n.to_json());
        assert_eq!(later.skipped, Some(Version(0, 4, 0)));
        assert_eq!(later.verdict(cur, Some(Version(0, 4, 0)), false), Verdict::Nothing);
        assert_eq!(later.verdict(cur, Some(Version(0, 4, 0)), true), Verdict::Dialog);
        assert_eq!(later.verdict(cur, Some(Version(0, 4, 1)), false), Verdict::Notify);
        // A manual dialog counts as the announcement of this run.
        let mut m = Notices::default();
        assert_eq!(m.verdict(cur, Some(Version(0, 5, 0)), true), Verdict::Dialog);
        assert_eq!(m.verdict(cur, Some(Version(0, 5, 0)), false), Verdict::Nothing);
        // update.json is read tolerantly.
        for bad in ["", "x", "[]", "{\"skipVersion\":3}", "{\"skipVersion\":\"1.2\"}"] {
            assert_eq!(Notices::from_json(bad).skipped, None, "{bad}");
        }
        assert_eq!(Notices::default().to_json(), "{}\n");
    }

    #[test]
    fn first_check_after_ten_minutes_then_daily_and_retry_after_failure() {
        let mut t = Timing::default();
        let now = 1_800_000_000;
        // Not before 10 minutes of uptime.
        assert_eq!(t.wait_secs(0, now), 600);
        assert_eq!(t.wait_secs(599, now), 1);
        assert_eq!(t.wait_secs(600, now), 0);
        // Answered: the next one a day later.
        t.done(now, true);
        assert_eq!(t.wait_secs(700, now), INTERVAL_SECS);
        assert_eq!(t.wait_secs(700, now + INTERVAL_SECS - 5), 5);
        assert_eq!(t.wait_secs(700, now + INTERVAL_SECS + 3600), 0);
        // Failed: again in an hour.
        t.done(now, false);
        assert_eq!(t.wait_secs(700, now), RETRY_SECS);
        // Clock set back by a week: check now instead of waiting a week.
        t.done(now, true);
        assert_eq!(t.wait_secs(700, now - 7 * INTERVAL_SECS), 0);
        // The timer itself is limited to an hour and at least a second.
        assert_eq!(Timing::timer_ms(INTERVAL_SECS), 3_600_000);
        assert_eq!(Timing::timer_ms(0), 1000);
        assert_eq!(Timing::timer_ms(600), 600_000);
    }

    #[test]
    fn progress_is_percent_or_kilobytes() {
        assert_eq!(progress_text(0, Some(1000)), "0 %");
        assert_eq!(progress_text(500, Some(1000)), "50 %");
        assert_eq!(progress_text(2000, Some(1000)), "100 %");
        assert_eq!(progress_text(2048, None), "2 kB");
        assert_eq!(progress_text(2048, Some(0)), "2 kB");
    }
}
