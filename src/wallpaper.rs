//! Wallpapers of the desktops (plan 12.17), the host side: which call sets them on this system,
//! applying the settings, a random picture for new desktops, "Shuffle now". The choice itself is
//! `core::wallpaper` (pure, tested).
//!
//! * "Do not change" touches nothing at all.
//! * Exiting AutoSpaces does not give the old wallpapers back: they are the user's settings now.
//! * Undocumented setters only on a variant that passed the self-test (`InternalBackend` exists
//!   only then), never while a window is carried (the overlay shows the wallpapers).

use crate::core::settings::{WallMode, Wallpapers};
use crate::core::wallpaper::{self as wp, Assigned, Caps, Rng};
use crate::desktops::{self, DesktopBackend, Guid, InternalBackend};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Posted to the main window by "Shuffle now" (a COM call from the button handler itself is
/// refused by Explorer, plan 12.12).
pub const WM_WALL_SHUFFLE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 60;

static MAIN: AtomicUsize = AtomicUsize::new(0);
// Touched only from the UI thread.
static mut ASSIGNED: Option<Assigned> = None;
/// Desktops already seen by `sync` (None = not started yet).
static mut KNOWN: Option<Vec<Guid>> = None;
/// Settings of a pending "Shuffle now" (from the open settings dialog).
static mut PENDING: Option<Wallpapers> = None;

pub fn set_main(hwnd: windows_sys::Win32::Foundation::HWND) {
    MAIN.store(hwnd as usize, Relaxed);
}

fn file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(Path::new(&base).join("AutoSpaces").join("wallpapers.json"))
}

fn assigned() -> &'static mut Assigned {
    unsafe {
        let a = &mut *std::ptr::addr_of_mut!(ASSIGNED);
        a.get_or_insert_with(|| file().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Assigned::from_json(&t)).unwrap_or_default())
    }
}

fn save_assigned() {
    if let Some(p) = file() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, assigned().to_json()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
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

/// One picture on every desktop. Ok = what was used, for the log.
fn set_all(b: Option<&InternalBackend>, c: Caps, list: &[Guid], path: &str) -> Result<&'static str, String> {
    if let Some(b) = b {
        if c.all_desktops && b.set_all_wallpapers(path).is_ok() {
            return Ok("UpdateWallpaperPathForAllDesktops");
        }
        if c.per_desktop && !list.is_empty() && list.iter().all(|d| b.set_desktop_wallpaper(*d, path).is_ok()) {
            return Ok("SetDesktopWallpaper per desktop");
        }
    }
    desktops::internal::set_wallpaper_documented(path).map(|()| "IDesktopWallpaper::SetWallpaper").map_err(|e| e.to_string())
}

/// After a call: what Explorer recorded per desktop (Windows 11), for the log only.
fn note_registry(list: &[Guid], want: &[(Guid, String)], log: &mut dyn FnMut(String)) {
    let mut same = 0;
    let mut other = 0;
    for d in list {
        let Some(w) = want.iter().find(|x| x.0 == *d).map(|x| &x.1) else { continue };
        match crate::overlay_ui::registry_wallpaper(d) {
            Some(r) if r.eq_ignore_ascii_case(w) => same += 1,
            Some(_) => other += 1,
            None => {}
        }
    }
    log(format!("wallpaper: registry shows the picture on {same} desktop(s), another one on {other}"));
}

