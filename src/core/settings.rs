//! Settings (plan 6.6, 12.2): defaults plus `settings.json` in `%APPDATA%\AutoSpaces`.

use crate::json::{parse_json, quote, Json};
use std::collections::BTreeMap;
use std::path::Path;

/// Upper bound of desktops a profile may ask for (sanity limit for hand-edited files).
pub const MAX_LAYOUT_DESKTOPS: u32 = 20;
const MAX_LAYOUT_RULES: usize = 50;

/// System hosts that never make sense to exclude, to put into a layout profile or to put on the
/// learned list (plan 6.7). `explorer.exe` and `applicationframehost.exe` also host ordinary
/// windows (File Explorer, UWP apps), so they are still processed; the shell-only ones
/// (`SHELL_ONLY`) are not processed at all.
pub const SYSTEM_HOSTS: [&str; 10] = [
    "applicationframehost.exe",
    "systemsettings.exe",
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "explorer.exe",
    "autospaces.exe",
    "lockapp.exe",
    "logonui.exe",
];

/// Shell processes whose windows are never real app windows: never moved, never learned.
const SHELL_ONLY: [&str; 6] = [
    "textinputhost.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "lockapp.exe",
    "logonui.exe",
];

/// Window classes of the shell, the lock screen and system CoreWindows (any process).
const SYSTEM_CLASSES: [&str; 9] = [
    "Windows.UI.Core.CoreWindow",
    "LockScreenBackstopFrame",
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "MultitaskingViewFrame",
    "XamlExplorerHostIslandWindow",
    "ForegroundStaging",
];

/// A shell / lock screen / system window: AutoSpaces never touches it (plan 6.7).
pub fn is_system_window(process: &str, class: &str) -> bool {
    SHELL_ONLY.contains(&process) || SYSTEM_CLASSES.contains(&class)
}

/// Never recorded on the learned list: system hosts (one bad window would block all windows
/// they host) and system windows.
pub fn never_learn(process: &str, class: &str) -> bool {
    process.is_empty() || SYSTEM_HOSTS.contains(&process) || is_system_window(process, class)
}

/// Launch command of a packaged app (`shell:AppsFolder\<AUMID>`).
pub fn apps_folder(aumid: &str) -> String {
    format!(r"shell:AppsFolder\{aumid}")
}

/// For an exe inside `...\WindowsApps\<package folder>\...`: (package folder path, exe file name
/// lower-case, package family name). The folder name is `Name_Version_Arch_Resource_PublisherId`.
pub fn windows_apps_package(path: &str) -> Option<(String, String, String)> {
    let p = path.trim().trim_matches('"');
    let lower = p.to_ascii_lowercase(); // same byte offsets as `p`
    let key = "\\windowsapps\\";
    let at = lower.find(key)? + key.len();
    let rest = &p[at..];
    let folder = rest.split(['\\', '/']).next()?;
    let parts: Vec<&str> = folder.split('_').collect();
    if parts.len() < 3 || parts[0].is_empty() || parts[parts.len() - 1].is_empty() || rest.len() <= folder.len() {
        return None;
    }
    let family = format!("{}_{}", parts[0], parts[parts.len() - 1]);
    Some((p[..at + folder.len()].to_string(), file_name_lower(p), family))
}

/// Application id from `AppxManifest.xml` text for the exe `exe` (lower-case file name);
/// a manifest with a single application gives that one.
pub fn manifest_app_id(manifest: &str, exe: &str) -> Option<String> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let pat = format!(" {name}=\"");
        let i = tag.find(&pat)? + pat.len();
        let j = tag[i..].find('"')? + i;
        Some(tag[i..j].to_string())
    };
    let mut apps: Vec<(String, String)> = Vec::new();
    let mut rest = manifest;
    while let Some(i) = rest.find("<Application ") {
        let tail = &rest[i..];
        let end = tail.find('>').unwrap_or(tail.len());
        let tag = tail[..end].replace(['\r', '\n', '\t'], " ");
        if let Some(id) = attr(&tag, "Id") {
            apps.push((id, attr(&tag, "Executable").map(|e| file_name_lower(&e)).unwrap_or_default()));
        }
        rest = &tail[end..];
    }
    match apps.iter().find(|a| a.1 == exe) {
        Some(a) => Some(a.0.clone()),
        None if apps.len() == 1 => Some(apps[0].0.clone()),
        None => None,
    }
}

