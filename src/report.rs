//! "Report a problem" (plan 12.12): the text of `report.txt` and the anonymization behind it.
//!
//! Everything here is pure (no Win32 calls except `display_info`), so it is covered by tests. The
//! file holds no window titles, no user name and no computer name: paths are shortened to
//! placeholders, readable app names, launch arguments and anything else that could be a title are
//! left out of the two JSON files. Nothing is ever sent by the application; the user attaches the
//! file to a GitHub issue himself.

use crate::core::settings::launch_label;
use crate::json::{parse_json, quote, Json};

/// The issue form on GitHub. No data of the user or the system is ever put into this address.
pub const ISSUE_URL: &str = "https://github.com/premekvibe/AutoSpaces/issues/new?template=bug_report.yml";

/// How many trailing lines of `log.txt` go into the report.
pub const LOG_TAIL_LINES: usize = 400;

/// Replaces a value that must not leave the computer.
const OMITTED: &str = "(omitted)";

/// What identifies the user and the computer in paths and texts.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub user: String,
    pub computer: String,
    pub domain: String,
    /// (folder, placeholder), longest folder first.
    pub paths: Vec<(String, String)>,
}

impl Env {
    /// Read from the environment of this process.
    pub fn from_process() -> Env {
        let var = |n: &str| std::env::var(n).unwrap_or_default();
        let mut paths: Vec<(String, String)> = [
            ("USERPROFILE", "%USERPROFILE%"),
            ("LOCALAPPDATA", "%LOCALAPPDATA%"),
            ("APPDATA", "%APPDATA%"),
            ("TEMP", "%TEMP%"),
            ("TMP", "%TEMP%"),
        ]
        .iter()
        .map(|(n, ph)| (var(n).trim_end_matches(['\\', '/']).to_string(), ph.to_string()))
        .filter(|(p, _)| !p.is_empty())
        .collect();
        paths.sort_by_key(|(p, _)| std::cmp::Reverse(p.chars().count()));
        Env { user: var("USERNAME"), computer: var("COMPUTERNAME"), domain: var("USERDOMAIN"), paths }
    }
}

fn same_char(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Where a match may sit.
#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Anywhere,
    /// Not followed by a letter or digit (a folder, not the start of a longer name).
    EndOfName,
    /// Not inside a longer word.
    WholeWord,
}

/// Case-insensitive replacement of `needle`.
fn replace_ci(text: &str, needle: &str, with: &str, edge: Edge) -> String {
    let h: Vec<char> = text.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < h.len() {
        let hit = i + n.len() <= h.len()
            && h[i..i + n.len()].iter().zip(&n).all(|(a, b)| same_char(*a, *b))
            && (edge == Edge::Anywhere || !(i + n.len() < h.len() && h[i + n.len()].is_alphanumeric()))
            && (edge != Edge::WholeWord || !(i > 0 && h[i - 1].is_alphanumeric()));
        if hit {
            out.push_str(with);
            i += n.len();
        } else {
            out.push(h[i]);
            i += 1;
        }
    }
    out
}

