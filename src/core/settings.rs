//! Settings (plan 6.6). Loading from a file is a later phase; for now plain defaults.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub switch_animation: bool,
    pub switch_back_when_others_remain: bool,
    pub fullscreen_settle_ms: u32,
    pub trigger_on_maximize: bool,
    pub trigger_on_fullscreen: bool,
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
        }
    }
}
