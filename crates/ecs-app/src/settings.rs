//! User settings kept between sessions (`settings.json` in the config folder).

use serde::{Deserialize, Serialize};

/// Default current-dot speed, as a factor of the base log-scale speed.
pub const DOT_SPEED: f64 = 0.35;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Current-dot speed factor (0.05 to 4).
    pub dot_speed: f64,
    pub grid: bool,
    pub volt_colors: bool,
    pub current_dots: bool,
    /// Dots move with electrons (− to +) instead of conventional current (+ to −).
    pub electron_flow: bool,
    /// Resistors as IEC rectangles instead of ANSI/IEEE zigzags.
    pub iec_resistors: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dot_speed: DOT_SPEED,
            grid: true,
            volt_colors: true,
            current_dots: true,
            electron_flow: false,
            iec_resistors: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        crate::packages::config_dir()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
            .map(|mut s| {
                s.dot_speed = s.dot_speed.clamp(0.05, 4.0);
                s
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Some(d) = crate::packages::config_dir() {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::fs::write(
                d.join("settings.json"),
                serde_json::to_string_pretty(self).unwrap_or_default(),
            );
        }
    }
}
