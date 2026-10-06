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

/// A successfully published backup, associated with its capture directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuccessfulBackup {
    /// Capture directory protected by this backup.
    pub source: PathBuf,
    /// Published backup directory.
    pub destination: PathBuf,
    /// Completion time as seconds since the Unix epoch.
    pub completed_at_unix: u64,
    /// Number of files in the backup.
    pub files: usize,
    /// Size of the saved files.
    pub bytes: u64,
}

/// Display preferences contain no patient or photo identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // Independent preferences, rather than a state machine.
pub struct InterfacePreferences {
    pub remember_layout: bool,
    pub sidebar_visible: bool,
    /// Zero preserves the responsive default width; other values are logical pixels.
    pub camera_panel_width: i32,
    pub viewer_panel_width: i32,
    pub consultation_panel_width: i32,
    pub photo_panel: i32,
    pub library_view: i32,
    /// 0 small, 1 medium, 2 large.
    pub thumbnail_size: i32,
    pub advanced_settings_expanded: bool,
    pub presentation_mode: bool,
    pub image_only: bool,
}

impl Default for InterfacePreferences {
    fn default() -> Self {
        Self {
            remember_layout: true,
            sidebar_visible: true,
            camera_panel_width: 0,
            viewer_panel_width: 0,
            consultation_panel_width: 0,
            photo_panel: 0,
            library_view: 0,
            thumbnail_size: 1,
            advanced_settings_expanded: false,
            presentation_mode: false,
            image_only: false,
        }
    }
}

impl InterfacePreferences {
    /// Clamp hand-edited or older preferences before using them for geometry.
    pub fn normalize(&mut self) {
        for (value, minimum, maximum) in [
            (&mut self.camera_panel_width, 260, 480),
            (&mut self.viewer_panel_width, 252, 480),
            (&mut self.consultation_panel_width, 200, 400),
        ] {
            if *value != 0 {
                *value = (*value).clamp(minimum, maximum);
            }
        }
        self.photo_panel = self.photo_panel.clamp(0, 5);
        self.library_view = self.library_view.clamp(0, 1);
        self.thumbnail_size = self.thumbnail_size.clamp(0, 2);
    }

    /// Disabling layout memory keeps explicit display options and restores panel defaults.
    #[must_use]
    pub fn initial_layout(&self) -> Self {
        if self.remember_layout {
            return self.clone();
        }
        Self {
            remember_layout: false,
            thumbnail_size: self.thumbnail_size,
            presentation_mode: self.presentation_mode,
            image_only: self.image_only,
            ..Self::default()
        }
    }
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
    /// Latest successful backup for each recently used capture directory (bounded to 20).
    #[serde(default)]
    pub backup_history: Vec<SuccessfulBackup>,
    /// Optional passive reminder in settings, after seven days without a backup.
    #[serde(default)]
    pub backup_reminder_enabled: bool,
    #[serde(default)]
    pub interface: InterfacePreferences,
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
            backup_history: Vec::new(),
            backup_reminder_enabled: false,
            interface: InterfacePreferences::default(),
        }
    }
}

impl AppSettings {
    /// Records only a successfully published backup and bounds the history.
    pub fn record_backup(&mut self, backup: SuccessfulBackup) {
        self.backup_history
            .retain(|entry| entry.source != backup.source);
        self.backup_history.insert(0, backup);
        self.backup_history.truncate(20);
    }

    /// Returns the latest successful backup of the active capture directory.
    #[must_use]
    pub fn last_backup(&self) -> Option<&SuccessfulBackup> {
        self.backup_history
            .iter()
            .filter(|entry| entry.source == self.capture_directory)
            .max_by_key(|entry| entry.completed_at_unix)
    }

    /// A missing backup or one older than seven days can trigger the optional reminder.
    #[must_use]
    pub fn backup_reminder_due(&self, now_unix: u64) -> bool {
        self.backup_reminder_enabled
            && self.last_backup().is_none_or(|entry| {
                now_unix.saturating_sub(entry.completed_at_unix) >= 7 * 24 * 60 * 60
            })
    }
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