/// `shell:AppsFolder\<AUMID>` for an exe path inside WindowsApps; `manifest` reads
/// `AppxManifest.xml` of the package folder. None for other paths or when it cannot be resolved.
pub fn packaged_launch(path: &str, manifest: impl Fn(&str) -> Option<String>) -> Option<String> {
    let (folder, exe, family) = windows_apps_package(path)?;
    let id = manifest_app_id(&manifest(&folder)?, &exe)?;
    Some(apps_folder(&format!("{family}!{id}")))
}

/// One application of the layout profile (plan 12.7): `launch` starts it, `process` pairs a
/// running window with it (never by pid: launchers exit and hand over to another process).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutRule {
    /// Target desktop, 1-based.
    pub desktop: u32,
    /// Readable name (display only; may be empty).
    pub name: String,
    /// Path to the exe or a command (`olk.exe`).
    pub launch: String,
    pub args: String,
    /// Lower-case process image name used for matching windows.
    pub process: String,
}

/// Desktop layout applied after start (plan 12.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub enabled: bool,
    pub desktop_count: u32,
    /// Wait after AutoSpaces starts, so apps are not launched into a system still booting.
    pub start_delay_ms: u32,
    pub rules: Vec<LayoutRule>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { enabled: false, desktop_count: 1, start_delay_ms: 5000, rules: Vec::new() }
    }
}

/// File name of a path or command, lower-case (`C:\x\Foo.EXE` -> `foo.exe`).
pub fn file_name_lower(path: &str) -> String {
    path.trim().trim_matches('"').rsplit(['\\', '/']).next().unwrap_or("").trim().to_lowercase()
}

impl LayoutRule {
    /// Builds a rule; None when neither a process nor a launch command can be derived.
    pub fn new(desktop: u32, name: &str, launch: &str, args: &str, process: &str) -> Option<LayoutRule> {
        let launch = launch.trim().to_string();
        let mut process = norm_process(process);
        if process.is_empty() {
            process = file_name_lower(&launch);
        }
        if process.is_empty() {
            return None;
        }
        let launch = if launch.is_empty() { process.clone() } else { launch };
        Some(LayoutRule {
            desktop: desktop.clamp(1, MAX_LAYOUT_DESKTOPS),
            name: name.trim().to_string(),
            launch,
            args: args.trim().to_string(),
            process,
        })
    }

    pub fn label(&self) -> String {
        display_label(&self.process, &self.name)
    }
}

impl Layout {
    /// Rules launching an exe inside WindowsApps (its path holds the version and breaks with the
    /// next update) are switched to `shell:AppsFolder\<AUMID>` where it can be resolved.
    /// True when anything changed.
    pub fn migrate_packaged(&mut self, manifest: impl Fn(&str) -> Option<String>) -> bool {
        let mut changed = false;
        for r in &mut self.rules {
            if let Some(cmd) = packaged_launch(&r.launch, &manifest) {
                r.launch = cmd;
                changed = true;
            }
        }
        changed
    }

    /// Desktops the profile needs: the chosen count, raised to the highest target desktop.
    pub fn count(&self) -> usize {
        let top = self.rules.iter().map(|r| r.desktop).max().unwrap_or(1);
        self.desktop_count.max(top).clamp(1, MAX_LAYOUT_DESKTOPS) as usize
    }

    fn from_json(j: &Json) -> Layout {
        let mut l = Layout::default();
        if let Some(Json::Bool(b)) = j.get("enabled") {
            l.enabled = *b;
        }
        if let Some(n) = j.get("desktopCount").and_then(Json::as_uint) {
            l.desktop_count = (n as u32).clamp(1, MAX_LAYOUT_DESKTOPS);
        }
        if let Some(n) = j.get("startDelayMs").and_then(Json::as_uint) {
            l.start_delay_ms = (n as u32).min(120_000);
        }
        if let Some(Json::Arr(a)) = j.get("rules") {
            for r in a.iter().take(MAX_LAYOUT_RULES) {
                let text = |k: &str| r.get(k).and_then(Json::as_str).unwrap_or("");
                let process = r.get("match").and_then(|m| m.get("process")).and_then(Json::as_str).unwrap_or("");
                let desktop = r.get("desktop").and_then(Json::as_uint).unwrap_or(1) as u32;
                if let Some(rule) = LayoutRule::new(desktop, text("name"), text("launch"), text("args"), process) {
                    if !l.rules.contains(&rule) {
                        l.rules.push(rule);
                    }
                }
            }
        }
        l
    }

