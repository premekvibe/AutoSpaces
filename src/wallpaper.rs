//! Wallpapers of the desktops (plan 12.17), the host side: one picture on all desktops.
//!
//! * "Do not change" touches nothing at all.
//! * The picture is set only when the choice changes (OK in the settings dialog). Before the call
//!   the picture Explorer records for each desktop is compared (case-insensitive path): desktops
//!   that already show it are left alone (a call with the same picture still makes Explorer
//!   repaint the background). Switching desktops, window events and registry changes never set
//!   anything.
//! * Exiting AutoSpaces does not give the old wallpaper back: it is the user's setting now.
//! * Undocumented setters only on a variant that passed the self-test (`InternalBackend` exists
//!   only then).
//! * Random pictures of a folder were tried in 0.2.2 and taken out (Windows 11 repaints the whole
//!   desktop at every switch between desktops with different wallpapers); the code is on the branch
//!   `feature/random-wallpapers`.

use crate::core::settings::{WallMode, Wallpapers};
use crate::core::wallpaper::Caps;
use crate::desktops::{self, DesktopBackend, Guid, InternalBackend};
use std::path::Path;

/// How the wallpapers can be set here: per desktop / all desktops only with the slots of a
/// self-tested variant; otherwise only the documented call.
pub fn caps(b: Option<&InternalBackend>) -> Caps {
    let (per, all) = b.map_or((false, false), |b| b.wallpaper_slots());
    Caps { per_desktop: per, all_desktops: all }
}

/// The note under the choice in the settings dialog (how it works on this Windows).
pub fn status(c: Caps) -> String {
    use crate::i18n::{t, Key};
    let build = desktops::internal::os_build();
    let what = if c.per_desktop {
        t(Key::WallPerDesktop)
    } else if build >= 22000 {
        t(Key::WallWin11Unknown)
    } else {
        t(Key::WallWin10)
    };
    format!("{what} {}", t(Key::WallNoRestore))
}

/// File name only (folders may contain the user's name; the log never gets them).
fn short(p: &str) -> String {
    Path::new(p).file_name().map_or_else(|| "?".into(), |n| n.to_string_lossy().into_owned())
}

fn same_path(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// What the wallpaper code needs from the system (a fake in the tests counts the calls).
pub trait WallSys {
    fn desktops(&self) -> Vec<Guid>;
    fn caps(&self) -> Caps;
    /// Undocumented `SetDesktopWallpaper`.
    fn set_desktop(&self, d: Guid, path: &str) -> Result<(), String>;
    /// Undocumented `UpdateWallpaperPathForAllDesktops`.
    fn set_all(&self, path: &str) -> Result<(), String>;
    /// Documented `IDesktopWallpaper::SetWallpaper(NULL, path)`.
    fn set_documented(&self, path: &str) -> Result<(), String>;
    /// The picture Explorer records for a desktop (Windows 11), read only.
    fn recorded(&self, d: Guid) -> Option<String>;
    /// The session's single wallpaper, read only.
    fn global(&self) -> Option<String>;
}

struct Sys<'a>(Option<&'a InternalBackend>);

impl WallSys for Sys<'_> {
    fn desktops(&self) -> Vec<Guid> {
        self.0.and_then(|b| b.desktops().ok()).unwrap_or_default()
    }
    fn caps(&self) -> Caps {
        caps(self.0)
    }
    fn set_desktop(&self, d: Guid, path: &str) -> Result<(), String> {
        self.0.ok_or("no backend")?.set_desktop_wallpaper(d, path).map_err(|e| e.to_string())
    }
    fn set_all(&self, path: &str) -> Result<(), String> {
        self.0.ok_or("no backend")?.set_all_wallpapers(path).map_err(|e| e.to_string())
    }
    fn set_documented(&self, path: &str) -> Result<(), String> {
        desktops::internal::set_wallpaper_documented(path).map_err(|e| e.to_string())
    }
    fn recorded(&self, d: Guid) -> Option<String> {
        crate::overlay_ui::registry_wallpaper(&d)
    }
    fn global(&self) -> Option<String> {
        crate::overlay_ui::global_wallpaper()
    }
}

/// One picture on every desktop; desktops that already show it are left alone. Ok = how, for the
/// log ("already set" = nothing was called).
fn put_all(s: &dyn WallSys, path: &str) -> Result<&'static str, String> {
    let c = s.caps();
    let list = s.desktops();
    if c.per_desktop && !list.is_empty() {
        let missing: Vec<Guid> = list.iter().copied().filter(|d| !s.recorded(*d).is_some_and(|r| same_path(&r, path))).collect();
        if missing.is_empty() {
            return Ok("already set");
        }
        if c.all_desktops && s.set_all(path).is_ok() {
            return Ok("UpdateWallpaperPathForAllDesktops");
        }
        for d in missing {
            s.set_desktop(d, path)?;
        }
        return Ok("SetDesktopWallpaper per desktop");
    }
    if s.global().is_some_and(|g| same_path(&g, path)) {
        return Ok("already set");
    }
    s.set_documented(path).map(|()| "IDesktopWallpaper::SetWallpaper")
}

