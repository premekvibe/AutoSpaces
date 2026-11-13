//! Languages and every user-visible string that is not part of a dialog template (plan 12.10).
//!
//! * Dialog templates live in `assets/dialogs.inc` (once) with their texts in
//!   `assets/strings-<lang>.inc`; `dialog_box` / `create_dialog` load the variant of the chosen
//!   language explicitly (never the system default).
//! * Everything else (menu, tooltip, messages, generated labels) is the `strings!` table below:
//!   one row per key, one column per language.
//!
//! Adding a language: a `Lang` variant (+ `LANG_COUNT`, `LANGS`, `code`, `primary`, `native_name`),
//! one more column in every `strings!` row (the compiler insists), a `strings-<code>.inc` file
//! included from `assets/autospaces.rc`. The tests check that nothing is missing.

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::System::LibraryLoader::{FindResourceExW, GetModuleHandleW, LoadResource, LockResource};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateDialogIndirectParamW, DialogBoxParamW, DLGPROC, DLGTEMPLATE, RT_DIALOG,
};

pub const LANG_COUNT: usize = 2;

/// A language the application has texts for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Cs = 0,
    En = 1,
}

/// All languages, in the order of the table columns and of the settings combo box.
pub const LANGS: [Lang; LANG_COUNT] = [Lang::Cs, Lang::En];

impl Lang {
    /// Value used in `settings.json`.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Cs => "cs",
            Lang::En => "en",
        }
    }

    /// Primary language id (LANG_CZECH = 0x05, LANG_ENGLISH = 0x09).
    pub fn primary(self) -> u16 {
        match self {
            Lang::Cs => 0x05,
            Lang::En => 0x09,
        }
    }

    /// Language id of the dialog resources (primary + SUBLANG_DEFAULT); must match `autospaces.rc`.
    pub fn langid(self) -> u16 {
        self.primary() | (1 << 10)
    }

    /// Name of the language in that language (shown in the settings combo box).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Cs => "Čeština",
            Lang::En => "English",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        LANGS.iter().copied().find(|l| l.code() == code.trim().to_ascii_lowercase())
    }
}

/// The user's choice: follow Windows or force a language (`"language"` in settings.json).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pref {
    #[default]
    Auto,
    Fixed(Lang),
}

impl Pref {
    pub fn code(self) -> &'static str {
        match self {
            Pref::Auto => "auto",
            Pref::Fixed(l) => l.code(),
        }
    }

    /// Unknown or missing values mean `Auto`.
    pub fn from_code(code: &str) -> Pref {
        Lang::from_code(code).map_or(Pref::Auto, Pref::Fixed)
    }

    /// Position in the settings combo box: 0 = automatic, then `LANGS` in order.
    pub fn index(self) -> usize {
        match self {
            Pref::Auto => 0,
            Pref::Fixed(l) => 1 + LANGS.iter().position(|x| *x == l).unwrap_or(0),
        }
    }

    pub fn from_index(i: usize) -> Pref {
        i.checked_sub(1).and_then(|i| LANGS.get(i)).map_or(Pref::Auto, |l| Pref::Fixed(*l))
    }
}

/// Language of a Windows language id (Czech -> Czech, everything else -> English).
pub fn detect(langid: u16) -> Lang {
    let primary = langid & 0x3ff;
    LANGS.iter().copied().find(|l| l.primary() == primary).unwrap_or(Lang::En)
}

