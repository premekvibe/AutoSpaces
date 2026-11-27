//! Wallpapers of the desktops (plan 12.17), the host side: which call sets them on this system,
//! applying the settings, a random picture for new desktops, "Shuffle now". The choice itself is
//! `core::wallpaper` (pure, tested).
//!
//! * "Do not change" touches nothing at all.
//! * A wallpaper is set only when what a desktop should show changes: the settings changed,
//!   "Shuffle now", or a desktop appeared. Switching desktops (which changes the registry key
//!   AutoSpaces watches) never sets anything. Before every call the picture Explorer records for
//!   the desktop is compared (case-insensitive path): when it already is the one, nothing is called
//!   (a call with the same picture still makes Explorer repaint the background).
//! * With per-desktop support, a random choice never touches the session's single wallpaper
//!   (`IDesktopWallpaper::SetWallpaper`).
//! * Exiting AutoSpaces does not give the old wallpapers back: they are the user's settings now.
//! * Undocumented setters only on a variant that passed the self-test (`InternalBackend` exists
//!   only then), never while a window is carried (the overlay shows the wallpapers).

use crate::core::settings::{WallMode, Wallpapers};
use crate::core::wallpaper::{self as wp, Assigned, Caps, Rng};
use crate::desktops::{self, DesktopBackend, Guid, InternalBackend};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Posted to the main window by "Shuffle now" (a COM call from the button handler itself is
/// refused by Explorer, plan 12.12).
pub const WM_WALL_SHUFFLE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 60;

static MAIN: AtomicUsize = AtomicUsize::new(0);
// Touched only from the UI thread.
static mut HOST: Option<Host> = None;
/// Settings of a pending "Shuffle now" (from the open settings dialog).
static mut PENDING: Option<Wallpapers> = None;

pub fn set_main(hwnd: windows_sys::Win32::Foundation::HWND) {
    MAIN.store(hwnd as usize, Relaxed);
}

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(Path::new(&base).join("AutoSpaces").join("wallpapers.json"))
}

fn host() -> &'static mut Host {
    unsafe { (*std::ptr::addr_of_mut!(HOST)).get_or_insert_with(|| Host::load(file())) }
}

fn seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    t ^ (std::process::id() as u64) << 32
}

/// How the wallpapers can be set here: per desktop / all desktops only with the slots of a
/// self-tested variant; otherwise only the documented call (one picture for everything).
pub fn caps(b: Option<&InternalBackend>) -> Caps {
    let (per, all) = b.map_or((false, false), |b| b.wallpaper_slots());
    Caps { per_desktop: per, all_desktops: all }
}

/// The note under the choice in the settings dialog (which option works how on this Windows).
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

/// The state of the wallpaper feature: pictures given out (by desktop GUID) and the desktops seen.
pub struct Host {
    assigned: Assigned,
    /// Desktops already seen by `sync` (None = not started yet).
    known: Option<Vec<Guid>>,
    /// `wallpapers.json`; None in tests.
    file: Option<PathBuf>,
    /// Fixed seed (tests); None = the clock.
    seed: Option<u64>,
}

