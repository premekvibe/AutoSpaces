//! Window layouts of a desktop in the style of Windows Snap layouts (plan 12.18): the templates,
//! their zones in the work area of the primary monitor, the compensation of invisible window
//! borders, and recognising a template from the windows on a desktop ("Use current layout").
//! Pure geometry; `windowing/system.rs` does the Win32 calls.

use super::overlay::Rect;

/// A window layout of one desktop. `Full` holds one window that keeps the window state of its slot
/// (maximized / normal / minimized, plan 12.16); every other template places normal windows into
/// its zones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Template {
    #[default]
    Full,
    /// Left and right half.
    Halves,
    /// 2/3 left, 1/3 right.
    TwoThirdsLeft,
    /// 1/3 left, 2/3 right.
    TwoThirdsRight,
    /// Three equal columns.
    Thirds,
    /// 1/4, 1/2, 1/4.
    WideCenter,
    /// Left half, right half split into top and bottom.
    BigLeft,
    /// Four quarters: top left, top right, bottom left, bottom right.
    Quarters,
}

/// Zones in twelfths of the work area: (left, top, right, bottom). The order is the reading order
/// (left to right, then top to bottom): zone 1, 2, ... in the dialog.
const Z_FULL: &[[u8; 4]] = &[[0, 0, 12, 12]];
const Z_HALVES: &[[u8; 4]] = &[[0, 0, 6, 12], [6, 0, 12, 12]];
const Z_TWO_THIRDS_LEFT: &[[u8; 4]] = &[[0, 0, 8, 12], [8, 0, 12, 12]];
const Z_TWO_THIRDS_RIGHT: &[[u8; 4]] = &[[0, 0, 4, 12], [4, 0, 12, 12]];
const Z_THIRDS: &[[u8; 4]] = &[[0, 0, 4, 12], [4, 0, 8, 12], [8, 0, 12, 12]];
const Z_WIDE_CENTER: &[[u8; 4]] = &[[0, 0, 3, 12], [3, 0, 9, 12], [9, 0, 12, 12]];
const Z_BIG_LEFT: &[[u8; 4]] = &[[0, 0, 6, 12], [6, 0, 12, 6], [6, 6, 12, 12]];
const Z_QUARTERS: &[[u8; 4]] = &[[0, 0, 6, 6], [6, 0, 12, 6], [0, 6, 6, 12], [6, 6, 12, 12]];

impl Template {
    /// In the order of the combo box of the layout dialog.
    pub const ALL: [Template; 8] = [
        Template::Full,
        Template::Halves,
        Template::TwoThirdsLeft,
        Template::TwoThirdsRight,
        Template::Thirds,
        Template::WideCenter,
        Template::BigLeft,
        Template::Quarters,
    ];

    /// Value of `"layout"` in `settings.json`.
    pub fn code(self) -> &'static str {
        match self {
            Template::Full => "full",
            Template::Halves => "halves",
            Template::TwoThirdsLeft => "twoThirdsLeft",
            Template::TwoThirdsRight => "twoThirdsRight",
            Template::Thirds => "thirds",
            Template::WideCenter => "wideCenter",
            Template::BigLeft => "bigLeft",
            Template::Quarters => "quarters",
        }
    }

    /// Unknown codes: None (the caller decides; `settings.json` reads them as `Full`).
    pub fn from_code(s: &str) -> Option<Template> {
        let s = s.trim();
        Self::ALL.iter().copied().find(|t| t.code().eq_ignore_ascii_case(s))
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Template {
        Self::ALL.get(i).copied().unwrap_or_default()
    }

    fn fractions(self) -> &'static [[u8; 4]] {
        match self {
            Template::Full => Z_FULL,
            Template::Halves => Z_HALVES,
            Template::TwoThirdsLeft => Z_TWO_THIRDS_LEFT,
            Template::TwoThirdsRight => Z_TWO_THIRDS_RIGHT,
            Template::Thirds => Z_THIRDS,
            Template::WideCenter => Z_WIDE_CENTER,
            Template::BigLeft => Z_BIG_LEFT,
            Template::Quarters => Z_QUARTERS,
        }
    }

    pub fn zone_count(self) -> usize {
        self.fractions().len()
    }
}

/// Position of the twelfth `f` between `a` and `b` (rounded): neighbouring zones share the edge,
/// so they never leave a gap or overlap.
fn edge(a: i32, b: i32, f: u8) -> i32 {
    let span = (b - a) as i64;
    a + ((span * f as i64 + 6).div_euclid(12)) as i32
}

