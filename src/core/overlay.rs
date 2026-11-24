//! The "overlay" way of carrying a window (plan 12.13), in the style of Task View: the screen
//! under the carried window is blurred, the window stays sharp in place and a row of live desktop
//! previews at the bottom shows where it goes. Pure geometry and timing; `overlay_ui.rs` draws.

/// Screen rectangle (physical pixels), `right`/`bottom` exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }
    pub fn w(&self) -> i32 {
        self.right - self.left
    }
    pub fn h(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }
}

/// Length of the highlight's move by one desktop, and of a jump round the edge.
pub const STEP_MS: f64 = 200.0;
pub const WRAP_MS: f64 = 140.0;
/// Layers appear / disappear.
pub const FADE_IN_MS: f64 = 70.0;
pub const FADE_OUT_MS: f64 = 150.0;
/// Size of a desktop preview that is not highlighted, relative to the highlighted one.
pub const TILE_SMALL: f64 = 0.93;

// ---------------------------------------------------------------- highlight motion

/// A value moving to a target with a smooth start and stop; a new target while it moves keeps
/// the speed (cubic Hermite from the current position and velocity), so fast steps merge into
/// one continuous motion instead of restarting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    from: f64,
    to: f64,
    v0: f64, // units per ms at `t0`
    t0: f64,
    dur: f64,
}

impl Motion {
    pub fn at(pos: f64) -> Motion {
        Motion { from: pos, to: pos, v0: 0.0, t0: 0.0, dur: 0.0 }
    }

    fn eval(&self, now: f64) -> (f64, f64) {
        if self.dur <= 0.0 || now >= self.t0 + self.dur {
            return (self.to, 0.0);
        }
        let t = ((now - self.t0) / self.dur).max(0.0);
        let (p0, p1, m0) = (self.from, self.to, self.v0 * self.dur);
        let (t2, t3) = (t * t, t * t * t);
        let pos = (2.0 * t3 - 3.0 * t2 + 1.0) * p0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * p1;
        let d = (6.0 * t2 - 6.0 * t) * p0 + (3.0 * t2 - 4.0 * t + 1.0) * m0 + (-6.0 * t2 + 6.0 * t) * p1;
        (pos, d / self.dur)
    }

    pub fn pos(&self, now: f64) -> f64 {
        self.eval(now).0
    }

    pub fn done(&self, now: f64) -> bool {
        self.dur <= 0.0 || now >= self.t0 + self.dur
    }

    /// Goes to `to` in `dur` ms from where it is now (keeping its speed). `jump`: starts afresh
    /// (the edge was crossed: no speed is carried over to the other end).
    pub fn retarget(&mut self, to: f64, now: f64, dur: f64, jump: bool) {
        let (pos, vel) = self.eval(now);
        *self = Motion { from: pos, to, v0: if jump { 0.0 } else { vel }, t0: now, dur };
        if dur <= 0.0 {
            self.from = to;
        }
    }
}

/// Duration of the highlight's move from desktop `from` to `to`: one step, or the short jump to
/// the other end of the row (rotation over the edge, or Esc from far away).
pub fn move_ms(from: usize, to: usize, animate: bool) -> (f64, bool) {
    if !animate {
        return (0.0, true);
    }
    if from.abs_diff(to) <= 1 {
        (STEP_MS, false)
    } else {
        (WRAP_MS, true)
    }
}

