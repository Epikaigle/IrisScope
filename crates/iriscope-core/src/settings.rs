//! Persistent application settings.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Behavior of the physical hardware button on the DE400.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum PhysicalButtonBehavior {
    /// Follows the active mode in the interface (Photo takes a photo, Video toggles recording).
    #[default]
    FollowMode,
    /// Always triggers a still photo capture.
    AlwaysPhoto,
    /// Always toggles video recording.
    AlwaysVideo,
}

/// Interface color theme.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum AppTheme {
    /// Follows the desktop environment.
    #[default]
    System,
    /// Light theme.
    Light,
    /// Dark theme.
    Dark,
}

/// Application settings persisted across restarts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Storage folder for pictures and videos.
    pub capture_directory: PathBuf,
    /// Filename pattern.
    pub filename_template: String,
    /// Hardware button behavior.
    pub physical_button_behavior: PhysicalButtonBehavior,
    /// UI theme.
    pub theme: AppTheme,
}

impl Default for AppSettings {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let capture_directory = PathBuf::from(home).join("Images").join("IrisScope");

        Self {
            capture_directory,
            filename_template: "{prenom}_{nom}_{oeil}_{date}_{heure}".to_string(),
            physical_button_behavior: PhysicalButtonBehavior::default(),
            theme: AppTheme::default(),
        }
    }
}

impl AppSettings {
    /// Loads settings from a JSON file, or returns the default if the file is missing or invalid.
    #[must_use]
    pub fn load_from_file(path: &Path) -> Self {
        if let Ok(data) = fs::read(path)
            && let Ok(settings) = serde_json::from_slice::<Self>(&data)
        {
            return settings;
        }
        Self::default()
    }

    /// Saves settings to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be created or the file written.
    pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::AppSettings;

    #[test]
    fn settings_roundtrip_preserves_values() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("valid time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_settings_{unique}.json"));

        let settings = AppSettings {
            filename_template: "test_{date}".to_string(),
            ..AppSettings::default()
        };
        settings.save_to_file(&path).expect("save settings");

        let loaded = AppSettings::load_from_file(&path);
        assert_eq!(loaded.filename_template, "test_{date}");
        let _ = fs::remove_file(path);
    }
}
