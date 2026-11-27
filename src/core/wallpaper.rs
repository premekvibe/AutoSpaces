//! Wallpapers of the desktops (plan 12.17): which files count as a picture and what the system
//! offers. No Win32 here; the host (`src/wallpaper.rs`) sets the picture.

/// Picture types Windows shows as a wallpaper and every Windows decodes (WIC).
pub const IMAGE_EXTS: [&str; 4] = ["jpg", "jpeg", "png", "bmp"];

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

/// How the wallpapers can be set on this system (plan 12.17).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// One picture per desktop (`SetDesktopWallpaper` of a self-tested Windows 11 variant).
    pub per_desktop: bool,
    /// One picture on every desktop in one call (`UpdateWallpaperPathForAllDesktops`).
    pub all_desktops: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_files_by_extension() {
        for n in ["a.jpg", "B.JPEG", "c.Png", "d.bmp", "x y.jpg"] {
            assert!(is_image(n), "{n}");
        }
        for n in ["a.gif", "b.webp", "c", ".jpg", "jpg", "d.jpg.txt", "e.heic", "f.", "desktop.ini"] {
            assert!(!is_image(n), "{n}");
        }
    }
}