/// Opacity 0..=1 of the layers `t` ms into a fade (`fade_in` or out).
pub fn fade(t: f64, fade_in: bool) -> f64 {
    let (d, x) = if fade_in { (FADE_IN_MS, t / FADE_IN_MS) } else { (FADE_OUT_MS, 1.0 - t / FADE_OUT_MS) };
    if d <= 0.0 {
        return if fade_in { 1.0 } else { 0.0 };
    }
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// ---------------------------------------------------------------- background

/// Downscale factor of a desktop's small wallpaper picture: about 320 pixels wide, never less
/// than 4 (under acrylic nothing of it is sharp).
pub fn mini_scale(monitor_w: i32) -> i32 {
    ((monitor_w as f64 / 320.0).round() as i32).max(4)
}

/// What the blurred background shows while the target moves (the highlight is at `pos`): the
/// desktops in view and their horizontal shift in pixels (`stride` = monitor width + gap) and
/// weight. A step slides (the target comes in from the side it lies on); a jump over the edge
/// (`jump` = from, to) cross-fades in place instead of racing past every desktop.
pub fn backdrop(pos: f64, jump: Option<(usize, usize)>, count: usize, stride: i32) -> Vec<(usize, i32, f64)> {
    if count == 0 {
        return Vec::new();
    }
    let last = count - 1;
    if let Some((from, to)) = jump.filter(|(a, b)| a != b && *a <= last && *b <= last) {
        let f = ((pos - from as f64) / (to as f64 - from as f64)).clamp(0.0, 1.0);
        let mut v = Vec::new();
        if f < 1.0 {
            v.push((from, 0, 1.0 - f));
        }
        if f > 0.0 {
            v.push((to, 0, f));
        }
        return v;
    }
    let pos = pos.clamp(0.0, last as f64);
    let k = pos.floor() as usize;
    let frac = pos - k as f64;
    let mut v = vec![(k, (-frac * stride as f64).round() as i32, 1.0)];
    if frac > 0.0 && k < last {
        v.push((k + 1, ((1.0 - frac) * stride as f64).round() as i32, 1.0));
    }
    v
}

/// A thumbnail's destination cut to `bounds` (the destination window), with its source cut the
/// same way (scaled). DWM does not draw a thumbnail whose destination starts outside the window
/// (measured: left/top above the window's edge = nothing or black). None = nothing visible.
pub fn clip_thumb(dest: Rect, src: Rect, bounds: &Rect) -> Option<(Rect, Rect)> {
    if dest.w() <= 0 || dest.h() <= 0 || !dest.intersects(bounds) {
        return None;
    }
    let (sx, sy) = (src.w() as f64 / dest.w() as f64, src.h() as f64 / dest.h() as f64);
    let d = Rect::new(dest.left.max(bounds.left), dest.top.max(bounds.top), dest.right.min(bounds.right), dest.bottom.min(bounds.bottom));
    let s = Rect::new(
        src.left + ((d.left - dest.left) as f64 * sx).round() as i32,
        src.top + ((d.top - dest.top) as f64 * sy).round() as i32,
        src.right - ((dest.right - d.right) as f64 * sx).round() as i32,
        src.bottom - ((dest.bottom - d.bottom) as f64 * sy).round() as i32,
    );
    Some((d, s))
}

/// The carried window drawn into a desktop preview `tile`: its frame on `monitor` scaled into it.
pub fn map_into(tile: &Rect, monitor: &Rect, frame: &Rect) -> Rect {
    let (sx, sy) = (tile.w() as f64 / monitor.w().max(1) as f64, tile.h() as f64 / monitor.h().max(1) as f64);
    let x = |v: i32| tile.left + ((v - monitor.left) as f64 * sx).round() as i32;
    let y = |v: i32| tile.top + ((v - monitor.top) as f64 * sy).round() as i32;
    Rect::new(x(frame.left), y(frame.top), x(frame.right), y(frame.bottom))
}

/// The part of a window's surface to show: its visible frame, in the source's coordinates (origin
/// = window rectangle). Measured on 26340: `DwmQueryThumbnailSourceSize` reports the frame size
/// (1146×613 for a 1160×620 window) while `rcSource` still counts from the window rectangle; the
/// first version scaled by reported size / window size, i.e. shrank the window by ~1 % – the
/// blurred "lower resolution" look. Now the frame is taken 1:1. Another size (DPI-virtualized
/// window): proportional.
pub fn source_rect(window: &Rect, frame: &Rect, src: (i32, i32)) -> Rect {
    if src == (frame.w(), frame.h()) || src == (window.w(), window.h()) {
        // Window coordinates (origin = window rectangle), exactly the frame: no scaling.
        return frame.offset(-window.left, -window.top);
    }
    let (ww, wh) = (window.w().max(1) as f64, window.h().max(1) as f64);
    let (sx, sy) = (src.0 as f64 / ww, src.1 as f64 / wh);
    let c = |v: i32, max: i32| v.clamp(0, max);
    Rect::new(
        c(((frame.left - window.left) as f64 * sx).round() as i32, src.0),
        c(((frame.top - window.top) as f64 * sy).round() as i32, src.1),
        c(((frame.right - window.left) as f64 * sx).round() as i32, src.0),
        c(((frame.bottom - window.top) as f64 * sy).round() as i32, src.1),
    )
}

// ---------------------------------------------------------------- the row of desktops

/// Where the row of desktop previews goes on the monitor of the carried window.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    /// The panel (rounded background) in screen coordinates.
    pub panel: Rect,
    /// Each desktop's preview at full (highlighted) size, screen coordinates.
    pub tiles: Vec<Rect>,
    /// The "+" place behind the last desktop ("create a new desktop" is on).
    pub plus: Option<Rect>,
    /// Height of a desktop's name under its preview.
    pub label_h: i32,
}