/// The language to use: the forced one, else the one of the Windows UI language.
pub fn resolve(pref: Pref, ui_langid: u16) -> Lang {
    match pref {
        Pref::Fixed(l) => l,
        Pref::Auto => detect(ui_langid),
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Lang::En as u8);

/// Applies the user's choice; menus and dialogs opened afterwards use it.
pub fn set_pref(pref: Pref) {
    let lang = resolve(pref, unsafe { GetUserDefaultUILanguage() });
    CURRENT.store(lang as u8, Relaxed);
}

pub fn current() -> Lang {
    LANGS.get(CURRENT.load(Relaxed) as usize).copied().unwrap_or(Lang::En)
}

macro_rules! strings {
    ($($key:ident => [$cs:expr, $en:expr]),* $(,)?) => {
        /// Keys of the string table.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Key { $($key),* }
        /// Every key, for the completeness tests.
        #[cfg(test)]
        pub const ALL_KEYS: &[Key] = &[$(Key::$key),*];
        const TABLE: &[[&str; LANG_COUNT]] = &[$([$cs, $en]),*];
    };
}

strings! {
    // Tray icon and its menu
    TrayTip => ["AutoSpaces", "AutoSpaces"],
    MenuEnabled => ["Zapnuto", "Enabled"],
    MenuLearned => ["Naučené výjimky", "Learned exceptions"],
    MenuNone => ["(žádné)", "(none)"],
    MenuForgetAll => ["Zapomenout vše", "Forget all"],
    MenuSettings => ["Nastavení…", "Settings…"],
    MenuAbout => ["O aplikaci…", "About…"],
    MenuExit => ["Ukončit", "Exit"],
    // Settings dialogs
    AppName => ["AutoSpaces", "AutoSpaces"],
    LangAuto => ["Automaticky", "Automatic"],
    // Long paragraphs of the dialogs (RC would cut them at 256 characters)
    SettingsHint => [
        "Aby klik na ikonu běžící aplikace přepnul na její plochu, nastavte ve Windows v části Plochy „Na hlavním panelu zobrazovat všechna otevřená okna“ na „Na všech plochách“. Shift + klik na ikonu spustí novou instanci aplikace.",
        "To make a click on a running app's taskbar icon switch to its desktop, open Settings > System > Multitasking in Windows and, under Desktops, set “On the taskbar, show all the open windows” to “On all desktops”. Shift + click on the icon starts a new instance of the app."
    ],
    LayoutInfo => [
        "Při startu AutoSpaces doplní počet ploch, okna běžících aplikací přesune na zvolené plochy a ostatní aplikace spustí. Okno, které se do 30 sekund neobjeví, se vzdá a zapíše do protokolu. Pohled zůstane na vaší ploše.",
        "At startup, AutoSpaces adds any missing desktops, moves the windows of running apps to the chosen desktops and launches the other apps. If a window does not appear within 30 seconds, AutoSpaces gives up on it and notes it in the log. Your view stays on your own desktop."
    ],
    PickExcludeTitle => ["Vyloučit aplikaci", "Exclude app"],
    PickExcludeButton => ["Vyloučit", "Exclude"],
    PickLayoutTitle => ["Přidat do rozložení", "Add to layout"],
    PickLayoutButton => ["Přidat", "Add"],
    LayoutCountLabel => ["Počet ploch po startu (nejméně {0}):", "Number of desktops at startup (at least {0}):"],
    LayoutRow => ["Plocha {0} – {1}", "Desktop {0} – {1}"],
    LayoutFull => ["V rozložení může být nejvýše {0} aplikací.", "The layout can hold at most {0} apps."],
    FileFilterPrograms => ["Programy (*.exe)", "Programs (*.exe)"],
    FileFilterAll => ["Všechny soubory (*.*)", "All files (*.*)"],
    FileChooseApp => ["Vybrat aplikaci", "Choose an app"],
    // About dialog and support reminder
    AboutVersion => ["Verze {0}", "Version {0}"],
    AboutVariant => ["Varianta Windows: {0}", "Windows variant: {0}"],
    AboutVariantNone => [
        "Varianta Windows: nebyla rozpoznána, aplikace je neaktivní",
        "Windows variant: not recognized, the app is inactive"
    ],
    AboutWeb => ["Web: <a href=\"{0}\">{1}</a>", "Website: <a href=\"{0}\">{1}</a>"],
    PayRevolutCaption => ["Revolut: naskenujte QR kód\n{0}", "Revolut: scan the QR code\n{0}"],
    PayAccountCaption => [
        "Platba na účet: naskenujte QR kód v bankovní aplikaci.",
        "Bank transfer: scan the QR code in your banking app."
    ],
    CopyDone => ["Adresa je zkopírována do schránky.", "The address has been copied to the clipboard."],
    CopyFailed => ["Schránku se nepodařilo otevřít.", "Could not open the clipboard."],
    WalletOpenedCopied => [
        "Adresa je zkopírována do schránky, peněženka se otevírá.",
        "The address has been copied to the clipboard and your wallet is opening."
    ],
    WalletOpened => ["Peněženka se otevírá.", "Your wallet is opening."],
    WalletMissingCopied => [
        "Žádná peněženka není nastavená pro odkazy lightning:. Adresa je zkopírována do schránky.",
        "No wallet is set up to open lightning: links. The address has been copied to the clipboard."
    ],
    WalletMissing => [
        "Žádná peněženka není nastavená pro odkazy lightning:.",
        "No wallet is set up to open lightning: links."
    ],
    SupportTitle => ["AutoSpaces – podpořte autora", "AutoSpaces – support the author"],
    SupportSummary => [
        "Líbí se vám AutoSpaces? Podpořte prosím autora zasláním jakékoli částky jednou z možností níže.",
        "Enjoying AutoSpaces? Please support the author by sending any amount using one of the options below."
    ],
}

/// Text of `key` in `lang`.
pub fn tr(lang: Lang, key: Key) -> &'static str {
    TABLE[key as usize][lang as usize]
}