/// The zones of `t` in `work` (screen pixels of the work area, e.g. `rcWork` of the primary
/// monitor; physical pixels in a per-monitor DPI aware process). No gap between zones: the
/// visible frames touch, as with Snap in Windows 11.
pub fn zones(t: Template, work: Rect) -> Vec<Rect> {
    t.fractions()
        .iter()
        .map(|f| {
            Rect::new(
                edge(work.left, work.right, f[0]),
                edge(work.top, work.bottom, f[1]),
                edge(work.left, work.right, f[2]),
                edge(work.top, work.bottom, f[3]),
            )
        })
        .collect()
}

/// Zone `i` of `t` in `work`.
pub fn zone(t: Template, work: Rect, i: usize) -> Option<Rect> {
    zones(t, work).get(i).copied()
}

/// Invisible borders of a window: how far the window rectangle (`GetWindowRect`) reaches beyond its
/// visible frame (`DWMWA_EXTENDED_FRAME_BOUNDS`) on each side. Windows 10/11 windows have an
/// invisible resize border left, right and at the bottom (7 px at 100 %, more at higher DPI).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margins {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Larger borders than this are not invisible borders (a broken frame rectangle): ignored.
const MAX_MARGIN: i32 = 64;

impl Margins {
    /// Measured from both rectangles; a side where the frame lies outside the window (no DWM frame,
    /// odd values) counts as 0.
    pub fn measure(window: Rect, frame: Rect) -> Margins {
        let side = |v: i32| if (0..=MAX_MARGIN).contains(&v) { v } else { 0 };
        Margins {
            left: side(frame.left - window.left),
            top: side(frame.top - window.top),
            right: side(window.right - frame.right),
            bottom: side(window.bottom - frame.bottom),
        }
    }

    /// Guess for a window whose borders cannot be measured (minimized or maximized): the usual
    /// invisible border of Windows 10/11 at `dpi` (7 px at 96 DPI), none at the top. A wrong guess
    /// is corrected by the next placement pass, which measures.
    pub fn guess(dpi: u32) -> Margins {
        let b = (7 * dpi.max(96) as i32 + 48) / 96;
        Margins { left: b, top: 0, right: b, bottom: b }
    }
}

/// The window rectangle to set so that the visible frame of a window with invisible borders `m`
/// covers exactly `zone`.
pub fn window_rect_for(zone: Rect, m: Margins) -> Rect {
    Rect::new(zone.left - m.left, zone.top - m.top, zone.right + m.right, zone.bottom + m.bottom)
}

/// Screen coordinates to the workspace coordinates of `WINDOWPLACEMENT::rcNormalPosition` (they are
/// relative to the work area of the window's monitor: a taskbar at the left or top shifts them).
pub fn to_workspace(r: Rect, work: Rect, monitor: Rect) -> Rect {
    r.offset(-(work.left - monitor.left), -(work.top - monitor.top))
}

/// The visible frame of a placed window lies in its zone (a pixel or two off is still a fit:
/// rounding of DPI scaling).
pub fn fits(frame: Rect, zone: Rect) -> bool {
    const TOL: i32 = 2;
    (frame.left - zone.left).abs() <= TOL
        && (frame.top - zone.top).abs() <= TOL
        && (frame.right - zone.right).abs() <= TOL
        && (frame.bottom - zone.bottom).abs() <= TOL
}

/// "Roughly in the zone" for recognising a layout: every edge within 1/20 of the shorter side of the
/// work area (at least 16 px; 52 px on a 1920 x 1040 work area). Also used to tell whether a
/// window of a layout went back to its zone after being restored (plan 12.19).
pub fn near(frame: Rect, zone: Rect, work: Rect) -> bool {
    let tol = (work.w().min(work.h()) / 20).max(16);
    (frame.left - zone.left).abs() <= tol
        && (frame.top - zone.top).abs() <= tol
        && (frame.right - zone.right).abs() <= tol
        && (frame.bottom - zone.bottom).abs() <= tol
}