/// Applies the settings now: `Same` puts the picture on every desktop, `Random` gives every desktop
/// a random picture of the folder (`shuffle` = the same with new pictures). `Off`: nothing (the
/// record of given-out pictures is forgotten). Returns false when there was nothing to set.
pub fn apply(b: Option<&InternalBackend>, w: &Wallpapers, log: &mut dyn FnMut(String)) -> bool {
    let c = caps(b);
    let list: Vec<Guid> = b.and_then(|b| b.desktops().ok()).unwrap_or_default();
    let done = match w.mode {
        WallMode::Off => {
            assigned().clear();
            save_assigned();
            return false;
        }
        WallMode::Same => {
            if !Path::new(&w.image).is_file() {
                log("wallpaper: the picture does not exist; nothing set".into());
                return false;
            }
            match set_all(b, c, &list, &w.image) {
                Ok(how) => {
                    log(format!("wallpaper: {} on all {} desktop(s) ({how})", short(&w.image), list.len()));
                    let want: Vec<(Guid, String)> = list.iter().map(|d| (*d, w.image.clone())).collect();
                    note_registry(&list, &want, log);
                    true
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
            let plan = wp::plan_random(c, &list, &list, assigned(), &images, &mut Rng::new(seed()));
            give(b, c, &list, &plan, images.len(), log)
        }
    };
    unsafe { *std::ptr::addr_of_mut!(KNOWN) = Some(list) };
    if done {
        crate::overlay_ui::wallpapers_changed();
    }
    done
}

/// Sets a planned random choice and records it.
fn give(b: Option<&InternalBackend>, c: Caps, list: &[Guid], plan: &[(Guid, String)], of: usize, log: &mut dyn FnMut(String)) -> bool {
    let Some(first) = plan.first() else { return false };
    if !c.per_desktop {
        // One shared picture (Windows 10, or a Windows 11 build without known slots).
        return match set_all(b, c, list, &first.1) {
            Ok(how) => {
                log(format!("wallpaper: random {} (of {of}) for all desktops ({how})", short(&first.1)));
                for (d, p) in plan {
                    assigned().set(*d, p);
                }
                save_assigned();
                true
            }
            Err(e) => {
                log(format!("wallpaper: could not be set: {e}"));
                false
            }
        };
    }
    let Some(b) = b else { return false };
    let mut ok = 0;
    for (d, p) in plan {
        match b.set_desktop_wallpaper(*d, p) {
            Ok(()) => {
                ok += 1;
                assigned().set(*d, p);
                let k = list.iter().position(|x| x == d).map_or("?".into(), |i| (i + 1).to_string());
                log(format!("wallpaper: desktop #{k} {d} -> {} (random of {of})", short(p)));
            }
            Err(e) => {
                log(format!("wallpaper: desktop {d}: {e}"));
                break; // switched off for this run, or Explorer is gone: no more calls
            }
        }
    }
    if ok > 0 {
        assigned().retain(list);
        save_assigned();
        note_registry(list, plan, log);
    }
    ok > 0
}

/// After every event: a desktop that appeared (created by AutoSpaces, the profile, a carry, or by
/// hand: the registry change brings it here) gets a wallpaper as the settings say. Cheap when
/// nothing changed (the orchestrator's own list, no call into Explorer). Never during a carry.
pub fn sync(b: &InternalBackend, list: &[Guid], carrying: bool, w: &Wallpapers, log: &mut dyn FnMut(String)) {
    if carrying || list.is_empty() {
        return;
    }
    let known = unsafe { &mut *std::ptr::addr_of_mut!(KNOWN) };
    let first = known.is_none();
    if known.as_deref() == Some(list) {
        return;
    }
    let before = known.replace(list.to_vec()).unwrap_or_default();
    let c = caps(Some(b));
    if w.mode == WallMode::Off || !c.per_desktop {
        return; // one wallpaper for all: a new desktop shows it anyway
    }
    let new: Vec<Guid> = list.iter().filter(|d| !before.contains(d)).copied().collect();
    let changed = match w.mode {
        WallMode::Same if !first && !new.is_empty() && Path::new(&w.image).is_file() => {
            let mut n = 0;
            for d in &new {
                if b.set_desktop_wallpaper(*d, &w.image).is_ok() {
                    n += 1;
                }
            }
            log(format!("wallpaper: {} on {n} new desktop(s)", short(&w.image)));
            n > 0
        }
        WallMode::Random if w.new_desktops => {
            // New ones, and at the start the desktops that never got a picture from us.
            let targets: Vec<Guid> =
                list.iter().filter(|d| !assigned().map.contains_key(d) && (first || new.contains(d))).copied().collect();
            if targets.is_empty() {
                false
            } else {
                let images = wp::list_images(Path::new(&w.folder), w.subfolders);
                let plan = wp::choose(list, &targets, &assigned().map, &images, &mut Rng::new(seed()));
                give(Some(b), c, list, &plan, images.len(), log)
            }
        }
        _ => false,
    };
    if changed {
        crate::overlay_ui::wallpapers_changed();
    }
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