    fn to_json(&self) -> String {
        let rules: Vec<String> = self
            .rules
            .iter()
            .map(|r| {
                format!(
                    "    {{\"desktop\": {}, \"name\": {}, \"launch\": {}, \"args\": {}, \"match\": {{\"process\": {}}}}}",
                    r.desktop,
                    quote(&r.name),
                    quote(&r.launch),
                    quote(&r.args),
                    quote(&r.process)
                )
            })
            .collect();
        let list = if rules.is_empty() { String::new() } else { format!("\n{}\n  ", rules.join(",\n")) };
        format!(
            "{{\"enabled\": {}, \"desktopCount\": {}, \"startDelayMs\": {}, \"rules\": [{}]}}",
            self.enabled, self.desktop_count, self.start_delay_ms, list
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub switch_animation: bool,
    pub switch_back_when_others_remain: bool,
    pub fullscreen_settle_ms: u32,
    pub trigger_on_maximize: bool,
    pub trigger_on_fullscreen: bool,
    /// A window that vanishes may be replaced at once (splash -> main window): wait this long
    /// before deleting the desktop it leaves empty (plan 12.1).
    pub empty_desktop_grace_ms: u32,
    /// Self-learning block list (plan 6.7); false = nothing recorded, nothing blocked.
    pub learning: bool,
    /// Days after which softly blocked apps (Flapping, ShortLived) are tried again.
    pub learned_retry_days: u32,
    /// Lower-case process names that are never moved (plan 12.2).
    pub excluded: Vec<String>,
    /// Readable names of excluded processes (display only; may be missing).
    pub names: BTreeMap<String, String>,
    /// Start with Windows (HKCU Run). The registry is the truth; this mirrors the last choice.
    pub autostart: bool,
    /// Desktop layout profile applied after start (plan 12.7).
    pub layout: Layout,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            switch_animation: false,
            switch_back_when_others_remain: true,
            fullscreen_settle_ms: 60,
            trigger_on_maximize: true,
            trigger_on_fullscreen: true,
            empty_desktop_grace_ms: 300,
            learning: true,
            learned_retry_days: 30,
            excluded: Vec::new(),
            names: BTreeMap::new(),
            autostart: false,
            layout: Layout::default(),
        }
    }
}

/// "Readable name (file.exe)", or just the file when the name is unknown or the same.
pub fn display_label(process: &str, name: &str) -> String {
    let n = name.trim();
    if n.is_empty() || n.eq_ignore_ascii_case(process) {
        process.to_string()
    } else {
        format!("{n} ({process})")
    }
}

/// Canonical form of a process name in the exclusion list.
pub fn norm_process(s: &str) -> String {
    s.trim().to_lowercase()
}

impl Settings {
    /// Launch mode (a): every maximized window and fullscreen; (b) fullscreen only.
    pub fn fullscreen_only(&self) -> bool {
        !self.trigger_on_maximize
    }

    pub fn set_fullscreen_only(&mut self, only: bool) {
        self.trigger_on_maximize = !only;
        self.trigger_on_fullscreen = true;
    }

    pub fn is_excluded(&self, process: &str) -> bool {
        !self.excluded.is_empty() && self.excluded.iter().any(|e| e == process)
    }

    /// Adds a process to the exclusion list (normalized, no duplicates); false = nothing added.
    pub fn exclude(&mut self, process: &str) -> bool {
        let p = norm_process(process);
        if p.is_empty() || self.excluded.contains(&p) {
            return false;
        }
        self.excluded.push(p);
        self.excluded.sort();
        true
    }

