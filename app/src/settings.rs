use std::path::PathBuf;

use baseus_protocol::ANC_DEFAULT_LEVEL;
use serde::{Deserialize, Serialize};

use crate::theme::Style;

/// Preferences that survive a restart.
///
/// "Start with Windows" is not stored here on purpose: the registry is the
/// truth, so the app and the Task Manager cannot disagree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub minimize_to_tray: bool,
    pub low_battery_alerts: bool,
    #[serde(default)]
    pub style: Style,
    /// Raw firmware values, so this module never depends on protocol enums.
    pub anc_mode: u8,
    pub anc_level: u8,
    pub eq_preset: u8,
    pub game_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            minimize_to_tray: true,
            low_battery_alerts: true,
            style: Style::HiFi,
            anc_mode: 0,
            anc_level: ANC_DEFAULT_LEVEL,
            eq_preset: 0,
            game_mode: false,
        }
    }
}

fn path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|base| {
        PathBuf::from(base)
            .join("AudifonosBaseus")
            .join("settings.json")
    })
}

pub fn load() -> Settings {
    let Some(file) = path() else {
        return Settings::default();
    };
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let Some(file) = path() else {
        return Ok(());
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_without_style_load_as_hifi() {
        let old = r#"{"minimize_to_tray":false,"low_battery_alerts":true}"#;
        let settings: Settings = serde_json::from_str(old).expect("legacy settings");
        assert_eq!(settings.style, Style::HiFi);
        assert!(!settings.minimize_to_tray);
        assert_eq!(settings.anc_mode, 0);
        assert_eq!(settings.anc_level, ANC_DEFAULT_LEVEL);
        assert_eq!(settings.eq_preset, 0);
        assert!(!settings.game_mode);
    }

    #[test]
    fn device_preferences_round_trip() {
        let settings = Settings {
            anc_mode: 2,
            anc_level: 0x80,
            eq_preset: 3,
            game_mode: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let back: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.anc_mode, 2);
        assert_eq!(back.anc_level, 0x80);
        assert_eq!(back.eq_preset, 3);
        assert!(back.game_mode);
    }

    #[test]
    fn style_round_trips() {
        for style in [Style::HiFi, Style::Sonoma] {
            let settings = Settings {
                style,
                ..Settings::default()
            };
            let json = serde_json::to_string(&settings).expect("serialize");
            let back: Settings = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back.style, style);
        }
    }
}
