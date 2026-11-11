//! Settings (plan 6.6, 12.2): defaults plus `settings.json` in `%APPDATA%\AutoSpaces`.

use crate::json::{parse_json, quote, Json};
use std::collections::BTreeMap;
use std::path::Path;

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
  \"switchAnimation\": {}
}}
",
            quote(if self.fullscreen_only() { "fullscreen" } else { "all" }),
            list.join(", "),
            names.join(", "),
            self.autostart,
            self.learning,
            self.switch_animation
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
}