impl Host {
    fn load(file: Option<PathBuf>) -> Host {
        let assigned = file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Assigned::from_json(&t)).unwrap_or_default();
        Host { assigned, known: None, file, seed: None }
    }

    fn rng(&self) -> Rng {
        Rng::new(self.seed.unwrap_or_else(seed))
    }

    fn save(&self) {
        if let Some(p) = &self.file {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, self.assigned.to_json()).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }

    /// One desktop gets `path`, unless Explorer already records it (then nothing is called).
    /// Ok(true) = set now.
    fn put(s: &dyn WallSys, d: Guid, path: &str) -> Result<bool, String> {
        if s.recorded(d).is_some_and(|r| same_path(&r, path)) {
            return Ok(false);
        }
        s.set_desktop(d, path).map(|()| true)
    }

    /// One picture on every desktop; desktops that already show it are left alone.
    fn put_all(s: &dyn WallSys, c: Caps, list: &[Guid], path: &str) -> Result<&'static str, String> {
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

    /// Applies the settings now: `Same` puts the picture on every desktop, `Random` gives every
    /// desktop a new random picture of the folder ("Shuffle now" is the same). `Off`: nothing (the
    /// record of given-out pictures is forgotten). False when nothing was set.
    pub fn apply(&mut self, s: &dyn WallSys, w: &Wallpapers, log: &mut dyn FnMut(String)) -> bool {
        let c = s.caps();
        let list = s.desktops();
        let done = match w.mode {
            WallMode::Off => {
                self.assigned.clear();
                self.save();
                return false;
            }
            WallMode::Same => {
                if !Path::new(&w.image).is_file() {
                    log("wallpaper: the picture does not exist; nothing set".into());
                    return false;
                }
                match Self::put_all(s, c, &list, &w.image) {
                    Ok(how) => {
                        log(format!("wallpaper: {} on all {} desktop(s) ({how})", short(&w.image), list.len()));
                        how != "already set"
                    }
                    Err(e) => {
                        log(format!("wallpaper: could not be set: {e}"));
                        false
                    }
                }
            }
            WallMode::Random => {
                let images = wp::list_images(Path::new(&w.folder), w.subfolders);
                if images.is_empty() {
                    log("wallpaper: no pictures (.jpg, .jpeg, .png, .bmp) in the folder; nothing set".into());
                    return false;
                }
                let plan = wp::plan_random(c, &list, &list, &self.assigned, &images, &mut self.rng());
                self.give(s, c, &list, &plan, images.len(), log)
            }
        };
        self.known = Some(list);
        if done {
            crate::overlay_ui::wallpapers_changed();
        }
        done
    }

    /// Sets a planned random choice and records it.
    fn give(&mut self, s: &dyn WallSys, c: Caps, list: &[Guid], plan: &[(Guid, String)], of: usize, log: &mut dyn FnMut(String)) -> bool {
        let Some(first) = plan.first() else { return false };
        if !c.per_desktop {
            // One shared picture (Windows 10, or a Windows 11 build without known slots).
            return match Self::put_all(s, c, list, &first.1) {
                Ok(how) => {
                    log(format!("wallpaper: random {} (of {of}) for all desktops ({how})", short(&first.1)));
                    for (d, p) in plan {
                        self.assigned.set(*d, p);
                    }
                    self.save();
                    how != "already set"
                }
                Err(e) => {
                    log(format!("wallpaper: could not be set: {e}"));
                    false
                }
            };
        }
        let mut set = 0;
        let mut ok = 0;
        for (d, p) in plan {
            match Self::put(s, *d, p) {
                Ok(called) => {
                    ok += 1;
                    set += called as usize;
                    self.assigned.set(*d, p);
                    let k = list.iter().position(|x| x == d).map_or("?".into(), |i| (i + 1).to_string());
                    let how = if called { "" } else { ", already shown" };
                    log(format!("wallpaper: desktop #{k} {d} -> {} (random of {of}{how})", short(p)));
                }
                Err(e) => {
                    log(format!("wallpaper: desktop {d}: {e}"));
                    break; // switched off for this run, or Explorer is gone: no more calls
                }
            }
        }
        if ok > 0 {
            self.assigned.retain(list);
            self.save();
        }
        set > 0
    }

    /// After every event: only a desktop that appeared (created by AutoSpaces, the profile, a
    /// carry, or by hand) gets a wallpaper as the settings say. A switch of the desktop, any other
    /// event or registry change leaves the list as it was: nothing is called. Never during a carry
    /// (caught up after the drop).
    pub fn sync(&mut self, s: &dyn WallSys, list: &[Guid], carrying: bool, w: &Wallpapers, log: &mut dyn FnMut(String)) -> bool {
        if carrying || list.is_empty() || self.known.as_deref() == Some(list) {
            return false;
        }
        let first = self.known.is_none();
        let before = self.known.replace(list.to_vec()).unwrap_or_default();
        let c = s.caps();
        if w.mode == WallMode::Off || !c.per_desktop {
            return false; // one wallpaper for all: a new desktop shows it anyway
        }
        let new: Vec<Guid> = list.iter().filter(|d| !before.contains(d)).copied().collect();
        let changed = match w.mode {
            WallMode::Same if !first && !new.is_empty() && Path::new(&w.image).is_file() => {
                let n = new.iter().filter(|d| Self::put(s, **d, &w.image) == Ok(true)).count();
                if n > 0 {
                    log(format!("wallpaper: {} on {n} new desktop(s)", short(&w.image)));
                }
                n > 0
            }
            WallMode::Random if w.new_desktops => {
                // New ones, and at the start the desktops that never got a picture from us.
                let targets: Vec<Guid> =
                    list.iter().filter(|d| !self.assigned.map.contains_key(d) && (first || new.contains(d))).copied().collect();
                if targets.is_empty() {
                    false
                } else {
                    let images = wp::list_images(Path::new(&w.folder), w.subfolders);
                    let plan = wp::choose(list, &targets, &self.assigned.map, &images, &mut self.rng());
                    self.give(s, c, list, &plan, images.len(), log)
                }
            }
            _ => false,
        };
        if changed {
            crate::overlay_ui::wallpapers_changed();
        }
        changed
    }
}