/// Lays the row out: centred at the bottom of the work area, above the taskbar; previews keep
/// the monitor's aspect ratio and shrink so that all of them fit into 90 % of the width.
pub fn strip(work: &Rect, monitor: &Rect, count: usize, plus: bool, dpi_scale: f64) -> Strip {
    let px = |v: f64| (v * dpi_scale).round() as i32;
    let (pad, gap, label_h, margin) = (px(14.0), px(14.0), px(26.0), px(20.0));
    let slots = count.max(1) + plus as usize;
    let aspect = monitor.h().max(1) as f64 / monitor.w().max(1) as f64;
    let max_w = (work.w() as f64 * 0.9 - 2.0 * pad as f64 - (slots - 1) as f64 * gap as f64) / slots as f64;
    let tile_w = max_w.min(work.w() as f64 * 0.12).min(px(240.0) as f64).max(px(40.0) as f64).floor() as i32;
    let tile_h = (tile_w as f64 * aspect).round() as i32;
    let plus_w = if plus { (tile_w / 2).max(px(28.0)) } else { 0 };
    let inner_w = count.max(1) as i32 * tile_w + (count.max(1) as i32 - 1) * gap + if plus { gap + plus_w } else { 0 };
    let panel_w = inner_w + 2 * pad;
    let panel_h = pad + tile_h + label_h + pad / 2;
    let left = work.left + (work.w() - panel_w) / 2;
    let bottom = work.bottom - margin;
    let panel = Rect::new(left, bottom - panel_h, left + panel_w, bottom);
    let top = panel.top + pad;
    let tiles = (0..count)
        .map(|i| {
            let x = panel.left + pad + i as i32 * (tile_w + gap);
            Rect::new(x, top, x + tile_w, top + tile_h)
        })
        .collect::<Vec<_>>();
    let plus = plus.then(|| {
        let x = panel.left + pad + count as i32 * (tile_w + gap);
        Rect::new(x, top, x + plus_w, top + tile_h)
    });
    Strip { panel, tiles, plus, label_h }
}

/// A preview drawn at `zoom` (1 = full size), shrunk around its centre.
pub fn zoomed(r: &Rect, zoom: f64) -> Rect {
    let dw = ((r.w() as f64 * (1.0 - zoom)) / 2.0).round() as i32;
    let dh = ((r.h() as f64 * (1.0 - zoom)) / 2.0).round() as i32;
    Rect::new(r.left + dw, r.top + dh, r.right - dw, r.bottom - dh)
}

/// Zoom of preview `i` while the highlight is at `pos` (between two previews while it moves).
pub fn tile_zoom(i: usize, pos: f64) -> f64 {
    let near = (1.0 - (i as f64 - pos).abs()).clamp(0.0, 1.0);
    TILE_SMALL + (1.0 - TILE_SMALL) * near
}

/// The highlight frame at `pos`: between the previews it moves across.
pub fn highlight(tiles: &[Rect], pos: f64) -> Option<Rect> {
    let last = tiles.len().checked_sub(1)?;
    let pos = pos.clamp(0.0, last as f64);
    let (i, f) = (pos.floor() as usize, pos.fract());
    let a = tiles[i];
    let b = tiles[(i + 1).min(last)];
    let l = |x: i32, y: i32| (x as f64 + (y - x) as f64 * f).round() as i32;
    Some(Rect::new(l(a.left, b.left), l(a.top, b.top), l(a.right, b.right), l(a.bottom, b.bottom)))
}

/// The name under a preview: the user's own name of the desktop, else "<word> N".
pub fn desktop_label(index: usize, name: Option<&str>, word: &str) -> String {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => format!("{word} {}", index + 1),
    }
}

// ---------------------------------------------------------------- wallpaper

/// How the wallpaper is placed (`WallpaperStyle` / `TileWallpaper` in `HKCU\Control Panel\Desktop`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Center,
    Tile,
    Stretch,
    Fit,
    Fill,
    Span,
}