    /// Remembers a readable name for an excluded process (empty names are ignored).
    pub fn set_name(&mut self, process: &str, name: &str) {
        let (p, n) = (norm_process(process), name.trim());
        if !n.is_empty() {
            self.names.insert(p, n.to_string());
        }
    }

    /// Reads `settings.json` text; tolerant: a missing or wrongly typed key keeps its default.
    /// Err only when the text is not JSON at all (the caller then uses the defaults).
    pub fn from_json(text: &str) -> Result<Settings, String> {
        let j = parse_json(text)?;
        let mut s = Settings::default();
        if !matches!(j, Json::Obj(_)) {
            return Err("settings.json is not an object".into());
        }
        let flag = |key: &str, dst: &mut bool| {
            if let Some(Json::Bool(b)) = j.get(key) {
                *dst = *b;
            }
        };
        match j.get("mode").and_then(Json::as_str) {
            Some("fullscreen") => s.set_fullscreen_only(true),
            Some("all") => s.set_fullscreen_only(false),
            _ => {}
        }
        flag("autostart", &mut s.autostart);
        flag("learning", &mut s.learning);
        flag("switchAnimation", &mut s.switch_animation);
        if let Some(l @ Json::Obj(_)) = j.get("layout") {
            s.layout = Layout::from_json(l);
        }
        if let Some(Json::Arr(a)) = j.get("excluded") {
            for item in a {
                if let Some(name) = item.as_str() {
                    s.exclude(name);
                }
            }
        }
        if let Some(Json::Obj(m)) = j.get("excludedNames") {
            for (k, v) in m {
                if let (Some(n), true) = (v.as_str(), s.excluded.contains(&norm_process(k))) {
                    s.set_name(k, n);
                }
            }
        }
        Ok(s)
    }

