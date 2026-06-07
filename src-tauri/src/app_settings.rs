use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use similarity_core::types::{AppError, SessionError};

fn default_rf_chapter_preset() -> String {
    "full_multi_pass".to_string()
}

/// Persisted app-wide UI preferences (not per-job display state).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    #[serde(default = "default_rf_chapter_preset")]
    pub rf_chapter_preset: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            rf_chapter_preset: default_rf_chapter_preset(),
        }
    }
}

pub fn app_settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("app_settings.json")
}

pub fn load_app_settings(app_data_dir: &Path) -> AppSettings {
    let path = app_settings_path(app_data_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return AppSettings::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_app_settings(app_data_dir: &Path, settings: &AppSettings) -> Result<(), AppError> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to create app data directory: {}", e),
        })
    })?;

    let json = serde_json::to_string_pretty(settings).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to serialize app settings: {}", e),
        })
    })?;

    std::fs::write(app_settings_path(app_data_dir), json).map_err(|e| {
        AppError::Session(SessionError {
            message: format!("Failed to write app settings: {}", e),
        })
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_preset_is_full_multi_pass() {
        assert_eq!(AppSettings::default().rf_chapter_preset, "full_multi_pass");
    }

    #[test]
    fn round_trip_app_settings() {
        let dir = std::env::temp_dir().join(format!(
            "similarity_map_app_settings_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = AppSettings {
            rf_chapter_preset: "act_fine".into(),
        };
        save_app_settings(&dir, &settings).unwrap();
        assert_eq!(load_app_settings(&dir), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