impl Placement {
    /// From the registry strings; unknown values = Fill (the Windows default).
    pub fn from_registry(style: &str, tile: &str) -> Placement {
        if tile.trim() == "1" {
            return Placement::Tile;
        }
        match style.trim() {
            "0" => Placement::Center,
            "2" => Placement::Stretch,
            "6" => Placement::Fit,
            "22" => Placement::Span,
            _ => Placement::Fill,
        }
    }
}

/// Where an image `iw`×`ih` goes in an area `w`×`h` (a monitor's small copy, `scale` = how many
/// screen pixels one of its pixels is). Several rectangles for Tile. Span is treated as Fill of
/// each monitor (one image per monitor).
pub fn place(p: Placement, iw: i32, ih: i32, w: i32, h: i32, scale: i32) -> Vec<Rect> {
    if iw <= 0 || ih <= 0 || w <= 0 || h <= 0 {
        return Vec::new();
    }
    let (fw, fh) = (iw as f64, ih as f64);
    let centred = |dw: f64, dh: f64| {
        let (dw, dh) = (dw.round().max(1.0) as i32, dh.round().max(1.0) as i32);
        let x = (w - dw) / 2;
        let y = (h - dh) / 2;
        Rect::new(x, y, x + dw, y + dh)
    };
    match p {
        Placement::Stretch => vec![Rect::new(0, 0, w, h)],
        Placement::Fill | Placement::Span => {
            let k = (w as f64 / fw).max(h as f64 / fh);
            vec![centred(fw * k, fh * k)]
        }
        Placement::Fit => {
            let k = (w as f64 / fw).min(h as f64 / fh);
            vec![centred(fw * k, fh * k)]
        }
        Placement::Center => vec![centred(fw / scale as f64, fh / scale as f64)],
        Placement::Tile => {
            let tw = ((fw / scale as f64).round() as i32).max(1);
            let th = ((fh / scale as f64).round() as i32).max(1);
            let mut v = Vec::new();
            let mut y = 0;
            while y < h && v.len() < 4096 {
                let mut x = 0;
                while x < w && v.len() < 4096 {
                    v.push(Rect::new(x, y, x + tw, y + th));
                    x += tw;
                }
                y += th;
            }
            v
        }
    }
}

/// Resizes 32-bit pixels (`sw`×`sh`, rows top-down) to `dw`×`dh`: each target pixel is the
/// average of the source pixels it covers (at least the nearest one).
pub fn resize(src: &[u32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u32> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh {
        return vec![0; dw * dh];
    }
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let y0 = y * sh / dh;
        let y1 = ((y + 1) * sh / dh).max(y0 + 1).min(sh);
        for x in 0..dw {
            let x0 = x * sw / dw;
            let x1 = ((x + 1) * sw / dw).max(x0 + 1).min(sw);
            let (mut b, mut g, mut r, mut n) = (0u32, 0u32, 0u32, 0u32);
            for yy in y0..y1 {
                for p in &src[yy * sw + x0..yy * sw + x1] {
                    b += p & 0xFF;
                    g += p >> 8 & 0xFF;
                    r += p >> 16 & 0xFF;
                    n += 1;
                }
            }
            out.push((b / n) | (g / n) << 8 | (r / n) << 16);
        }
    }
    out
}

/// `"R G B"` (`HKCU\Control Panel\Colors\Background`) as a COLORREF (0x00BBGGRR).
pub fn parse_rgb(s: &str) -> Option<u32> {
    let v: Vec<u32> = s.split_whitespace().filter_map(|x| x.parse::<u32>().ok()).collect();
    (v.len() == 3 && v.iter().all(|c| *c <= 255)).then(|| v[0] | v[1] << 8 | v[2] << 16)
}