/// The template the windows `frames` (visible frames of normal windows on one desktop, topmost
/// first) are arranged in, with the window of each zone (index into `frames`, None = empty zone).
/// Each zone takes the topmost window lying roughly in it. The template with the most filled
/// zones wins (at least two: one window alone is no layout), on a tie the one with fewer zones
/// (fewer empty ones). `Full` is never returned; None = no template fits.
pub fn recognize(frames: &[Rect], work: Rect) -> Option<(Template, Vec<Option<usize>>)> {
    if work.w() <= 0 || work.h() <= 0 {
        return None;
    }
    let mut best: Option<(usize, Template, Vec<Option<usize>>)> = None;
    for t in Template::ALL.into_iter().filter(|t| *t != Template::Full) {
        let mut used = vec![false; frames.len()];
        let mut slots = Vec::new();
        for z in zones(t, work) {
            let hit = (0..frames.len()).find(|&i| !used[i] && near(frames[i], z, work));
            if let Some(i) = hit {
                used[i] = true;
            }
            slots.push(hit);
        }
        let filled = slots.iter().filter(|s| s.is_some()).count();
        let better = match &best {
            None => true,
            Some((f, bt, _)) => filled > *f || (filled == *f && t.zone_count() < bt.zone_count()),
        };
        if filled >= 2 && better {
            best = Some((filled, t, slots));
        }
    }
    best.map(|(_, t, s)| (t, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: Rect = Rect::new(0, 0, 1920, 1040); // 1920 x 1080, taskbar 40 px at the bottom

    fn r(l: i32, t: i32, rr: i32, b: i32) -> Rect {
        Rect::new(l, t, rr, b)
    }

    /// No gaps, no overlaps, the union is the work area (checked by area and pairwise).
    fn tiles_exactly(zs: &[Rect], work: Rect) {
        let area: i64 = zs.iter().map(|z| z.w() as i64 * z.h() as i64).sum();
        assert_eq!(area, work.w() as i64 * work.h() as i64, "{zs:?}");
        for (i, a) in zs.iter().enumerate() {
            assert!(a.left >= work.left && a.top >= work.top && a.right <= work.right && a.bottom <= work.bottom);
            assert!(a.w() > 0 && a.h() > 0);
            for b in &zs[i + 1..] {
                assert!(!a.intersects(b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn zones_of_every_template_on_full_hd() {
        let z = |t| zones(t, FHD);
        assert_eq!(z(Template::Full), vec![FHD]);
        assert_eq!(z(Template::Halves), vec![r(0, 0, 960, 1040), r(960, 0, 1920, 1040)]);
        assert_eq!(z(Template::TwoThirdsLeft), vec![r(0, 0, 1280, 1040), r(1280, 0, 1920, 1040)]);
        assert_eq!(z(Template::TwoThirdsRight), vec![r(0, 0, 640, 1040), r(640, 0, 1920, 1040)]);
        assert_eq!(z(Template::Thirds), vec![r(0, 0, 640, 1040), r(640, 0, 1280, 1040), r(1280, 0, 1920, 1040)]);
        assert_eq!(z(Template::WideCenter), vec![r(0, 0, 480, 1040), r(480, 0, 1440, 1040), r(1440, 0, 1920, 1040)]);
        assert_eq!(z(Template::BigLeft), vec![r(0, 0, 960, 1040), r(960, 0, 1920, 520), r(960, 520, 1920, 1040)]);
        assert_eq!(
            z(Template::Quarters),
            vec![r(0, 0, 960, 520), r(960, 0, 1920, 520), r(0, 520, 960, 1040), r(960, 520, 1920, 1040)]
        );
        for t in Template::ALL {
            assert_eq!(z(t).len(), t.zone_count());
            tiles_exactly(&z(t), FHD);
        }
    }

    #[test]
    fn zones_follow_any_work_area_odd_sizes_and_offsets() {
        // Taskbar at the left (work area not at the origin), odd sizes, a 150 % and a 175 % screen
        // (physical pixels: the process is per-monitor DPI aware), a portrait monitor.
        let works = [
            r(62, 0, 1920, 1080),
            r(0, 0, 1366, 728),
            r(0, 0, 2560, 1380),   // 2560 x 1440 at 150 % (taskbar 60 px)
            r(0, 0, 3840, 2090),   // 3840 x 2160 at 175 % (taskbar 70 px)
            r(-1080, 0, 0, 1872),  // left of the main monitor, portrait
            r(0, 48, 1001, 1033),  // taskbar at the top, odd width
        ];
        for w in works {
            for t in Template::ALL {
                // Same area and no overlap: neighbours share their edges exactly (no rounding gap).
                tiles_exactly(&zones(t, w), w);
            }
        }
        let zs = zones(Template::Thirds, r(0, 0, 1001, 700));
        assert_eq!((zs[0].right, zs[1].right), (334, 667), "rounded, shared");
        assert_eq!(zone(Template::Halves, FHD, 1), Some(r(960, 0, 1920, 1040)));
        assert_eq!(zone(Template::Halves, FHD, 2), None);
    }

    #[test]
    fn invisible_borders_are_compensated_so_visible_frames_meet() {
        // 100 %: a typical window rect reaches 7 px beyond the frame left, right and at the bottom.
        let frame = r(100, 100, 900, 700);
        let window = r(93, 100, 907, 707);
        let m = Margins::measure(window, frame);
        assert_eq!(m, Margins { left: 7, top: 0, right: 7, bottom: 7 });
        let zs = zones(Template::Halves, FHD);
        let left = window_rect_for(zs[0], m);
        let right = window_rect_for(zs[1], m);
        assert_eq!(left, r(-7, 0, 967, 1047));
        assert_eq!(right, r(953, 0, 1927, 1047));
        // The visible frames (window minus borders) touch at x = 960 exactly.
        let vis = |w: Rect| r(w.left + m.left, w.top + m.top, w.right - m.right, w.bottom - m.bottom);
        assert_eq!(vis(left).right, vis(right).left);
        assert!(fits(vis(left), zs[0]) && fits(vis(right), zs[1]));
        // 150 %: 11 px borders.
        let m15 = Margins::measure(r(989, 0, 2571, 1391), r(1000, 0, 2560, 1380));
        assert_eq!(m15, Margins { left: 11, top: 0, right: 11, bottom: 11 });
        assert_eq!(window_rect_for(r(1280, 0, 2560, 1380), m15), r(1269, 0, 2571, 1391));
        // No DWM frame (frame = window) or nonsense: no compensation.
        assert_eq!(Margins::measure(frame, frame), Margins::default());
        assert_eq!(Margins::measure(frame, r(0, 0, 2000, 2000)), Margins::default());
        assert_eq!(Margins::measure(r(0, 0, 1000, 1000), r(500, 0, 600, 1000)).left, 0, "too big to be a border");
        // Guess for minimized / maximized windows, by DPI.
        assert_eq!(Margins::guess(96), Margins { left: 7, top: 0, right: 7, bottom: 7 });
        assert_eq!(Margins::guess(144).left, 11);
        assert_eq!(Margins::guess(192).left, 14);
        assert_eq!(Margins::guess(0).left, 7);
    }

    #[test]
    fn workspace_coordinates_and_fit_tolerance() {
        let mon = r(0, 0, 1920, 1080);
        assert_eq!(to_workspace(r(100, 50, 200, 150), r(0, 0, 1920, 1040), mon), r(100, 50, 200, 150), "taskbar at the bottom");
        assert_eq!(to_workspace(r(100, 50, 200, 150), r(62, 0, 1920, 1080), mon), r(38, 50, 138, 150), "taskbar at the left");
        assert_eq!(to_workspace(r(100, 50, 200, 150), r(0, 48, 1920, 1080), mon), r(100, 2, 200, 102), "taskbar at the top");
        let z = r(0, 0, 960, 1040);
        assert!(fits(r(1, -2, 962, 1040), z));
        assert!(!fits(r(0, 0, 963, 1040), z));
    }

    #[test]
    fn codes_and_indexes_roundtrip() {
        for t in Template::ALL {
            assert_eq!(Template::from_code(t.code()), Some(t));
            assert_eq!(Template::from_index(t.index()), t);
        }
        assert_eq!(Template::from_code(" QUARTERS "), Some(Template::Quarters));
        assert_eq!(Template::from_code("hexagons"), None);
        assert_eq!(Template::from_index(99), Template::Full);
    }

    #[test]
    fn current_windows_are_recognized_as_a_template() {
        let zs = |t| zones(t, FHD);
        // Two windows snapped left and right (frames a few px off): halves, topmost first.
        let h = zs(Template::Halves);
        let frames = [r(962, 0, 1920, 1038), r(0, 3, 958, 1040)];
        assert_eq!(recognize(&frames, FHD), Some((Template::Halves, vec![Some(1), Some(0)])));
        // Four quarters, one window elsewhere (ignored).
        let q = zs(Template::Quarters);
        let frames = [q[3], r(300, 200, 900, 700), q[0], q[2], q[1]];
        assert_eq!(recognize(&frames, FHD), Some((Template::Quarters, vec![Some(2), Some(4), Some(3), Some(0)])));
        // Three of four quarters: quarters with an empty zone (BigLeft would fill only two).
        let frames = [q[0], q[1], q[2]];
        assert_eq!(recognize(&frames, FHD), Some((Template::Quarters, vec![Some(0), Some(1), Some(2), None])));
        // 2/3 + 1/3, 1/3 + 2/3, wide middle column, big left.
        for t in [Template::TwoThirdsLeft, Template::TwoThirdsRight, Template::WideCenter, Template::BigLeft, Template::Thirds] {
            let frames = zs(t);
            let got = recognize(&frames, FHD).unwrap();
            assert_eq!(got.0, t, "{t:?}");
            assert!(got.1.iter().all(|s| s.is_some()));
        }
        // Two halves where one window is also taller than its zone by much: only one fits -> none.
        let frames = [h[0], r(960, 0, 1920, 600)];
        assert_eq!(recognize(&frames, FHD), None);
        // One window alone, free-floating windows, nothing: no template.
        assert_eq!(recognize(&[h[0]], FHD), None);
        assert_eq!(recognize(&[r(100, 100, 800, 600), r(500, 300, 1500, 900)], FHD), None);
        assert_eq!(recognize(&[], FHD), None);
        assert_eq!(recognize(&frames, r(0, 0, 0, 0)), None);
        // Duplicates in one zone: the topmost takes it, the other is left out.
        let frames = [h[0], h[0], h[1]];
        assert_eq!(recognize(&frames, FHD), Some((Template::Halves, vec![Some(0), Some(2)])));
    }
}
