//! Persistent application settings.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SETTINGS_WRITE_ID: AtomicU64 = AtomicU64::new(0);

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

/// An image control value saved independently of the camera backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SavedCameraControlValue {
    /// A bounded integer control.
    Integer(i64),
    /// An on/off control.
    Boolean(bool),
    /// A value chosen from the camera's menu.
    Menu(i64),
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
    /// Optional iridology chart image used as a visual reference.
    #[serde(default)]
    pub iridology_map_path: Option<PathBuf>,
    /// Optional image containing iridology signs/symbols used as a visual reference.
    #[serde(default)]
    pub iridology_symbols_path: Option<PathBuf>,
    /// Values selected for writable image controls, indexed by stable control key.
    #[serde(default)]
    pub camera_control_values: BTreeMap<String, SavedCameraControlValue>,
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
            iridology_map_path: None,
            iridology_symbols_path: None,
            camera_control_values: BTreeMap::new(),
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
    /// Returns an I/O error if the directory cannot be created or the file replaced.
    pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut temporary_name = path.as_os_str().to_os_string();
        temporary_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT_SETTINGS_WRITE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary_path = PathBuf::from(temporary_name);
        let result = (|| {
            let mut temporary_file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            temporary_file.write_all(&json)?;
            temporary_file.sync_all()?;
            drop(temporary_file);
            fs::rename(&temporary_path, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::{AppSettings, SavedCameraControlValue};

    #[test]
    fn settings_roundtrip_preserves_values() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("valid time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_settings_{unique}.json"));

        let mut settings = AppSettings {
            filename_template: "test_{date}".to_string(),
            ..AppSettings::default()
        };
        settings.camera_control_values.insert(
            "standard:Brightness".to_owned(),
            SavedCameraControlValue::Integer(42),
        );
        settings.save_to_file(&path).expect("save settings");

        let loaded = AppSettings::load_from_file(&path);
        assert_eq!(loaded.filename_template, "test_{date}");
        assert_eq!(loaded.camera_control_values, settings.camera_control_values);
        let replacement = AppSettings {
            filename_template: "autre_{prenom}_{nom}_{oeil}".to_string(),
            ..AppSettings::default()
        };
        replacement.save_to_file(&path).expect("replace settings");
        let loaded = AppSettings::load_from_file(&path);
        assert_eq!(loaded.filename_template, replacement.filename_template);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn settings_without_camera_controls_load_with_empty_values() {
        let mut legacy = serde_json::to_value(AppSettings::default()).expect("serialize settings");
        legacy
            .as_object_mut()
            .expect("object")
            .remove("camera_control_values");
        let loaded: AppSettings = serde_json::from_value(legacy).expect("load legacy settings");
        assert!(loaded.camera_control_values.is_empty());
    }
}
