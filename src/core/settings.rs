//! Settings (plan 6.6). Loading from a file is a later phase; for now plain defaults.

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
        }
    }
}