    use super::{
        AppSettings, InterfacePreferences, SavedCameraControlValue, VideoQualityPreference,
    };

    #[test]
    fn legacy_and_partial_interface_preferences_keep_safe_defaults() {
        let mut legacy = serde_json::to_value(AppSettings::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("interface");
        let loaded: AppSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(loaded.interface, InterfacePreferences::default());
        let partial: InterfacePreferences =
            serde_json::from_str(r#"{"thumbnail_size":2}"#).unwrap();
        assert!(partial.remember_layout && partial.sidebar_visible);
        assert!(
            !partial.presentation_mode
                && !partial.image_only
                && !partial.advanced_settings_expanded
        );
        assert_eq!(partial.thumbnail_size, 2);
    }

    #[test]
    fn interface_geometry_is_bounded_and_layout_memory_can_be_disabled() {
        let mut prefs = InterfacePreferences {
            camera_panel_width: i32::MAX,
            viewer_panel_width: -100,
            consultation_panel_width: 0,
            library_view: 9,
            photo_panel: -7,
            thumbnail_size: 3,
            ..InterfacePreferences::default()
        };
        prefs.normalize();
        assert_eq!(
            (
                prefs.camera_panel_width,
                prefs.viewer_panel_width,
                prefs.consultation_panel_width
            ),
            (480, 252, 0)
        );
        assert_eq!(
            (prefs.library_view, prefs.photo_panel, prefs.thumbnail_size),
            (1, 0, 2)
        );
        prefs.remember_layout = false;
        prefs.sidebar_visible = false;
        prefs.presentation_mode = true;
        prefs.image_only = true;
        let restored = prefs.initial_layout();
        assert!(restored.sidebar_visible);
        assert_eq!(
            (
                restored.camera_panel_width,
                restored.viewer_panel_width,
                restored.library_view
            ),
            (0, 0, 0)
        );
        assert!(restored.presentation_mode && restored.image_only);
        assert_eq!(restored.thumbnail_size, 2);
    }

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
    fn legacy_settings_upgrade_preserves_paths_preferences_and_camera_controls() {
        let mut settings = AppSettings {
            capture_directory: std::path::PathBuf::from("Images/Émilie — mes captures"),
            filename_template: "{nom}_{prenom}_{oeil}_{date}".into(),
            theme: super::AppTheme::Dark,
            video_quality: VideoQualityPreference::Smooth,
            iridology_map_path: Some("Images/ma carte.jpg".into()),
            ..AppSettings::default()
        };
        settings.camera_control_values.insert(
            "standard:Brightness".into(),
            SavedCameraControlValue::Integer(42),
        );
        let mut legacy = serde_json::to_value(&settings).unwrap();
        legacy.as_object_mut().unwrap().remove("backup_history");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("backup_reminder_enabled");
        let upgraded: AppSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(upgraded, settings);
        assert!(upgraded.backup_history.is_empty());
        assert!(!upgraded.backup_reminder_enabled);
    }
    #[test]
    fn backup_history_is_bounded_and_reminders_follow_the_active_directory() {
        let mut settings = AppSettings {
            capture_directory: "captures-0".into(),
            backup_reminder_enabled: true,
            ..AppSettings::default()
        };
        assert!(settings.backup_reminder_due(1));
        for number in 0..25 {
            settings.record_backup(super::SuccessfulBackup {
                source: format!("captures-{number}").into(),
                destination: format!("backup-{number}").into(),
                completed_at_unix: 1_000_000,
                files: 2,
                bytes: 4096,
            });
        }
        assert_eq!(settings.backup_history.len(), 20);
        assert!(settings.last_backup().is_none());
        settings.capture_directory = "captures-24".into();
        assert!(!settings.backup_reminder_due(1_000_000 + 7 * 86400 - 1));
        assert!(settings.backup_reminder_due(1_000_000 + 7 * 86400));
        settings.backup_reminder_enabled = false;
        assert!(!settings.backup_reminder_due(u64::MAX));
        let serialized = serde_json::to_vec(&settings).unwrap();
        assert_eq!(
            serde_json::from_slice::<AppSettings>(&serialized).unwrap(),
            settings
        );
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