/// Applies `w` when it changed from `saved` (OK in the settings dialog). True when a picture was set.
pub fn apply_changed(s: &dyn WallSys, w: &Wallpapers, saved: &Wallpapers, log: &mut dyn FnMut(String)) -> bool {
    if !w.changed_from(saved) || w.mode != WallMode::Same {
        return false;
    }
    if !Path::new(&w.image).is_file() {
        log("wallpaper: the picture does not exist; nothing set".into());
        return false;
    }
    match put_all(s, &w.image) {
        Ok(how) => {
            log(format!("wallpaper: {} on all desktops ({how})", short(&w.image)));
            let set = how != "already set";
            if set {
                crate::overlay_ui::wallpapers_changed();
            }
            set
        }
        Err(e) => {
            log(format!("wallpaper: could not be set: {e}"));
            false
        }
    }
}

/// The settings dialog was confirmed.
pub fn settings_confirmed(b: Option<&InternalBackend>, w: &Wallpapers, saved: &Wallpapers, log: &mut dyn FnMut(String)) {
    apply_changed(&Sys(b), w, saved, log);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn g(n: u32) -> Guid {
        Guid { d1: n, ..Guid::ZERO }
    }

    /// Explorer as far as wallpapers go: what each desktop records, and every setter call.
    struct Fake {
        list: Vec<Guid>,
        caps: Caps,
        all_fails: bool,
        recorded: RefCell<HashMap<Guid, String>>,
        global: RefCell<Option<String>>,
        /// "desktop" | "all" | "documented"
        calls: RefCell<Vec<&'static str>>,
    }

    impl Fake {
        fn new(per: bool) -> Fake {
            Fake {
                list: (1..=3).map(g).collect(),
                caps: Caps { per_desktop: per, all_desktops: per },
                all_fails: false,
                recorded: RefCell::default(),
                global: RefCell::default(),
                calls: RefCell::default(),
            }
        }
        fn count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl WallSys for Fake {
        fn desktops(&self) -> Vec<Guid> {
            self.list.clone()
        }
        fn caps(&self) -> Caps {
            self.caps
        }
        fn set_desktop(&self, d: Guid, path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push("desktop");
            self.recorded.borrow_mut().insert(d, path.into());
            Ok(())
        }
        fn set_all(&self, path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push("all");
            if self.all_fails {
                return Err("E_FAIL".into());
            }
            for d in &self.list {
                self.recorded.borrow_mut().insert(*d, path.into());
            }
            Ok(())
        }
        fn set_documented(&self, path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push("documented");
            *self.global.borrow_mut() = Some(path.into());
            Ok(())
        }
        fn recorded(&self, d: Guid) -> Option<String> {
            self.recorded.borrow().get(&d).cloned()
        }
        fn global(&self) -> Option<String> {
            self.global.borrow().clone()
        }
    }

    /// An existing picture file (removed when the test ends).
    struct Pic(std::path::PathBuf);
    impl Pic {
        fn new(tag: &str) -> Pic {
            let p = std::env::temp_dir().join(format!("autospaces-wall-{tag}-{}.jpg", std::process::id()));
            std::fs::write(&p, b"x").unwrap();
            Pic(p)
        }
        fn same(&self) -> Wallpapers {
            Wallpapers { mode: WallMode::Same, image: self.0.to_string_lossy().into_owned() }
        }
    }
    impl Drop for Pic {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn quiet() -> impl FnMut(String) {
        |_| {}
    }

    #[test]
    fn one_call_for_all_desktops_and_never_again_for_the_same_picture() {
        let pic = Pic::new("a");
        let f = Fake::new(true);
        let (off, w) = (Wallpapers::default(), pic.same());
        assert!(apply_changed(&f, &w, &off, &mut quiet()));
        assert_eq!(*f.calls.borrow(), vec!["all"], "UpdateWallpaperPathForAllDesktops, no global call");
        // Unchanged settings: nothing. Off -> same again with the picture already shown: nothing.
        assert!(!apply_changed(&f, &w, &w, &mut quiet()));
        let upper = Wallpapers { image: w.image.to_uppercase(), ..w.clone() };
        assert!(!apply_changed(&f, &upper, &off, &mut quiet()), "case of the path does not matter");
        assert_eq!(f.count(), 1);
    }

    #[test]
    fn only_desktops_without_the_picture_get_it_when_the_all_call_fails() {
        let pic = Pic::new("b");
        let mut f = Fake::new(true);
        f.all_fails = true;
        let w = pic.same();
        f.recorded.borrow_mut().insert(g(2), w.image.clone());
        assert!(apply_changed(&f, &w, &Wallpapers::default(), &mut quiet()));
        assert_eq!(*f.calls.borrow(), vec!["all", "desktop", "desktop"], "desktop 2 already shows it");
    }

    #[test]
    fn without_per_desktop_support_the_documented_call_once() {
        let pic = Pic::new("c");
        let f = Fake::new(false);
        let w = pic.same();
        assert!(apply_changed(&f, &w, &Wallpapers::default(), &mut quiet()));
        assert!(!apply_changed(&f, &w, &Wallpapers::default(), &mut quiet()), "the global wallpaper already is it");
        assert_eq!(*f.calls.borrow(), vec!["documented"]);
    }

    #[test]
    fn off_missing_picture_and_unchanged_choice_touch_nothing() {
        let f = Fake::new(true);
        let off = Wallpapers::default();
        let missing = Wallpapers { mode: WallMode::Same, image: r"C:\does\not\exist.jpg".into() };
        assert!(!apply_changed(&f, &off, &missing, &mut quiet()));
        assert!(!apply_changed(&f, &missing, &off, &mut quiet()));
        assert_eq!(f.count(), 0);
    }
}