/// Text of `key` in the current language.
pub fn t(key: Key) -> &'static str {
    tr(current(), key)
}

/// `{0}`, `{1}` ... replaced by `args`.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut s = template.to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}

/// Text of `key` in the current language with `{0}`, `{1}` ... replaced by `args`.
pub fn tf(key: Key, args: &[&str]) -> String {
    fill(t(key), args)
}

/// Pointer to the dialog template `id` in the current language (falls back to the other
/// languages, so a missing variant never prevents a dialog from opening).
unsafe fn template(id: u16) -> *const DLGTEMPLATE {
    let hinst = GetModuleHandleW(std::ptr::null());
    let order = std::iter::once(current()).chain(LANGS.iter().copied().filter(|l| *l != current()));
    for lang in order {
        let res = FindResourceExW(hinst, RT_DIALOG, std::ptr::without_provenance::<u16>(id as usize), lang.langid());
        if res.is_null() {
            continue;
        }
        let data = LoadResource(hinst, res);
        if !data.is_null() {
            let p = LockResource(data);
            if !p.is_null() {
                return p as *const DLGTEMPLATE;
            }
        }
    }
    std::ptr::null()
}

/// Modal dialog `id` in the current language.
pub unsafe fn dialog_box(id: u16, parent: HWND, proc: DLGPROC, param: LPARAM) -> isize {
    let hinst = GetModuleHandleW(std::ptr::null());
    let tpl = template(id);
    if tpl.is_null() {
        return DialogBoxParamW(hinst, std::ptr::without_provenance::<u16>(id as usize), parent, proc, param);
    }
    windows_sys::Win32::UI::WindowsAndMessaging::DialogBoxIndirectParamW(hinst, tpl, parent, proc, param)
}