/// Applies the settings now (OK in the settings dialog, "Shuffle now").
pub fn apply(b: Option<&InternalBackend>, w: &Wallpapers, log: &mut dyn FnMut(String)) -> bool {
    host().apply(&Sys(b), w, log)
}

/// After every event (cheap when the desktop list did not change: no call at all).
pub fn sync(b: &InternalBackend, list: &[Guid], carrying: bool, w: &Wallpapers, log: &mut dyn FnMut(String)) {
    host().sync(&Sys(Some(b)), list, carrying, w, log);
}

/// "Shuffle now" in the settings dialog: remembered and posted to the main window.
pub fn request_shuffle(w: Wallpapers) {
    unsafe { *std::ptr::addr_of_mut!(PENDING) = Some(w) };
    let main = MAIN.load(Relaxed);
    if main != 0 {
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(main as _, WM_WALL_SHUFFLE, 0, 0) };
    }
}

/// The posted "Shuffle now".
pub fn take_pending() -> Option<Wallpapers> {
    unsafe { (*std::ptr::addr_of_mut!(PENDING)).take() }
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
        list: RefCell<Vec<Guid>>,
        caps: Caps,
        recorded: RefCell<HashMap<Guid, String>>,
        global: RefCell<Option<String>>,
        /// ("desktop" | "all" | "documented", path)
        calls: RefCell<Vec<(&'static str, String)>>,
    }

    impl Fake {
        fn new(n: u32, per: bool) -> Fake {
            Fake {
                list: RefCell::new((1..=n).map(g).collect()),
                caps: Caps { per_desktop: per, all_desktops: per },
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
            self.list.borrow().clone()
        }
        fn caps(&self) -> Caps {
            self.caps
        }
        fn set_desktop(&self, d: Guid, path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(("desktop", path.into()));
            self.recorded.borrow_mut().insert(d, path.into());
            Ok(())
        }
        fn set_all(&self, path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(("all", path.into()));
            for d in self.list.borrow().iter() {
                self.recorded.borrow_mut().insert(*d, path.into());
            }
            Ok(())
        }
        fn set_documented(&self, path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(("documented", path.into()));
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

    /// A folder with `n` pictures (removed when the test ends).
    struct Pics(PathBuf);
    impl Pics {
        fn new(tag: &str, n: usize) -> Pics {
            let dir = std::env::temp_dir().join(format!("autospaces-wallhost-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for i in 0..n {
                std::fs::write(dir.join(format!("p{i}.jpg")), b"x").unwrap();
            }
            Pics(dir)
        }
        fn random(&self) -> Wallpapers {
            Wallpapers { mode: WallMode::Random, folder: self.0.to_string_lossy().into_owned(), ..Wallpapers::default() }
        }
        fn same(&self) -> Wallpapers {
            Wallpapers { mode: WallMode::Same, image: self.0.join("p0.jpg").to_string_lossy().into_owned(), ..Wallpapers::default() }
        }
    }
    impl Drop for Pics {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn host() -> Host {
        Host { assigned: Assigned::default(), known: None, file: None, seed: Some(7) }
    }

    fn quiet() -> impl FnMut(String) {
        |_| {}
    }

    #[test]
    fn random_per_desktop_sets_each_desktop_once_and_never_the_global_wallpaper() {
        let pics = Pics::new("a", 6);
        let (f, mut h) = (Fake::new(3, true), host());
        assert!(h.apply(&f, &pics.random(), &mut quiet()));
        assert_eq!(f.count(), 3, "shuffle = one call per desktop");
        assert!(f.calls.borrow().iter().all(|c| c.0 == "desktop"), "no global SetWallpaper: {:?}", f.calls.borrow());
    }

    #[test]
    fn switching_desktops_and_other_events_set_nothing() {
        let pics = Pics::new("b", 6);
        let (f, mut h) = (Fake::new(3, true), host());
        let w = pics.random();
        h.apply(&f, &w, &mut quiet());
        let after_apply = f.count();
        // A switch changes the registry key (poll) and many window events follow: the list of
        // desktops stays the same, so nothing is called, however often.
        let list = f.desktops();
        for _ in 0..50 {
            assert!(!h.sync(&f, &list, false, &w, &mut quiet()));
        }
        assert_eq!(f.count(), after_apply, "a switch = 0 calls");
        // The same after a restart of AutoSpaces with the pictures already given out.
        let mut h2 = Host { assigned: h.assigned.clone(), known: None, file: None, seed: Some(1) };
        assert!(!h2.sync(&f, &list, false, &w, &mut quiet()));
        for _ in 0..10 {
            h2.sync(&f, &list, false, &w, &mut quiet());
        }
        assert_eq!(f.count(), after_apply, "start + switches = 0 calls");
    }

    #[test]
    fn a_new_desktop_gets_one_call_and_shuffle_n() {
        let pics = Pics::new("c", 6);
        let (f, mut h) = (Fake::new(3, true), host());
        let w = pics.random();
        h.apply(&f, &w, &mut quiet());
        let base = f.count();
        f.list.borrow_mut().push(g(4));
        let list = f.desktops();
        assert!(h.sync(&f, &list, false, &w, &mut quiet()));
        assert_eq!(f.count(), base + 1, "a new desktop = 1 call");
        assert_ne!(f.recorded(g(4)), f.recorded(g(3)), "differs from its neighbour");
        h.sync(&f, &list, false, &w, &mut quiet());
        assert_eq!(f.count(), base + 1, "and nothing more afterwards");
        // During a carry it waits, then catches up once.
        f.list.borrow_mut().push(g(5));
        let list = f.desktops();
        assert!(!h.sync(&f, &list, true, &w, &mut quiet()));
        assert_eq!(f.count(), base + 1);
        h.sync(&f, &list, false, &w, &mut quiet());
        assert_eq!(f.count(), base + 2);
        // "Shuffle now": every desktop a new picture = N calls.
        let n = f.desktops().len();
        h.apply(&f, &w, &mut quiet());
        assert_eq!(f.count(), base + 2 + n, "shuffle = N calls");
        // With "new desktops" off a new desktop gets nothing.
        let off = Wallpapers { new_desktops: false, ..w.clone() };
        f.list.borrow_mut().push(g(6));
        let list = f.desktops();
        h.sync(&f, &list, false, &off, &mut quiet());
        assert_eq!(f.count(), base + 2 + n);
    }

    #[test]
    fn a_picture_explorer_already_records_is_not_set_again() {
        let pics = Pics::new("d", 3);
        let (f, mut h) = (Fake::new(3, true), host());
        let same = pics.same();
        assert!(h.apply(&f, &same, &mut quiet()));
        assert_eq!(f.count(), 1, "one call for all desktops");
        // Applying the same picture again (case of the path differs): nothing is called.
        let upper = Wallpapers { image: same.image.to_uppercase(), ..same.clone() };
        assert!(!h.apply(&f, &upper, &mut quiet()) || !Path::new(&upper.image).is_file());
        assert_eq!(f.count(), 1);
        // A new desktop that Windows gave the same picture already: no call.
        f.list.borrow_mut().push(g(4));
        f.recorded.borrow_mut().insert(g(4), same.image.clone());
        let list = f.desktops();
        h.sync(&f, &list, false, &same, &mut quiet());
        assert_eq!(f.count(), 1);
        // One that shows something else: one call for it.
        f.list.borrow_mut().push(g(5));
        f.recorded.borrow_mut().insert(g(5), "C:\\other.jpg".into());
        let list = f.desktops();
        h.sync(&f, &list, false, &same, &mut quiet());
        assert_eq!(f.count(), 2);
        assert_eq!(f.calls.borrow()[1].0, "desktop");
    }

    #[test]
    fn without_per_desktop_support_only_the_documented_call_and_never_on_events() {
        let pics = Pics::new("e", 4);
        let (f, mut h) = (Fake::new(3, false), host());
        let w = pics.random();
        assert!(h.apply(&f, &w, &mut quiet()));
        assert_eq!(f.calls.borrow().iter().map(|c| c.0).collect::<Vec<_>>(), vec!["documented"]);
        f.list.borrow_mut().push(g(4));
        let list = f.desktops();
        h.sync(&f, &list, false, &w, &mut quiet());
        assert_eq!(f.count(), 1, "a new desktop shows the one wallpaper anyway");
        let same = pics.same();
        h.apply(&f, &same, &mut quiet());
        h.apply(&f, &same, &mut quiet());
        assert_eq!(f.count(), 2, "the same global picture is not set twice");
    }

    #[test]
    fn off_touches_nothing() {
        let (f, mut h) = (Fake::new(3, true), host());
        let off = Wallpapers::default();
        assert!(!h.apply(&f, &off, &mut quiet()));
        f.list.borrow_mut().push(g(4));
        let list = f.desktops();
        h.sync(&f, &list, false, &off, &mut quiet());
        h.sync(&f, &list, false, &off, &mut quiet());
        assert_eq!(f.count(), 0);
    }
}
