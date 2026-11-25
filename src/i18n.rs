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
    TrayTipUnsupported => [
        "AutoSpaces – nepodporovaná verze Windows, nic se nepřesouvá",
        "AutoSpaces – unsupported Windows version, nothing is moved"
    ],
    TrayTipForcedOff => [
        "AutoSpaces – přesouvání vypnuto v settings.json",
        "AutoSpaces – moving is switched off in settings.json"
    ],
    MenuReport => ["Nahlásit problém…", "Report a problem…"],
    NoticeUnsupported => [
        "Tato verze Windows (sestavení {0}) není podporována, AutoSpaces proto nic nepřesouvá. Nahlásit ji můžete v nabídce ikony: Nahlásit problém.",
        "This version of Windows (build {0}) is not supported, so AutoSpaces moves nothing. You can report it in the tray menu: Report a problem."
    ],
    NoticeForcedOff => [
        "Rozhraní ploch je vypnuto klíčem \"backend\": \"{0}\" v settings.json, AutoSpaces nic nepřesouvá.",
        "The desktop interface is switched off by \"backend\": \"{0}\" in settings.json, AutoSpaces moves nothing."
    ],
    MenuEnabled => ["Zapnuto", "Enabled"],
    MenuLearned => ["Naučené výjimky", "Learned exceptions"],
    MenuNone => ["(žádné)", "(none)"],
    MenuForgetAll => ["Zapomenout vše", "Forget all"],
    MenuSettings => ["Nastavení…", "Settings…"],
    MenuAbout => ["O aplikaci…", "About…"],
    MenuExit => ["Ukončit", "Exit"],
    // Settings dialogs
    AppName => ["AutoSpaces", "AutoSpaces"],
    FileExplorer => ["Průzkumník souborů", "File Explorer"],
    LangAuto => ["Automaticky", "Automatic"],
    // Tabs of the settings dialog
    TabGeneral => ["Obecné", "General"],
    TabApps => ["Aplikace", "Apps"],
    TabAdvanced => ["Pokročilé", "Advanced"],
    TabKeys => ["Klávesy", "Keys"],
    // Carrying a window (plan 12.13)
    CarryHotkeyBusy => [
        "Zkratku Win+Ctrl+Shift+←/→ používá jiná aplikace, přenášení okna je proto vypnuté.",
        "Another app uses Win+Ctrl+Shift+←/→, so carrying a window is off."
    ],
    CarryUnavailable => [
        "Přenášení okna není k dispozici: tato verze Windows není podporována.",
        "Carrying a window is not available: this version of Windows is not supported."
    ],
    CarryCannotMove => ["Toto okno nelze přesunout", "This window cannot be moved"],
    DesktopWord => ["Plocha", "Desktop"],
    // Long paragraphs of the dialogs (RC would cut them at 256 characters)
    SettingsHint => [
        "Aby klik na ikonu běžící aplikace přepnul na její plochu, nastavte ve Windows v části Plochy „Na hlavním panelu zobrazovat všechna otevřená okna“ na „Na všech plochách“. Shift + klik na ikonu spustí novou instanci aplikace.",
        "To make a click on a running app's taskbar icon switch to its desktop, open Settings > System > Multitasking in Windows and, under Desktops, set “On the taskbar, show all the open windows” to “On all desktops”. Shift + click on the icon starts a new instance of the app."
    ],
    LayoutInfo => [
        "Při startu AutoSpaces doplní počet ploch, okna běžících aplikací přesune na zvolené plochy a ostatní aplikace spustí. Okno, které se do 30 sekund neobjeví, se vzdá a zapíše do protokolu. Pohled zůstane na vaší ploše.",
        "At startup, AutoSpaces adds any missing desktops, moves the windows of running apps to the chosen desktops and launches the other apps. If a window does not appear within 30 seconds, AutoSpaces gives up on it and notes it in the log. Your view stays on your own desktop."
    ],
    ReportIntro => [
        "AutoSpaces vytvoří textový soubor report.txt s údaji, které pomohou chybu najít: verze AutoSpaces a Windows, nastavení, naučené výjimky a poslední řádky záznamu činnosti. Neobsahuje titulky oken ani jméno uživatele či počítače a nikam se neodesílá. Před odesláním si ho můžete prohlédnout. Pak otevřete hlášení na GitHubu a soubor do něj přetáhněte.",
        "AutoSpaces will create a text file, report.txt, with information that helps to find the problem: the AutoSpaces and Windows versions, your settings, learned exceptions and the last lines of the activity log. It contains no window titles and no user or computer name, and it is not sent anywhere. You can look at it before you send it. Then open the report form on GitHub and drag the file into it."
    ],
    ReportLogWarn => [
        "Záznam činnosti je vypnutý. Pro užitečnou diagnostiku ho zapněte, chybu zopakujte a teprve potom soubor vytvořte.",
        "Activity logging is off. For a useful report, turn it on, repeat the problem and only then create the file."
    ],
    ReportLogNowOn => [
        "Záznam činnosti je zapnutý. Nyní chybu zopakujte a potom soubor vytvořte.",
        "Activity logging is on. Now repeat the problem, then create the file."
    ],
    ReportCreated => [
        "Soubor report.txt je vytvořen a ukázán ve složce. Přetáhněte ho do hlášení na GitHubu.",
        "report.txt has been created and is shown in its folder. Drag it into your report on GitHub."
    ],
    ReportFailed => ["Soubor se nepodařilo vytvořit.", "The file could not be created."],
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
    AboutVariantUnverified => ["Varianta Windows: {0} (neověřená)", "Windows variant: {0} (unverified)"],
    AboutUnsupported => [
        "Sestavení Windows {0} není podporováno, aplikace nic nepřesouvá.",
        "Windows build {0} is not supported; the app moves nothing."
    ],
    AboutForcedOff => [
        "Rozhraní ploch vypnuto v settings.json, nic se nepřesouvá.",
        "Desktop interface off in settings.json; nothing is moved."
    ],
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
    // Updates (plan 12.15)
    MenuCheckUpdates => ["Zkontrolovat aktualizace", "Check for updates"],
    NoticeUpdate => [
        "Je k dispozici AutoSpaces {0}. Klikněte pro instalaci.",
        "AutoSpaces {0} is available. Click to install."
    ],
    NoticeUpdatePage => [
        "Je k dispozici AutoSpaces {0}. Klikněte pro podrobnosti.",
        "AutoSpaces {0} is available. Click for details."
    ],
    UpdateAvailable => [
        "Je k dispozici AutoSpaces {0}. Nainstalovaná verze je {1}.",
        "AutoSpaces {0} is available. Your version is {1}."
    ],
    UpdateWhatsNew => ["<a>Co je nového ve verzi {0}</a>", "<a>What's new in version {0}</a>"],
    UpdateInstallNote => [
        "Instalace proběhne na pozadí: AutoSpaces se ukončí (vrátí okna a plochy), nainstaluje se nová verze a AutoSpaces se znovu spustí. Nastavení zůstane zachováno.",
        "The update installs in the background: AutoSpaces exits (giving windows and desktops back), the new version is installed and AutoSpaces starts again. Your settings are kept."
    ],
    UpdateAdminNote => [
        "AutoSpaces je nainstalovaný pro všechny uživatele, Windows proto požádá o oprávnění správce.",
        "AutoSpaces is installed for all users, so Windows will ask for administrator permission."
    ],
    UpdatePortableNote => [
        "Tato kopie AutoSpaces nebyla nainstalována instalátorem, proto se nemůže aktualizovat sama. Novou verzi stáhněte ze stránky vydání.",
        "This copy of AutoSpaces was not installed by the installer, so it cannot update itself. Download the new version from the release page."
    ],
    UpdateNoHash => [
        "U vydání chybí instalátor nebo jeho kontrolní součet SHA-256, aktualizaci proto nelze ověřit a nainstalovat automaticky. Novou verzi najdete na stránce vydání.",
        "The release lacks the installer or its SHA-256 checksum, so the update cannot be verified and installed automatically. You can find the new version on the release page."
    ],
    UpdateOpenPage => ["Otevřít stránku vydání", "Open release page"],
    UpdateDownloading => ["Stahuji instalátor… {0}", "Downloading the installer… {0}"],
    UpdateStarting => ["Spouštím instalátor…", "Starting the installer…"],
    UpdateCancel => ["Zrušit", "Cancel"],
    UpdateLater => ["Později", "Later"],
    UpdateDownloadFailed => [
        "Stažení se nezdařilo. Zkuste to znovu později, nebo novou verzi stáhněte ze stránky vydání.",
        "The download failed. Try again later, or download the new version from the release page."
    ],
    UpdateHashMismatch => [
        "Stažený soubor neodpovídá kontrolnímu součtu SHA-256 uvedenému u vydání. Nebyl spuštěn a byl smazán.",
        "The downloaded file does not match the SHA-256 checksum given with the release. It was not started and has been deleted."
    ],
    UpdateLaunchFailed => [
        "Instalátor se nepodařilo spustit. AutoSpaces běží dál beze změny.",
        "The installer could not be started. AutoSpaces keeps running unchanged."
    ],
    UpdateUpToDate => ["Máte nejnovější verzi AutoSpaces ({0}).", "You have the latest version of AutoSpaces ({0})."],
    UpdateCheckFailed => [
        "Kontrola aktualizací se nezdařila. Zkontrolujte připojení k internetu a zkuste to později.",
        "Checking for updates failed. Check your internet connection and try again later."
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
