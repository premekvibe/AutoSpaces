//! Wallpapers of the desktops (plan 12.17): which pictures of a folder count, which desktop gets
//! which picture, and what was given out (by desktop GUID, so a renumbering of the desktops never
//! changes a wallpaper). No Win32 here; the host sets the pictures.

use crate::desktops::Guid;
use crate::json::{parse_json, quote, Json};
use std::collections::HashMap;
use std::path::Path;

/// Picture types Windows shows as a wallpaper and every Windows decodes (WIC).
pub const IMAGE_EXTS: [&str; 4] = ["jpg", "jpeg", "png", "bmp"];
/// Sanity limits for huge folders: the listing stops there.
pub const MAX_IMAGES: usize = 5000;
const MAX_DEPTH: usize = 8;
/// Remembered desktops (more than anyone has; old entries go first).
const MAX_ASSIGNED: usize = 200;

/// A file name with one of `IMAGE_EXTS` (case does not matter; hidden dot files do not count).
pub fn is_image(name: &str) -> bool {
    let n = name.trim();
    if n.starts_with('.') {
        return false;
    }
    match n.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => IMAGE_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)),
        _ => false,
    }
}

/// Pictures of `folder` (with `subfolders` also below it, at most `MAX_DEPTH` deep), full paths,
/// sorted (a stable order, so the same seed gives the same choice). Unreadable entries are skipped.
pub fn list_images(folder: &Path, subfolders: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if subfolders && depth < MAX_DEPTH {
                    dirs.push((e.path(), depth + 1));
                }
            } else if ft.is_file() && is_image(&e.file_name().to_string_lossy()) {
                out.push(e.path().to_string_lossy().into_owned());
                if out.len() >= MAX_IMAGES {
                    out.sort();
                    return out;
                }
            }
        }
    }
    out.sort_by_key(|p| p.to_lowercase());
    out
}

/// Small deterministic generator (xorshift64*): tests use a fixed seed, the host the clock.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // splitmix64 of the seed: neighbouring seeds give unrelated streams; never zero.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)).max(1))
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// New pictures for the desktops `targets` (in the order of `desks`, the desktop list):
/// * a picture differs from the pictures of the neighbouring desktops whenever another one is
///   left, and from the picture the desktop had (so "shuffle" changes it) when that is possible;
/// * among the allowed pictures the least used one wins (every picture is used once before any is
///   used twice), ties are broken at random.
///
/// `have` = pictures given out so far (desktops outside `targets` keep theirs and count).
pub fn choose(desks: &[Guid], targets: &[Guid], have: &HashMap<Guid, String>, images: &[String], rng: &mut Rng) -> Vec<(Guid, String)> {
    if images.is_empty() || targets.is_empty() {
        return Vec::new();
    }
    let mut cur: HashMap<Guid, String> =
        desks.iter().filter(|d| !targets.contains(d)).filter_map(|d| have.get(d).map(|p| (*d, p.clone()))).collect();
    let mut used: HashMap<&str, usize> = HashMap::new();
    for p in cur.values() {
        if let Some(i) = images.iter().find(|i| *i == p) {
            *used.entry(i.as_str()).or_default() += 1;
        }
    }
    let mut order: Vec<Guid> = desks.iter().filter(|d| targets.contains(d)).copied().collect();
    order.extend(targets.iter().filter(|t| !desks.contains(t)).copied());
    let mut out = Vec::new();
    for t in order {
        let at = desks.iter().position(|d| *d == t);
        let near: Vec<&String> = at
            .map(|i| [i.checked_sub(1), Some(i + 1)].into_iter().flatten().filter_map(|k| desks.get(k)).filter_map(|d| cur.get(d)).collect())
            .unwrap_or_default();
        let before = have.get(&t);
        let allowed = |avoid_before: bool| -> Vec<&String> {
            images.iter().filter(|i| !near.contains(i) && !(avoid_before && Some(*i) == before)).collect()
        };
        let mut cand = allowed(true);
        if cand.is_empty() {
            cand = allowed(false);
        }
        if cand.is_empty() {
            cand = images.iter().collect();
        }
        let least = cand.iter().map(|i| used.get(i.as_str()).copied().unwrap_or(0)).min().unwrap_or(0);
        let best: Vec<&String> = cand.into_iter().filter(|i| used.get(i.as_str()).copied().unwrap_or(0) == least).collect();
        let pick = best[rng.below(best.len())].clone();
        *used.entry(images.iter().find(|i| **i == pick).map_or("", |s| s.as_str())).or_default() += 1;
        cur.insert(t, pick.clone());
        out.push((t, pick));
    }
    out
}