    pub fn to_json(&self) -> String {
        let list: Vec<String> = self.excluded.iter().map(|e| quote(e)).collect();
        let names: Vec<String> = self
            .names
            .iter()
            .filter(|(k, _)| self.excluded.contains(*k))
            .map(|(k, v)| format!("{}: {}", quote(k), quote(v)))
            .collect();
        format!(
            "{{
  \"mode\": {},
  \"excluded\": [{}],
  \"excludedNames\": {{{}}},
  \"autostart\": {},
  \"learning\": {},
  \"switchAnimation\": {},
  \"layout\": {}
}}
",
            quote(if self.fullscreen_only() { "fullscreen" } else { "all" }),
            list.join(", "),
            names.join(", "),
            self.autostart,
            self.learning,
            self.switch_animation,
            self.layout.to_json()
        )
    }

    /// Missing or unreadable file = defaults.
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|t| Settings::from_json(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_empty_and_partial() {
        let d = Settings::default();
        assert_eq!(Settings::from_json("{}").unwrap(), d);
        let s = Settings::from_json(r#"{"mode":"fullscreen"}"#).unwrap();
        assert!(s.fullscreen_only() && s.trigger_on_fullscreen && s.excluded.is_empty() && s.learning);
    }

    #[test]
    fn tolerant_to_wrong_types_and_unknown_keys() {
        let s = Settings::from_json(
            r#"{"mode":5,"excluded":"x","autostart":1,"learning":false,"extra":[1],"switchAnimation":"yes"}"#,
        )
        .unwrap();
        assert!(!s.fullscreen_only() && !s.autostart && !s.learning && !s.switch_animation);
        assert!(s.excluded.is_empty());
        assert!(Settings::from_json("not json").is_err());
        assert!(Settings::from_json("[1]").is_err());
    }

    #[test]
    fn roundtrip_and_normalization() {
        let mut s = Settings::default();
        s.set_fullscreen_only(true);
        s.autostart = true;
        s.learning = false;
        assert!(s.exclude("  Chrome.EXE "));
        assert!(!s.exclude("chrome.exe"));
        assert!(!s.exclude("   "));
        assert!(s.exclude("a\"b.exe"));
        let back = Settings::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
        assert!(back.is_excluded("chrome.exe") && !back.is_excluded("code.exe"));
        // names from a hand-edited file are normalized and deduplicated
        let h = Settings::from_json(r#"{"excluded":["Foo.exe","foo.exe",5,""," Bar.exe"]}"#).unwrap();
        assert_eq!(h.excluded, vec!["bar.exe", "foo.exe"]);
    }

    #[test]
    fn file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("autospaces-test-{}", std::process::id()));
        let path = dir.join("sub").join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let mut s = Settings::default();
        s.exclude("x.exe");
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_old_and_new_format() {
        // Old file (strings only): names unknown, label is the bare file.
        let old = Settings::from_json(r#"{"excluded":["pdr.exe"]}"#).unwrap();
        assert!(old.names.is_empty());
        assert_eq!(display_label("pdr.exe", old.names.get("pdr.exe").map_or("", String::as_str)), "pdr.exe");
        // New file: names round-trip; names of non-excluded processes are dropped.
        let mut s = old.clone();
        s.set_name("PDR.exe", "CyberLink PowerDirector 365");
        s.set_name("gone.exe", "Gone");
        s.set_name("pdr.exe", "  ");
        let back = Settings::from_json(&s.to_json()).unwrap();
        assert_eq!(back.names["pdr.exe"], "CyberLink PowerDirector 365");
        assert!(!back.names.contains_key("gone.exe"));
    }

    #[test]
    fn label_formatting() {
        assert_eq!(display_label("a.exe", ""), "a.exe");
        assert_eq!(display_label("a.exe", "A.EXE"), "a.exe");
        assert_eq!(display_label("a.exe", "App"), "App (a.exe)");
    }
    #[test]
    fn layout_roundtrip_and_old_files() {
        // Old file without `layout`: profile off, defaults.
        let old = Settings::from_json(r#"{"mode":"all","excluded":["a.exe"]}"#).unwrap();
        assert_eq!(old.layout, Layout::default());
        assert!(!old.layout.enabled && old.layout.rules.is_empty());
        let mut s = Settings::default();
        s.layout.enabled = true;
        s.layout.desktop_count = 4;
        s.layout.start_delay_ms = 2500;
        s.layout.rules = vec![
            LayoutRule::new(2, "Outlook \"new\"", r"C:\Program Files\Outlook\olk.exe", "--x y", "OLK.exe").unwrap(),
            LayoutRule::new(3, "", "TradingView.exe", "", "").unwrap(),
        ];
        let text = s.to_json();
        let back = Settings::from_json(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.layout.rules[0].process, "olk.exe");
        assert_eq!(back.layout.rules[1].process, "tradingview.exe");
        assert_eq!(back.layout.rules[0].label(), "Outlook \"new\" (olk.exe)");
        assert_eq!(back.layout.rules[1].label(), "tradingview.exe");
        assert!(text.contains("\"match\": {\"process\": \"olk.exe\"}"));
    }

    #[test]
    fn layout_is_tolerant() {
        let s = Settings::from_json(
            r#"{"layout":{"enabled":"yes","desktopCount":999,"startDelayMs":-5,"rules":[
                {"desktop":0,"launch":"C:\\a\\One.exe"},
                {"desktop":"x","match":{"process":" Two.EXE "}},
                {"desktop":7,"name":5,"launch":""},
                7,
                {"desktop":2,"launch":"C:\\a\\One.exe"},
                {"desktop":1,"launch":"C:\\a\\One.exe"}]}}"#,
        )
        .unwrap();
        let l = &s.layout;
        assert!(!l.enabled, "wrong type keeps the default");
        assert_eq!(l.desktop_count, 20, "clamped");
        assert_eq!(l.start_delay_ms, 5000, "negative value keeps the default");
        let got: Vec<(u32, &str, &str)> =
            l.rules.iter().map(|r| (r.desktop, r.process.as_str(), r.launch.as_str())).collect();
        assert_eq!(
            got,
            vec![(1, "one.exe", r"C:\a\One.exe"), (1, "two.exe", "two.exe"), (2, "one.exe", r"C:\a\One.exe")]
        );
        // `layout` of the wrong type is ignored altogether.
        assert_eq!(Settings::from_json(r#"{"layout":[1]}"#).unwrap().layout, Layout::default());
    }

    #[test]
    fn layout_count_follows_the_highest_rule() {
        let mut l = Layout::default();
        assert_eq!(l.count(), 1);
        l.desktop_count = 2;
        l.rules.push(LayoutRule::new(5, "", "a.exe", "", "").unwrap());
        assert_eq!(l.count(), 5);
        assert!(LayoutRule::new(1, "x", "", "", "").is_none());
        assert_eq!(file_name_lower(r#""C:\X y\App.EXE""#), "app.exe");
    }

    const OLK: &str = r"C:\Program Files\WindowsApps\Microsoft.OutlookForWindows_1.2026.1026.100_x64__8wekyb3d8bbwe\olk.exe";

    #[test]
    fn packaged_paths_become_apps_folder_commands() {
        let (folder, exe, family) = windows_apps_package(OLK).unwrap();
        assert!(folder.ends_with(r"\Microsoft.OutlookForWindows_1.2026.1026.100_x64__8wekyb3d8bbwe"));
        assert_eq!((exe.as_str(), family.as_str()), ("olk.exe", "Microsoft.OutlookForWindows_8wekyb3d8bbwe"));
        assert!(windows_apps_package(r"C:\Program Files\App\app.exe").is_none());
        assert!(windows_apps_package(r"C:\Program Files\WindowsApps\Broken\x.exe").is_none());
        let manifest = r#"<Package><Applications>
            <Application Id="Other" Executable="helper.exe"/>
            <Application Id="Microsoft.OutlookforWindows"
                Executable="olk.exe" EntryPoint="Windows.FullTrustApplication"></Application>
            </Applications></Package>"#;
        assert_eq!(manifest_app_id(manifest, "olk.exe").as_deref(), Some("Microsoft.OutlookforWindows"));
        assert_eq!(manifest_app_id(manifest, "nope.exe"), None, "ambiguous");
        assert_eq!(manifest_app_id(r#"<Application Id="App" Executable="app\TradingView.exe">"#, "x.exe").as_deref(), Some("App"));
        let read = |dir: &str| dir.ends_with("8wekyb3d8bbwe").then(|| manifest.to_string());
        assert_eq!(
            packaged_launch(OLK, read).as_deref(),
            Some(r"shell:AppsFolder\Microsoft.OutlookForWindows_8wekyb3d8bbwe!Microsoft.OutlookforWindows")
        );
        // Old settings.json: the WindowsApps path is replaced, other rules are kept.
        let mut l = Layout::default();
        l.rules.push(LayoutRule::new(2, "Outlook", OLK, "", "olk.exe").unwrap());
        l.rules.push(LayoutRule::new(3, "", r"C:\a\b.exe", "", "").unwrap());
        assert!(l.migrate_packaged(read));
        assert_eq!(l.rules[0].launch, apps_folder("Microsoft.OutlookForWindows_8wekyb3d8bbwe!Microsoft.OutlookforWindows"));
        assert_eq!((l.rules[0].process.as_str(), l.rules[1].launch.as_str()), ("olk.exe", r"C:\a\b.exe"));
        assert!(!l.migrate_packaged(read), "idempotent");
        // Unreadable manifest: the path stays.
        let mut l2 = Layout::default();
        l2.rules.push(LayoutRule::new(2, "", OLK, "", "olk.exe").unwrap());
        assert!(!l2.migrate_packaged(|_: &str| None));
        assert_eq!(l2.rules[0].launch, OLK);
    }

    #[test]
    fn system_windows_and_hosts() {
        for (p, c) in [
            ("lockapp.exe", "Windows.UI.Core.CoreWindow"),
            ("explorer.exe", "LockScreenBackstopFrame"),
            ("textinputhost.exe", "Anything"),
            ("app.exe", "Windows.UI.Core.CoreWindow"),
        ] {
            assert!(is_system_window(p, c), "{p} {c}");
            assert!(never_learn(p, c), "{p} {c}");
        }
        // File Explorer and UWP frames are real windows, but never learned (they host many apps).
        assert!(!is_system_window("explorer.exe", "CabinetWClass"));
        assert!(never_learn("explorer.exe", "CabinetWClass"));
        assert!(never_learn("applicationframehost.exe", "ApplicationFrameWindow"));
        assert!(!never_learn("msedge.exe", "Chrome_WidgetWin_1"));
        assert!(never_learn("", "X"));
    }

}