/// Modeless dialog `id` in the current language (null on failure).
pub unsafe fn create_dialog(id: u16, parent: HWND, proc: DLGPROC, param: LPARAM) -> HWND {
    CreateDialogIndirectParamW(GetModuleHandleW(std::ptr::null()), template(id), parent, proc, param)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn czech_ui_gives_czech_everything_else_english() {
        assert_eq!(detect(0x0405), Lang::Cs); // cs-CZ
        assert_eq!(detect(0x0409), Lang::En); // en-US
        assert_eq!(detect(0x0809), Lang::En); // en-GB
        assert_eq!(detect(0x0407), Lang::En); // de-DE
        assert_eq!(detect(0x041B), Lang::En); // sk-SK is not Czech
        assert_eq!(detect(0), Lang::En);
        assert_eq!(detect(0x0005), Lang::Cs); // neutral Czech
    }

    #[test]
    fn setting_overrides_windows_language() {
        assert_eq!(resolve(Pref::Auto, 0x0405), Lang::Cs);
        assert_eq!(resolve(Pref::Auto, 0x0409), Lang::En);
        assert_eq!(resolve(Pref::Fixed(Lang::En), 0x0405), Lang::En);
        assert_eq!(resolve(Pref::Fixed(Lang::Cs), 0x0409), Lang::Cs);
    }

    #[test]
    fn preference_codes_and_combo_indexes_roundtrip() {
        for p in [Pref::Auto, Pref::Fixed(Lang::Cs), Pref::Fixed(Lang::En)] {
            assert_eq!(Pref::from_code(p.code()), p);
            assert_eq!(Pref::from_index(p.index()), p);
        }
        assert_eq!(Pref::from_code(" EN "), Pref::Fixed(Lang::En));
        assert_eq!(Pref::from_code("de"), Pref::Auto);
        assert_eq!(Pref::from_code(""), Pref::Auto);
        assert_eq!(Pref::from_index(99), Pref::Auto);
        for l in LANGS {
            assert_eq!(Lang::from_code(l.code()), Some(l));
            assert_eq!(detect(l.langid()), l);
        }
    }

    fn placeholders(s: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = s;
        while let Some(i) = rest.find('{') {
            if let Some(j) = rest[i..].find('}') {
                out.insert(rest[i..=i + j].to_string());
                rest = &rest[i + j + 1..];
            } else {
                break;
            }
        }
        out
    }

    #[test]
    fn every_key_has_every_language_and_none_is_empty() {
        assert_eq!(ALL_KEYS.len(), TABLE.len());
        for &k in ALL_KEYS {
            for l in LANGS {
                assert!(!tr(l, k).trim().is_empty(), "{k:?} is empty in {l:?}");
            }
            // The same placeholders in every language.
            let first = placeholders(tr(LANGS[0], k));
            for l in LANGS {
                assert_eq!(placeholders(tr(l, k)), first, "{k:?}: placeholders differ in {l:?}");
            }
        }
    }

    #[test]
    fn placeholders_are_filled() {
        assert_eq!(fill("a {0} b {1} {0}", &["x", "y"]), "a x b y x");
        for l in LANGS {
            assert!(fill(tr(l, Key::LayoutRow), &["2", "Foo"]).contains("2"));
            assert!(!fill(tr(l, Key::LayoutRow), &["2", "Foo"]).contains('{'));
        }
    }

    /// `assets/dialogs.inc` uses `T(NAME)`; every language file must define `T_<LANG>_NAME` for each
    /// of them (and nothing else), non-empty.
    #[test]
    fn dialog_texts_are_complete_in_every_language() {
        let dir = format!("{}/assets", env!("CARGO_MANIFEST_DIR"));
        let dialogs = std::fs::read_to_string(format!("{dir}/dialogs.inc")).unwrap();
        let mut used = BTreeSet::new();
        let mut rest = dialogs.as_str();
        while let Some(i) = rest.find("T(") {
            let tail = &rest[i + 2..];
            let end = tail.find(')').unwrap();
            let name = &tail[..end];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                used.insert(name.to_string());
            }
            rest = &tail[end..];
        }
        assert!(used.len() > 40, "dialogs.inc seems to use too few texts: {}", used.len());
        for l in LANGS {
            let prefix = format!("T_{}_", l.code().to_ascii_uppercase());
            let file = std::fs::read_to_string(format!("{dir}/strings-{}.inc", l.code())).unwrap();
            let mut defined = BTreeSet::new();
            for line in file.lines() {
                if let Some(def) = line.strip_prefix("#define ").and_then(|d| d.strip_prefix(&prefix)) {
                    let (name, value) = def.split_once(' ').unwrap_or((def, ""));
                    let v = value.trim();
                    assert!(v.len() > 2 || v.parse::<i32>().is_ok(), "{prefix}{name} is empty");
                    // RC silently cuts control texts at 256 characters.
                    assert!(v.chars().count() <= 258, "{prefix}{name} is too long for RC; set it from the string table");
                    defined.insert(name.to_string());
                }
            }
            assert_eq!(used, defined, "texts of {l:?} differ from those used by dialogs.inc");
        }
    }
}