/// What AutoSpaces gave out: desktop GUID -> picture (`%LOCALAPPDATA%\AutoSpaces\wallpapers.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assigned {
    pub map: HashMap<Guid, String>,
    /// Insertion order, oldest first (for the size limit).
    order: Vec<Guid>,
}

impl Assigned {
    pub fn set(&mut self, d: Guid, path: &str) {
        self.order.retain(|x| *x != d);
        self.order.push(d);
        self.map.insert(d, path.to_string());
        while self.order.len() > MAX_ASSIGNED {
            let old = self.order.remove(0);
            self.map.remove(&old);
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    /// Forgets desktops that no longer exist.
    pub fn retain(&mut self, list: &[Guid]) {
        self.map.retain(|d, _| list.contains(d));
        self.order.retain(|d| list.contains(d));
    }

    /// Tolerant: anything unreadable is skipped.
    pub fn from_json(text: &str) -> Assigned {
        let mut a = Assigned::default();
        if let Ok(j) = parse_json(text) {
            if let Some(Json::Arr(list)) = j.get("assigned") {
                for e in list {
                    let g = e.get("desktop").and_then(Json::as_str).and_then(Guid::parse);
                    let p = e.get("wallpaper").and_then(Json::as_str).filter(|p| !p.trim().is_empty());
                    if let (Some(g), Some(p)) = (g, p) {
                        a.set(g, p);
                    }
                }
            }
        }
        a
    }

    pub fn to_json(&self) -> String {
        let rows: Vec<String> = self
            .order
            .iter()
            .filter_map(|d| self.map.get(d).map(|p| format!("  {{\"desktop\": {}, \"wallpaper\": {}}}", quote(&d.to_string()), quote(p))))
            .collect();
        if rows.is_empty() {
            "{\"assigned\": []}\n".into()
        } else {
            format!("{{\"assigned\": [\n{}\n]}}\n", rows.join(",\n"))
        }
    }
}

/// How the wallpapers can be set on this system (plan 12.17).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// One picture per desktop (`SetDesktopWallpaper` of a self-tested Windows 11 variant).
    pub per_desktop: bool,
    /// One picture on every desktop in one call (`UpdateWallpaperPathForAllDesktops`).
    pub all_desktops: bool,
}

/// What to do now: the desktops in `list` that get which picture. `Same` with per-desktop
/// support: every target the picture; without it the host sets one picture for all.
/// `Random` without per-desktop support: one random picture (index 0 is used for all).
pub fn plan_random(caps: Caps, list: &[Guid], targets: &[Guid], have: &Assigned, images: &[String], rng: &mut Rng) -> Vec<(Guid, String)> {
    if caps.per_desktop {
        choose(list, targets, &have.map, images, rng)
    } else if images.is_empty() {
        Vec::new()
    } else {
        // One shared picture: differs from the last one when possible.
        let last = list.first().and_then(|d| have.map.get(d));
        let cand: Vec<&String> = images.iter().filter(|i| Some(*i) != last).collect();
        let cand = if cand.is_empty() { images.iter().collect() } else { cand };
        let pick = cand[rng.below(cand.len())].clone();
        list.iter().map(|d| (*d, pick.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(n: u32) -> Guid {
        Guid { d1: n, ..Guid::ZERO }
    }

    fn imgs(n: usize) -> Vec<String> {
        (0..n).map(|i| format!(r"C:\w\{i}.jpg")).collect()
    }

    #[test]
    fn image_files_by_extension() {
        for n in ["a.jpg", "B.JPEG", "c.Png", "d.bmp", "x y.jpg"] {
            assert!(is_image(n), "{n}");
        }
        for n in ["a.gif", "b.webp", "c", ".jpg", "jpg", "d.jpg.txt", "e.heic", "f.", "desktop.ini"] {
            assert!(!is_image(n), "{n}");
        }
    }

    #[test]
    fn folder_listing_filters_sorts_and_recurses_on_request() {
        let dir = std::env::temp_dir().join(format!("autospaces-wall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub").join("deeper")).unwrap();
        for f in ["b.png", "A.jpg", "notes.txt", "c.gif", "sub/s.jpeg", "sub/deeper/d.bmp", "sub/x.doc"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let names = |v: Vec<String>| -> Vec<String> {
            v.iter().map(|p| Path::new(p).file_name().unwrap().to_string_lossy().into_owned()).collect()
        };
        assert_eq!(names(list_images(&dir, false)), ["A.jpg", "b.png"]);
        let mut deep = names(list_images(&dir, true));
        deep.sort();
        assert_eq!(deep, ["A.jpg", "b.png", "d.bmp", "s.jpeg"]);
        assert!(list_images(&dir.join("missing"), true).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enough_pictures_every_desktop_differs_and_none_repeats() {
        let desks: Vec<Guid> = (1..=5).map(g).collect();
        let images = imgs(8);
        for seed in 0..50 {
            let out = choose(&desks, &desks, &HashMap::new(), &images, &mut Rng::new(seed));
            assert_eq!(out.len(), 5);
            let mut p: Vec<&String> = out.iter().map(|o| &o.1).collect();
            p.sort();
            p.dedup();
            assert_eq!(p.len(), 5, "seed {seed}: no picture twice while others are unused");
        }
    }

    #[test]
    fn few_pictures_are_spread_evenly_and_neighbours_differ() {
        let desks: Vec<Guid> = (1..=7).map(g).collect();
        for n in [2usize, 3] {
            let images = imgs(n);
            for seed in 0..50 {
                let out = choose(&desks, &desks, &HashMap::new(), &images, &mut Rng::new(seed));
                for w in out.windows(2) {
                    assert_ne!(w[0].1, w[1].1, "{n} pictures, seed {seed}: neighbours differ");
                }
                let counts: Vec<usize> = images.iter().map(|i| out.iter().filter(|o| &o.1 == i).count()).collect();
                let (lo, hi) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
                assert!(hi - lo <= 1, "{n} pictures, seed {seed}: even {counts:?}");
            }
        }
        // One picture: every desktop gets it (nothing better is possible).
        let out = choose(&desks, &desks, &HashMap::new(), &imgs(1), &mut Rng::new(1));
        assert!(out.iter().all(|o| o.1 == imgs(1)[0]) && out.len() == 7);
    }

    #[test]
    fn same_seed_same_choice_other_seeds_vary() {
        let desks: Vec<Guid> = (1..=4).map(g).collect();
        let images = imgs(10);
        let a = choose(&desks, &desks, &HashMap::new(), &images, &mut Rng::new(42));
        let b = choose(&desks, &desks, &HashMap::new(), &images, &mut Rng::new(42));
        assert_eq!(a, b);
        let distinct: std::collections::HashSet<Vec<(Guid, String)>> =
            (0..20).map(|s| choose(&desks, &desks, &HashMap::new(), &images, &mut Rng::new(s))).collect();
        assert!(distinct.len() > 10, "random enough: {}", distinct.len());
        // Uniform over many draws: each of 4 pictures is picked for a single desktop about equally often.
        let four = imgs(4);
        let mut counts = [0usize; 4];
        let mut rng = Rng::new(7);
        for _ in 0..4000 {
            let o = choose(&[g(1)], &[g(1)], &HashMap::new(), &four, &mut rng);
            counts[four.iter().position(|i| *i == o[0].1).unwrap()] += 1;
        }
        assert!(counts.iter().all(|&c| (850..=1150).contains(&c)), "{counts:?}");
    }

    #[test]
    fn a_new_desktop_avoids_its_neighbour_and_the_others_keep_theirs() {
        let images = imgs(3);
        let mut have = HashMap::new();
        have.insert(g(1), images[0].clone());
        have.insert(g(2), images[1].clone());
        let desks = [g(1), g(2), g(3)];
        for seed in 0..30 {
            let out = choose(&desks, &[g(3)], &have, &images, &mut Rng::new(seed));
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].0, g(3));
            assert_eq!(out[0].1, images[2], "least used and not the neighbour's");
        }
        // Shuffling a desktop changes its picture when another one is allowed.
        for seed in 0..30 {
            let out = choose(&desks, &[g(1)], &have, &images, &mut Rng::new(seed));
            assert_ne!(out[0].1, images[0]);
            assert_ne!(out[0].1, images[1], "neighbour");
        }
        // Pictures are remembered by GUID: the order of the list (renumbering) does not matter.
        let moved = [g(2), g(1), g(3)];
        let out = choose(&moved, &[g(3)], &have, &images, &mut Rng::new(3));
        assert_ne!(out[0].1, images[0], "the neighbour of g3 is g1 now");
    }

    #[test]
    fn without_per_desktop_support_one_shared_random_picture() {
        let list = [g(1), g(2), g(3)];
        let images = imgs(5);
        let caps = Caps { per_desktop: false, all_desktops: false };
        let mut have = Assigned::default();
        for d in list {
            have.set(d, &images[2]);
        }
        for seed in 0..20 {
            let out = plan_random(caps, &list, &list, &have, &images, &mut Rng::new(seed));
            assert_eq!(out.len(), 3);
            assert!(out.iter().all(|o| o.1 == out[0].1), "one picture for all");
            assert_ne!(out[0].1, images[2], "a new one");
        }
        assert!(plan_random(caps, &list, &list, &have, &[], &mut Rng::new(1)).is_empty());
        let per = plan_random(Caps { per_desktop: true, all_desktops: true }, &list, &list, &Assigned::default(), &images, &mut Rng::new(1));
        let mut p: Vec<&String> = per.iter().map(|o| &o.1).collect();
        p.dedup();
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn assignments_roundtrip_and_are_bounded() {
        let mut a = Assigned::default();
        a.set(g(1), r#"C:\a "x".jpg"#);
        a.set(g(2), r"D:\b.png");
        a.set(g(1), r"C:\c.jpg");
        let back = Assigned::from_json(&a.to_json());
        assert_eq!(back.map, a.map);
        assert_eq!(back.map[&g(1)], r"C:\c.jpg");
        a.retain(&[g(2)]);
        assert_eq!(a.map.len(), 1);
        assert_eq!(Assigned::from_json("nonsense"), Assigned::default());
        assert_eq!(Assigned::from_json(r#"{"assigned":[{"desktop":"x","wallpaper":"a"},{"desktop":5},7]}"#), Assigned::default());
        let mut big = Assigned::default();
        for i in 0..(MAX_ASSIGNED as u32 + 10) {
            big.set(g(i), "p.jpg");
        }
        assert_eq!(big.map.len(), MAX_ASSIGNED);
        assert!(!big.map.contains_key(&g(0)) && big.map.contains_key(&g(MAX_ASSIGNED as u32 + 9)));
        assert_eq!(Assigned::default().to_json(), "{\"assigned\": []}\n");
    }
}