/// Folders of other users in `X:\Users\<name>\...` (the name ends at a separator, a quote or a space).
fn redact_user_dirs(text: &str) -> String {
    const KEEP: [&str; 6] = ["public", "default", "all", "<user>", "%userprofile%", "%localappdata%"];
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        out.push(c[i]);
        let sep = |x: char| x == '\\' || x == '/';
        let users: Vec<char> = "users".chars().collect();
        let head = c[i] == ':'
            && i + 7 < c.len()
            && sep(c[i + 1])
            && c[i + 2..i + 7].iter().zip(&users).all(|(a, b)| same_char(*a, *b))
            && sep(c[i + 7]);
        if head {
            let start = i + 8;
            let mut end = start;
            while end < c.len() && !sep(c[end]) && !c[end].is_whitespace() && !"\"'|*?".contains(c[end]) {
                end += 1;
            }
            let name: String = c[start..end].iter().collect();
            out.extend(&c[i + 1..start]);
            if name.is_empty() || KEEP.contains(&name.to_lowercase().as_str()) {
                out.push_str(&name);
            } else {
                out.push_str("<user>");
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// A text without the user's folders, user name and computer name.
pub fn anonymize(text: &str, env: &Env) -> String {
    let mut s = text.to_string();
    for (prefix, ph) in &env.paths {
        if prefix.chars().count() >= 4 {
            s = replace_ci(&s, prefix, ph, Edge::EndOfName);
        }
    }
    s = redact_user_dirs(&s);
    for (value, ph) in [(&env.computer, "<computer>"), (&env.domain, "<domain>"), (&env.user, "<user>")] {
        if value.chars().count() >= 3 {
            s = replace_ci(&s, value, ph, Edge::WholeWord);
        }
    }
    s
}

fn redact_value(v: &Json, env: &Env) -> Json {
    match v {
        Json::Str(s) => Json::Str(anonymize(s, env)),
        Json::Arr(a) => Json::Arr(a.iter().map(|x| redact_value(x, env)).collect()),
        Json::Obj(m) => Json::Obj(
            m.iter()
                .map(|(k, x)| {
                    let key = anonymize(k, env);
                    let val = match (k.as_str(), x) {
                        // Readable names may come from a window title: only the process names stay.
                        ("excludedNames", Json::Obj(names)) => {
                            Json::Obj(names.iter().map(|(p, _)| (anonymize(p, env), Json::Str(OMITTED.into()))).collect())
                        }
                        // Wallpaper picture and folder (plan 12.17): personal names and places.
                        ("name" | "args" | "image" | "folder", Json::Str(s)) if !s.is_empty() => Json::Str(OMITTED.into()),
                        ("launch", Json::Str(s)) => Json::Str(anonymize(&launch_label(s), env)),
                        _ => redact_value(x, env),
                    };
                    (key, val)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn is_scalar(v: &Json) -> bool {
    !matches!(v, Json::Arr(_) | Json::Obj(_))
}

fn render(v: &Json, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent + 1);
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(&b.to_string()),
        Json::Num(n) => out.push_str(&n.to_string()),
        Json::Str(s) => out.push_str(&quote(s)),
        Json::Arr(a) if a.iter().all(is_scalar) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                render(x, indent, out);
            }
            out.push(']');
        }
        Json::Arr(a) => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                out.push_str(&pad);
                render(x, indent + 1, out);
                out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
            }
            out.push_str(&"  ".repeat(indent));
            out.push(']');
        }
        Json::Obj(m) if m.is_empty() => out.push_str("{}"),
        Json::Obj(m) => {
            out.push_str("{\n");
            for (i, (k, x)) in m.iter().enumerate() {
                out.push_str(&format!("{pad}{}: ", quote(k)));
                render(x, indent + 1, out);
                out.push_str(if i + 1 < m.len() { ",\n" } else { "\n" });
            }
            out.push_str(&"  ".repeat(indent));
            out.push('}');
        }
    }
}

/// Text of `settings.json` / `learned.json` for the report: parsed, names and launch arguments
/// left out, launch commands reduced to the file name or AUMID, paths anonymized. A file that
/// cannot be parsed is not copied (it could hold anything).
pub fn redact_json(text: &str, env: &Env) -> String {
    match parse_json(text) {
        Ok(j) => {
            let mut s = String::new();
            render(&redact_value(&j, env), 0, &mut s);
            s
        }
        Err(e) => format!("(unreadable, not included: {e})"),
    }
}

/// The last `n` lines.
pub fn tail_lines(text: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// Everything the report states, collected by the caller.
#[derive(Default)]
pub struct ReportData {
    pub version: String,
    pub windows_build: u32,
    pub ubr: Option<u32>,
    pub ui_language: String,
    pub monitors: i32,
    pub primary_size: (i32, i32),
    pub primary_dpi: u32,
    /// "normal", "unsupported (build N)", "switched off in settings.json", "starting".
    pub mode: String,
    /// Name of the chosen variant, if any.
    pub variant: Option<String>,
    pub variant_verified: bool,
    /// What the variant selection did, line by line.
    pub selection: Vec<String>,
    /// Number of desktops, or why it is unknown.
    pub desktops: Option<String>,
    pub logging: bool,
    pub settings_json: Option<String>,
    pub learned_json: Option<String>,
    pub log: Option<String>,
}

/// The whole text of `report.txt`.
pub fn build_report(d: &ReportData, env: &Env) -> String {
    let mut s = String::new();
    s.push_str("AutoSpaces diagnostic report\n");
    s.push_str("Created by AutoSpaces on this computer; nothing was sent anywhere. It holds no window titles and\n");
    s.push_str("no user or computer name (folders of the user's profile are shown as %USERPROFILE% etc.).\n\n");
    s.push_str(&format!("AutoSpaces version: {}\n", d.version));
    let ubr = d.ubr.map_or(String::new(), |u| format!(".{u}"));
    s.push_str(&format!("Windows build: {}{ubr}\n", d.windows_build));
    s.push_str(&format!("UI language: {}\n", d.ui_language));
    let scale = d.primary_dpi * 100 / 96;
    s.push_str(&format!(
        "Primary monitor: {}x{} px, {} DPI ({scale}% scaling)\n",
        d.primary_size.0, d.primary_size.1, d.primary_dpi
    ));
    s.push_str(&format!("Monitors: {}\n", d.monitors));
    s.push_str(&format!("Mode: {}\n", d.mode));
    match &d.variant {
        Some(v) => {
            let tag = if d.variant_verified { "verified" } else { "UNVERIFIED (taken from public sources)" };
            s.push_str(&format!("Interface variant: {v} - {tag}\n"));
        }
        None => s.push_str("Interface variant: none\n"),
    }
    s.push_str(&format!("Desktops: {}\n", d.desktops.as_deref().unwrap_or("unknown")));
    s.push_str(&format!("Activity logging: {}\n", if d.logging { "on" } else { "off" }));
    s.push_str("\n== Variant selection ==\n");
    if d.selection.is_empty() {
        s.push_str("(not run)\n");
    }
    for l in &d.selection {
        s.push_str(&anonymize(l, env));
        s.push('\n');
    }
    for (title, text) in [("settings.json", &d.settings_json), ("learned.json", &d.learned_json)] {
        s.push_str(&format!("\n== {title} ==\n"));
        match text {
            Some(t) => s.push_str(&redact_json(t, env)),
            None => s.push_str("(none)"),
        }
        s.push('\n');
    }
    match &d.log {
        Some(t) => {
            let lines = tail_lines(t, LOG_TAIL_LINES);
            s.push_str(&format!("\n== log.txt (last {} lines) ==\n", lines.len()));
            for l in lines {
                s.push_str(&anonymize(l, env));
                s.push('\n');
            }
        }
        None => s.push_str("\n== log.txt ==\n(none: activity logging was off, or nothing was logged yet)\n"),
    }
    s
}

/// (monitor count, primary monitor size in pixels, its DPI) for the report.
pub fn display_info() -> (i32, (i32, i32), u32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS, SM_CXSCREEN, SM_CYSCREEN};
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 || dx == 0 {
            dx = 96;
        }
        (GetSystemMetrics(SM_CMONITORS), (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)), dx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            user: "Jana".into(),
            computer: "DESKTOP-4ABC".into(),
            domain: "DESKTOP-4ABC".into(),
            paths: vec![
                (r"C:\Users\Jana\AppData\Local\Temp".into(), "%TEMP%".into()),
                (r"C:\Users\Jana\AppData\Local".into(), "%LOCALAPPDATA%".into()),
                (r"C:\Users\Jana\AppData\Roaming".into(), "%APPDATA%".into()),
                (r"C:\Users\Jana".into(), "%USERPROFILE%".into()),
            ],
        }
    }

    #[test]
    fn profile_folders_become_placeholders() {
        let e = env();
        assert_eq!(anonymize(r"C:\Users\Jana\Documents\a.exe", &e), r"%USERPROFILE%\Documents\a.exe");
        assert_eq!(anonymize(r"c:\users\JANA\appdata\local\Foo\b.exe", &e), r"%LOCALAPPDATA%\Foo\b.exe");
        assert_eq!(anonymize(r"C:\Users\Jana\AppData\Local\Temp\x", &e), r"%TEMP%\x");
        assert_eq!(anonymize(r"C:\Users\Jana\AppData\Roaming\AutoSpaces", &e), r"%APPDATA%\AutoSpaces");
        // Not a prefix of a longer name.
        assert_eq!(anonymize(r"C:\Users\Janak\x", &e), r"C:\Users\<user>\x");
        // Folders outside the profile stay.
        assert_eq!(anonymize(r"C:\Program Files\Foo\foo.exe", &e), r"C:\Program Files\Foo\foo.exe");
    }

    #[test]
    fn other_users_and_system_profiles() {
        let e = env();
        assert_eq!(anonymize(r"D:\Users\Jana\x.exe", &e), r"D:\Users\<user>\x.exe");
        assert_eq!(anonymize(r"C:\Users\Public\Documents", &e), r"C:\Users\Public\Documents");
        assert_eq!(anonymize(r"C:\Users\Default\NTUSER.DAT", &e), r"C:\Users\Default\NTUSER.DAT");
        assert_eq!(anonymize("C:/Users/Eva/x", &e), "C:/Users/<user>/x");
        assert_eq!(anonymize(r#""C:\Users\Eva""#, &e), r#""C:\Users\<user>""#);
    }

    #[test]
    fn names_are_replaced_as_whole_words_only() {
        let e = env();
        assert_eq!(anonymize("user Jana logged in", &e), "user <user> logged in");
        assert_eq!(anonymize("host desktop-4abc!", &e), "host <computer>!");
        assert_eq!(anonymize("Janaditated", &e), "Janaditated");
        let short = Env { user: "ab".into(), ..Default::default() };
        assert_eq!(anonymize("a ab b", &short), "a ab b");
        assert_eq!(anonymize("", &e), "");
    }

    #[test]
    fn czech_names_are_found_whatever_the_case() {
        let e = Env { user: "Přemysl".into(), ..Default::default() };
        assert_eq!(anonymize("ahoj PŘEMYSL, přemysl.", &e), "ahoj <user>, <user>.");
    }

    #[test]
    fn launch_commands_and_names_are_cut_in_json() {
        let e = env();
        let settings = r#"{"mode":"all","excluded":["a.exe"],"excludedNames":{"a.exe":"Secret title - Document1"},
          "layout":{"enabled":true,"desktopCount":3,"rules":[
            {"desktop":2,"name":"My Inbox - Outlook","launch":"C:\\Users\\Jana\\AppData\\Local\\Programs\\Foo\\foo.exe","args":"--profile Jana","match":{"process":"foo.exe"}},
            {"desktop":3,"name":"","launch":"shell:AppsFolder\\Fam_8we!App","args":"","match":{"process":"x.exe"}}]}}"#;
        let out = redact_json(settings, &e);
        for bad in ["Secret", "Inbox", "Jana", "Users", "--profile", "Programs"] {
            assert!(!out.contains(bad), "{bad} in {out}");
        }
        for good in ["\"a.exe\"", "\"foo.exe\"", "shell:AppsFolder\\\\Fam_8we!App", "\"desktopCount\": 3", "(omitted)"] {
            assert!(out.contains(good), "{good} missing in {out}");
        }
        // The output is valid JSON again.
        assert!(parse_json(&out).is_ok(), "{out}");
    }

    #[test]
    fn wallpaper_paths_are_left_out() {
        let e = env();
        let settings = r#"{"wallpapers":{"mode":"random","image":"D:\\Fotky\\Jana u more.jpg","folder":"D:\\Fotky\\Dovolena","subfolders":true,"newDesktops":true}}"#;
        let out = redact_json(settings, &e);
        for bad in ["Fotky", "Jana", "Dovolena"] {
            assert!(!out.contains(bad), "{bad} in {out}");
        }
        assert!(out.contains("\"mode\": \"random\"") && out.contains("(omitted)"), "{out}");
    }

    #[test]
    fn learned_names_are_left_out() {
        let e = env();
        let learned = r#"{"version":1,"entries":[{"process":"app.exe","class":"Cls","reason":"NoView","strikes":3,"blocked":true,"backend":"internal:win11-24h2","exeStamp":"1:2","first":"2026-01-01","last":"2026-01-02","name":"Some Window Title"}]}"#;
        let out = redact_json(learned, &e);
        assert!(!out.contains("Some Window Title"));
        assert!(out.contains("app.exe") && out.contains("NoView") && out.contains("internal:win11-24h2"));
    }

    #[test]
    fn broken_json_is_not_copied() {
        let out = redact_json(r#"{"secret": "Jana" oops"#, &env());
        assert!(out.starts_with("(unreadable"));
        assert!(!out.contains("secret") && !out.contains("Jana"));
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), ["c", "d"]);
        assert_eq!(tail_lines("a\nb", 5), ["a", "b"]);
        assert!(tail_lines("", 5).is_empty());
    }

    fn sample() -> ReportData {
        ReportData {
            version: "0.1.0".into(),
            windows_build: 26100,
            ubr: Some(1234),
            ui_language: "cs (setting: auto)".into(),
            monitors: 2,
            primary_size: (2560, 1440),
            primary_dpi: 144,
            mode: "normal".into(),
            variant: Some("internal:win11-24h2".into()),
            variant_verified: true,
            selection: vec![r"variants: C:\Users\Jana\AppData\Local\Programs\AutoSpaces\variants.json".into()],
            desktops: Some("3".into()),
            logging: true,
            settings_json: Some(r#"{"mode":"all"}"#.into()),
            learned_json: None,
            log: Some(
                (0..500)
                    .map(|i| format!("2026-11-13 10:00:00.{i:03} layout: launched C:\\Users\\Jana\\x.exe for {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }

    #[test]
    fn report_has_the_fields_and_no_personal_data() {
        let r = build_report(&sample(), &env());
        for field in [
            "AutoSpaces version: 0.1.0",
            "Windows build: 26100.1234",
            "UI language: cs",
            "Primary monitor: 2560x1440 px, 144 DPI (150% scaling)",
            "Monitors: 2",
            "Mode: normal",
            "Interface variant: internal:win11-24h2 - verified",
            "Desktops: 3",
            "Activity logging: on",
            "== Variant selection ==",
            "== settings.json ==",
            "== learned.json ==\n(none)",
            "== log.txt (last 400 lines) ==",
        ] {
            assert!(r.contains(field), "{field} missing:\n{r}");
        }
        assert!(!r.contains("Jana") && !r.to_lowercase().contains("desktop-4abc"), "{r}");
        assert!(r.contains("%USERPROFILE%\\x.exe"));
        // Only the last 400 of the 500 lines.
        assert!(r.contains(".499 ") && r.contains(".100 ") && !r.contains(".099 "));
    }

    #[test]
    fn report_without_log_or_variant() {
        let d = ReportData { variant: None, log: None, selection: vec![], ..sample() };
        let r = build_report(&d, &env());
        assert!(r.contains("Interface variant: none") && r.contains("(not run)") && r.contains("== log.txt ==\n(none"));
        let u = ReportData { variant_verified: false, ..sample() };
        assert!(build_report(&u, &env()).contains("UNVERIFIED"));
    }

    #[test]
    fn issue_url_is_fixed_and_carries_no_data() {
        assert_eq!(ISSUE_URL, "https://github.com/premekvibe/AutoSpaces/issues/new?template=bug_report.yml");
        assert_eq!(ISSUE_URL.matches('?').count(), 1);
        assert!(!ISSUE_URL.contains('&'));
        assert!(!ISSUE_URL.contains(env!("CARGO_PKG_VERSION")));
    }
}