/// The hint "this window cannot be moved": centred at the bottom of the work area, above the
/// taskbar, `margin` (already scaled) above its bottom edge.
pub fn hint_pos(work: &Rect, w: i32, h: i32, margin: i32) -> (i32, i32) {
    (work.left + (work.w() - w) / 2, (work.bottom - margin - h).max(work.top))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect::new(0, 0, 2560, 1440);
    const WORK: Rect = Rect::new(0, 0, 2560, 1392);

    #[test]
    fn motion_ends_at_target_and_keeps_speed_on_retarget() {
        let mut m = Motion::at(0.0);
        m.retarget(1.0, 0.0, 200.0, false);
        assert_eq!(m.pos(0.0), 0.0);
        assert!((m.pos(100.0) - 0.5).abs() < 1e-9, "symmetric ease");
        assert!(m.pos(50.0) < 0.25, "slow start");
        assert_eq!(m.pos(200.0), 1.0);
        assert!(m.done(200.0) && !m.done(199.0));
        // A second step half way: no jump in position, speed carried on, ends at the new target.
        let before = m.pos(100.0);
        m.retarget(2.0, 100.0, 200.0, false);
        assert!((m.pos(100.0) - before).abs() < 1e-9);
        let (a, b) = (m.pos(100.0), m.pos(101.0));
        assert!(b - a > 0.005, "still moving at once");
        assert_eq!(m.pos(300.0), 2.0);
        // Monotonic, never overshoots.
        let mut last = 0.0;
        for t in 100..=300 {
            let p = m.pos(t as f64);
            assert!(p >= last - 1e-9 && p <= 2.0 + 1e-9);
            last = p;
        }
    }

    #[test]
    fn jump_over_the_edge_is_short_and_starts_afresh() {
        assert_eq!(move_ms(2, 3, true), (STEP_MS, false));
        assert_eq!(move_ms(3, 2, true), (STEP_MS, false));
        assert_eq!(move_ms(3, 0, true), (WRAP_MS, true), "rotation: short jump to the other end");
        assert_eq!(move_ms(0, 3, false), (0.0, true), "animations off: at once");
        let mut m = Motion::at(3.0);
        m.retarget(0.0, 0.0, WRAP_MS, true);
        assert_eq!(m.pos(WRAP_MS), 0.0);
        let mut m = Motion::at(1.0);
        m.retarget(0.0, 5.0, 0.0, true);
        assert_eq!((m.pos(5.0), m.done(5.0)), (0.0, true));
    }

    #[test]
    fn fades() {
        assert_eq!(fade(0.0, true), 0.0);
        assert_eq!(fade(FADE_IN_MS, true), 1.0);
        assert_eq!(fade(0.0, false), 1.0);
        assert_eq!(fade(FADE_OUT_MS + 5.0, false), 0.0);
        assert!((fade(FADE_OUT_MS / 2.0, false) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn source_is_the_visible_frame() {
        let win = Rect::new(93, 93, 1107, 807); // 7 px invisible borders left/right/bottom
        let frame = Rect::new(100, 93, 1100, 800);
        // A source as large as the window rectangle: the invisible borders are cut off.
        assert_eq!(source_rect(&win, &frame, (1014, 714)), Rect::new(7, 0, 1007, 707));
        // What DWM reports on Windows 11 (measured): the frame size; still window coordinates, 1:1.
        assert_eq!(source_rect(&win, &frame, (1000, 707)), Rect::new(7, 0, 1007, 707));
        let (w, f) = (Rect::new(700, 300, 1860, 920), Rect::new(707, 300, 1853, 913));
        let s = source_rect(&w, &f, (1146, 613));
        assert_eq!((s, s.w(), s.h()), (Rect::new(7, 0, 1153, 613), f.w(), f.h()), "no scaling");
        // DPI virtualized (half size): proportional.
        assert_eq!(source_rect(&win, &frame, (507, 357)), Rect::new(4, 0, 504, 354));
        // A borderless window: whole.
        assert_eq!(source_rect(&frame, &frame, (1000, 707)), Rect::new(0, 0, 1000, 707));
    }

    #[test]
    fn strip_sits_centred_above_the_taskbar() {
        let s = strip(&WORK, &MON, 4, false, 1.0);
        assert_eq!(s.tiles.len(), 4);
        assert!(s.plus.is_none());
        assert!(s.panel.bottom <= WORK.bottom - 20 && s.panel.bottom > WORK.bottom - 40);
        assert_eq!(s.panel.left - WORK.left, WORK.right - s.panel.right, "centred");
        let t = s.tiles[0];
        assert_eq!(t.w(), 240);
        assert_eq!(t.h(), 135, "monitor aspect");
        assert!(s.tiles.windows(2).all(|p| p[0].right < p[1].left));
        assert!(s.tiles.iter().all(|t| t.top >= s.panel.top && t.bottom + s.label_h <= s.panel.bottom));
        // "+" behind the last one when new desktops are created.
        let p = strip(&WORK, &MON, 4, true, 1.0);
        assert!(p.plus.unwrap().left > p.tiles[3].right);
        // Many desktops shrink to fit 90 % of the width.
        let m = strip(&WORK, &MON, 30, false, 1.0);
        assert!(m.panel.w() <= (WORK.w() as f64 * 0.9) as i32 + 1, "{:?}", m.panel);
        // 150 % scaling: bigger margins, same rules.
        let h = strip(&WORK, &MON, 2, false, 1.5);
        assert!(h.panel.bottom <= WORK.bottom - 30);
    }

    #[test]
    fn strip_on_a_monitor_with_negative_coordinates_and_taskbar_on_top() {
        let mon = Rect::new(-1920, -1080, 0, 0);
        let work = Rect::new(-1920, -1032, 0, 0); // taskbar at the top
        let s = strip(&work, &mon, 3, false, 1.0);
        assert!(s.panel.left > mon.left && s.panel.right < mon.right);
        assert!(s.panel.bottom <= work.bottom && s.panel.top > work.top);
    }

    #[test]
    fn highlight_moves_between_previews_and_they_zoom() {
        let s = strip(&WORK, &MON, 3, false, 1.0);
        assert_eq!(highlight(&s.tiles, 1.0), Some(s.tiles[1]));
        let mid = highlight(&s.tiles, 0.5).unwrap();
        assert_eq!(mid.left, (s.tiles[0].left + s.tiles[1].left + 1) / 2);
        assert_eq!(highlight(&s.tiles, 9.0), Some(s.tiles[2]), "clamped");
        assert_eq!(highlight(&[], 0.0), None);
        assert_eq!(tile_zoom(1, 1.0), 1.0);
        assert_eq!(tile_zoom(0, 1.0), TILE_SMALL);
        assert!((tile_zoom(0, 0.5) - (TILE_SMALL + 1.0) / 2.0).abs() < 1e-9);
        let z = zoomed(&Rect::new(0, 0, 200, 100), 0.9);
        assert_eq!(z, Rect::new(10, 5, 190, 95));
    }

    #[test]
    fn labels() {
        assert_eq!(desktop_label(0, None, "Plocha"), "Plocha 1");
        assert_eq!(desktop_label(2, Some("  "), "Desktop"), "Desktop 3");
        assert_eq!(desktop_label(1, Some("Práce"), "Plocha"), "Práce");
    }

    #[test]
    fn wallpaper_placement_from_registry() {
        use Placement::*;
        assert_eq!(Placement::from_registry("10", "0"), Fill);
        assert_eq!(Placement::from_registry("6", "0"), Fit);
        assert_eq!(Placement::from_registry("2", "0"), Stretch);
        assert_eq!(Placement::from_registry("0", "0"), Center);
        assert_eq!(Placement::from_registry("0", "1"), Tile);
        assert_eq!(Placement::from_registry("22", "0"), Span);
        assert_eq!(Placement::from_registry("", ""), Fill);
    }

    #[test]
    fn wallpaper_placement_geometry() {
        use Placement::*;
        // 4:3 image on a 16:9 small copy (320×180, scale 8).
        assert_eq!(place(Fill, 1600, 1200, 320, 180, 8), vec![Rect::new(0, -30, 320, 210)]);
        assert_eq!(place(Fit, 1600, 1200, 320, 180, 8), vec![Rect::new(40, 0, 280, 180)]);
        assert_eq!(place(Stretch, 1600, 1200, 320, 180, 8), vec![Rect::new(0, 0, 320, 180)]);
        assert_eq!(place(Center, 800, 400, 320, 180, 8), vec![Rect::new(110, 65, 210, 115)]);
        let t = place(Tile, 800, 800, 320, 180, 8);
        assert_eq!(t[0], Rect::new(0, 0, 100, 100));
        assert_eq!(t.len(), 4 * 2);
        assert!(place(Fill, 0, 10, 320, 180, 8).is_empty());
        assert_eq!(parse_rgb("0 99 177"), Some(0x00B1_6300));
        assert_eq!(parse_rgb("1 2"), None);
        assert_eq!(parse_rgb("300 0 0"), None);
    }

    #[test]
    fn hint_at_the_bottom_above_the_taskbar() {
        assert_eq!(hint_pos(&WORK, 200, 66, 32), (1180, 1392 - 32 - 66));
        // Taskbar on the left: still the bottom edge of the work area.
        let w = Rect::new(48, 0, 2560, 1440);
        assert_eq!(hint_pos(&w, 200, 66, 32), (48 + (2512 - 200) / 2, 1440 - 32 - 66));
        // Never above the work area.
        assert_eq!(hint_pos(&Rect::new(0, 0, 300, 50), 200, 66, 32).1, 0);
    }

    #[test]
    fn background_slides_with_the_target_and_fades_over_the_edge() {
        let stride = 2600;
        // At rest: just the desktop, unshifted.
        assert_eq!(backdrop(2.0, None, 4, stride), vec![(2, 0, 1.0)]);
        // Step right half way: the current one goes out left, the next comes in from the right.
        assert_eq!(backdrop(2.5, None, 4, stride), vec![(2, -1300, 1.0), (3, 1300, 1.0)]);
        // Step left (pos falls from 2 to 1): the same pair, the other way round in time.
        assert_eq!(backdrop(1.25, None, 4, stride), vec![(1, -650, 1.0), (2, 1950, 1.0)]);
        // Never outside the desktops.
        assert_eq!(backdrop(7.0, None, 4, stride), vec![(3, 0, 1.0)]);
        assert!(backdrop(0.0, None, 0, stride).is_empty());
        // Jump 3 → 0 over the edge: no sliding past desktops 2 and 1, a cross-fade in place.
        assert_eq!(backdrop(3.0, Some((3, 0)), 4, stride), vec![(3, 0, 1.0)]);
        assert_eq!(backdrop(1.5, Some((3, 0)), 4, stride), vec![(3, 0, 0.5), (0, 0, 0.5)]);
        assert_eq!(backdrop(0.0, Some((3, 0)), 4, stride), vec![(0, 0, 1.0)]);
    }

    #[test]
    fn small_wallpaper_scale() {
        assert_eq!(mini_scale(2560), 8);
        assert_eq!(mini_scale(1920), 6);
        assert_eq!(mini_scale(800), 4);
    }

    #[test]
    fn thumbnails_are_cut_to_their_window() {
        let b = Rect::new(0, 0, 2560, 1354);
        // The monitor 38 px above the work area: the top 38 rows are cut, 1:1.
        let (d, s) = clip_thumb(Rect::new(0, -38, 2560, 1402), Rect::new(0, 0, 2560, 1440), &b).unwrap();
        assert_eq!((d, s), (Rect::new(0, 0, 2560, 1354), Rect::new(0, 38, 2560, 1392)));
        // Sliding half out to the left.
        let (d, s) = clip_thumb(Rect::new(-1300, 0, 1260, 1440), Rect::new(0, 0, 2560, 1440), &b).unwrap();
        assert_eq!((d, s), (Rect::new(0, 0, 1260, 1354), Rect::new(1300, 0, 2560, 1354)));
        // A maximized window's invisible borders outside the panel; scaled source (half size).
        let (d, s) = clip_thumb(Rect::new(-8, -8, 2568, 1448), Rect::new(0, 0, 1288, 728), &Rect::new(0, 0, 2560, 1440)).unwrap();
        assert_eq!((d, s), (Rect::new(0, 0, 2560, 1440), Rect::new(4, 4, 1284, 724)));
        assert!(clip_thumb(Rect::new(3000, 0, 4000, 100), Rect::new(0, 0, 10, 10), &b).is_none());
    }

    #[test]
    fn carried_window_in_a_preview() {
        let tile = Rect::new(1000, 1200, 1240, 1335);
        assert_eq!(map_into(&tile, &MON, &Rect::new(1280, 720, 2560, 1440)), Rect::new(1120, 1268, 1240, 1335));
        assert_eq!(map_into(&tile, &MON, &MON), tile);
    }

    #[test]
    fn resize_averages_and_keeps_size() {
        // 4×2 → 2×1: averages of 2×2 blocks.
        let src = [0x000000, 0x0000FF, 0xFF0000, 0xFF0000, 0x000000, 0x0000FF, 0xFF0000, 0xFF0000];
        assert_eq!(resize(&src, 4, 2, 2, 1), vec![0x00007F, 0xFF0000]);
        // Upscale: nearest pixel.
        assert_eq!(resize(&[1, 2], 2, 1, 4, 1), vec![1, 1, 2, 2]);
        assert_eq!(resize(&[], 0, 0, 3, 2).len(), 6);
    }

}
