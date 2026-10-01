//! Persistent application settings.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::storage::create_private_directory;

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

/// Preferred camera mode used for live preview and video recordings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum VideoQualityPreference {
    /// Highest available resolution, used by default.
    #[default]
    Best,
    /// A middle resolution when the camera offers one.
    Balanced,
    /// Highest available frame rate for smoother motion.
    Smooth,
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
    /// Preferred video quality and preview frame rate.
    #[serde(default)]
    pub video_quality: VideoQualityPreference,
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
            video_quality: VideoQualityPreference::default(),
            theme: AppTheme::default(),
            iridology_map_path: None,
            iridology_symbols_path: None,
            camera_control_values: BTreeMap::new(),
        }
    }
}

impl AppSettings {
    /// Loads settings, using defaults only when the file does not exist.
    ///
    /// # Errors
    ///
    /// Reports unreadable or invalid JSON instead of allowing the caller to overwrite it.
    pub fn try_load_from_file(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        match fs::metadata(parent) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        }
        // Atomic replacement makes readers safe without creating a lock file.
        // This also lets settings load from a read-only configuration directory.
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::read(settings_backup_path(path)) {
                    Ok(data) => data,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(Self::default());
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        serde_json::from_slice(&data).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid settings file {}: {error}", path.display()),
            )
        })
    }

    fn load_unlocked(path: &Path) -> io::Result<Self> {
        let backup_path = settings_backup_path(path);
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::rename(&backup_path, path) {
                    Ok(()) => fs::read(path)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(Self::default());
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        serde_json::from_slice(&data).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid settings file {}: {error}", path.display()),
            )
        })
    }

    /// Compatibility helper. Prefer `try_load_from_file` when settings may be saved again.
    #[must_use]
    pub fn load_from_file(path: &Path) -> Self {
        Self::try_load_from_file(path).unwrap_or_default()
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
            create_private_directory(parent)?;
        }
        // Serialize the read/replace sequence across application processes. Refuse
        // to overwrite unreadable or malformed settings.
        let _lock = lock_settings_file(path)?;
        let _ = Self::load_unlocked(path)?;
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut temporary_name = path.as_os_str().to_os_string();
        temporary_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT_SETTINGS_WRITE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary_path = PathBuf::from(temporary_name);
        let backup_path = settings_backup_path(path);
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut temporary_file = options.open(&temporary_path)?;
            temporary_file.write_all(&json)?;
            temporary_file.sync_all()?;
            drop(temporary_file);
            #[cfg(not(windows))]
            fs::rename(&temporary_path, path)?;
            #[cfg(windows)]
            {
                if path.exists() {
                    if backup_path.exists() {
                        fs::remove_file(&backup_path)?;
                    }
                    fs::rename(path, &backup_path)?;
                }
                if let Err(error) = fs::rename(&temporary_path, path) {
                    if backup_path.exists() {
                        let _ = fs::rename(&backup_path, path);
                    }
                    return Err(error);
                }
            }
            #[cfg(unix)]
            fs::File::open(
                path.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .sync_all()?;
            if backup_path.exists() {
                fs::remove_file(&backup_path)?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }
}

fn lock_settings_file(path: &Path) -> io::Result<File> {
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options.open(PathBuf::from(lock_name))?;
    lock.lock()?;
    Ok(lock)
}

fn settings_backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::{AppSettings, SavedCameraControlValue, VideoQualityPreference};

    #[test]
    fn settings_roundtrip_preserves_values() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("valid time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_settings_{unique}.json"));

        let mut settings = AppSettings {
            filename_template: "test_{date}".to_string(),
            video_quality: VideoQualityPreference::Smooth,
            ..AppSettings::default()
        };
        settings.camera_control_values.insert(
            "standard:Brightness".to_owned(),
            SavedCameraControlValue::Integer(42),
        );
        settings.save_to_file(&path).expect("save settings");

        let loaded = AppSettings::load_from_file(&path);
        assert_eq!(loaded.filename_template, "test_{date}");
        assert_eq!(loaded.video_quality, VideoQualityPreference::Smooth);
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
        legacy
            .as_object_mut()
            .expect("object")
            .remove("video_quality");
        let loaded: AppSettings = serde_json::from_value(legacy).expect("load legacy settings");
        assert!(loaded.camera_control_values.is_empty());
        assert_eq!(loaded.video_quality, VideoQualityPreference::Best);
    }

    #[test]
    fn invalid_settings_are_reported_and_preserved() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_bad_settings_{unique}.json"));
        let invalid = b"{ this is not JSON";
        fs::write(&path, invalid).expect("write invalid settings");
        let error = AppSettings::try_load_from_file(&path).expect_err("invalid file must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            AppSettings::default()
                .save_to_file(&path)
                .expect_err("must not overwrite invalid settings")
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).expect("read original"), invalid);
        fs::remove_file(path).expect("remove test file");
    }

    #[test]
    fn interrupted_settings_replacement_reads_backup_without_writing() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_backup_settings_{unique}.json"));
        let settings = AppSettings::default();
        settings.save_to_file(&path).expect("save settings");
        let backup = super::settings_backup_path(&path);
        fs::rename(&path, &backup).expect("simulate interrupted replacement");
        let recovered = AppSettings::try_load_from_file(&path).expect("recover settings");
        assert_eq!(recovered.filename_template, settings.filename_template);
        assert!(!path.exists());
        fs::remove_file(backup).expect("remove test file");
    }
}
