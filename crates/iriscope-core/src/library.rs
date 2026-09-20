//! Capture library indexing and privacy-aware presentation.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{
    session::{CaptureSession, Eye},
    storage::CaptureTimestamp,
};

const LIBRARY_INDEX_FILE: &str = ".iriscope-index.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LibraryIndex {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, StoredCaptureMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredCaptureMetadata {
    first_name: Option<String>,
    last_name: Option<String>,
    eye: Eye,
    kind: CaptureKind,
    date_str: String,
    time_str: String,
}

/// The type of capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaptureKind {
    /// Still photograph.
    Photo,
    /// Video clip.
    Video,
}

/// Metadata extracted from a capture file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LibraryEntry {
    /// Full path to the file on disk.
    pub file_path: PathBuf,
    /// Type of capture.
    pub kind: CaptureKind,
    /// Patient first name (if parsed from filename).
    pub first_name: Option<String>,
    /// Patient last name (if parsed from filename).
    pub last_name: Option<String>,
    /// Associated eye.
    pub eye: Eye,
    /// Formatted date string (YYYY-MM-DD).
    pub date_str: String,
    /// Formatted time string (HH:MM:SS).
    pub time_str: String,
    /// Modification timestamp for sorting.
    pub modified_time: SystemTime,
}

/// An entry formatted for presentation in the user interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentedLibraryItem {
    /// Full path to the original file on disk.
    pub file_path: PathBuf,
    /// Type of capture.
    pub kind: CaptureKind,
    /// Display title. If belonging to the current patient session, shows their name.
    /// If belonging to another patient, strictly anonymized to e.g. "Photo Iris Droit".
    pub display_title: String,
    /// Date and time for display.
    pub date_time: String,
    /// Eye label ("Œil Droit", "Œil Gauche", or "Œil non renseigné").
    pub eye_label: String,
    /// Indicates whether this capture belongs to the currently active patient session.
    pub is_current_session: bool,
}

/// Filter for browsing the library.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LibraryFilter {
    /// All media.
    #[default]
    All,
    /// Photographs only.
    PhotosOnly,
    /// Video clips only.
    VideosOnly,
}

/// Records reliable metadata for a newly created capture.
///
/// The hidden index is used by the library instead of trying to infer identity from a
/// customizable filename. Existing captures without an index entry continue to use
/// filename parsing as a compatibility fallback.
///
/// # Errors
///
/// Returns an I/O error when the index cannot be created or written.
pub fn record_capture_metadata(
    directory: &Path,
    file_path: &Path,
    session: &CaptureSession,
    kind: CaptureKind,
    timestamp: CaptureTimestamp,
) -> io::Result<()> {
    fs::create_dir_all(directory)?;

    let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture path does not contain a valid UTF-8 filename",
        ));
    };

    let mut index = load_library_index(directory);
    index.version = 1;
    index.entries.insert(
        file_name.to_owned(),
        StoredCaptureMetadata {
            first_name: (!session.first_name().is_empty())
                .then(|| session.first_name().to_owned()),
            last_name: (!session.last_name().is_empty()).then(|| session.last_name().to_owned()),
            eye: session.eye(),
            kind,
            date_str: format!(
                "{:04}-{:02}-{:02}",
                timestamp.year, timestamp.month, timestamp.day
            ),
            time_str: format!(
                "{:02}:{:02}:{:02}",
                timestamp.hour, timestamp.minute, timestamp.second
            ),
        },
    );

    save_library_index(directory, &index)
}

fn load_library_index(directory: &Path) -> LibraryIndex {
    let path = directory.join(LIBRARY_INDEX_FILE);
    fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn save_library_index(directory: &Path, index: &LibraryIndex) -> io::Result<()> {
    let final_path = directory.join(LIBRARY_INDEX_FILE);
    let temporary_path = directory.join(".iriscope-index.json.tmp");
    let data = serde_json::to_vec_pretty(index)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    fs::write(&temporary_path, data)?;
    if let Err(error) = fs::rename(&temporary_path, &final_path) {
        if final_path.exists() {
            fs::remove_file(&final_path)?;
            fs::rename(&temporary_path, &final_path)?;
        } else {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
    }
    Ok(())
}

/// Scans a directory and returns indexed library entries sorted by newest first.
#[must_use]
pub fn scan_library_directory(directory: &Path) -> Vec<LibraryEntry> {
    let mut entries = Vec::new();
    let index = load_library_index(directory);
    let Ok(read_dir) = fs::read_dir(directory) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };

        let kind = match extension.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" | "png" => CaptureKind::Photo,
            "avi" | "mp4" | "mkv" => CaptureKind::Video,
            _ => continue,
        };

        let modified_time = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let indexed_metadata = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| index.entries.get(name));

        let (first_name, last_name, eye, date_str, time_str, indexed_kind) =
            if let Some(metadata) = indexed_metadata {
                (
                    metadata.first_name.clone(),
                    metadata.last_name.clone(),
                    metadata.eye,
                    metadata.date_str.clone(),
                    metadata.time_str.clone(),
                    Some(metadata.kind),
                )
            } else {
                let file_stem = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("");
                let (first_name, last_name, eye, date_str, time_str) =
                    parse_filename(file_stem);
                (first_name, last_name, eye, date_str, time_str, None)
            };

        entries.push(LibraryEntry {
            file_path: path,
            kind: indexed_kind.unwrap_or(kind),
            first_name,
            last_name,
            eye,
            date_str,
            time_str,
            modified_time,
        });
    }

    // Sort newest first
    entries.sort_by(|a, b| b.modified_time.cmp(&a.modified_time));
    entries
}

/// Formats indexed entries according to medical confidentiality rules.
///
/// If `active_session` has patient identity and matches the entry's patient,
/// the identity is displayed. All other records are strictly anonymized.
#[must_use]
pub fn present_library_items(
    entries: &[LibraryEntry],
    active_session: &CaptureSession,
    filter: LibraryFilter,
) -> Vec<PresentedLibraryItem> {
    entries
        .iter()
        .filter(|entry| match filter {
            LibraryFilter::All => true,
            LibraryFilter::PhotosOnly => entry.kind == CaptureKind::Photo,
            LibraryFilter::VideosOnly => entry.kind == CaptureKind::Video,
        })
        .map(|entry| {
            let matches_session = active_session.has_identity()
                && entry
                    .first_name
                    .as_deref()
                    .is_some_and(|f| f.eq_ignore_ascii_case(active_session.first_name()))
                && entry
                    .last_name
                    .as_deref()
                    .is_some_and(|l| l.eq_ignore_ascii_case(active_session.last_name()));

            let eye_text = match entry.eye {
                Eye::Left => "Œil Gauche",
                Eye::Right => "Œil Droit",
                Eye::Unspecified => "Œil non renseigné",
            };

            let kind_text = match entry.kind {
                CaptureKind::Photo => "Photo",
                CaptureKind::Video => "Vidéo",
            };

            let display_title = if matches_session {
                let name = format!(
                    "{} {}",
                    active_session.first_name(),
                    active_session.last_name()
                );
                format!("{name} ({eye_text})")
            } else {
                format!("{kind_text} {eye_text}")
            };

            let date_time = format!("{} {}", entry.date_str, entry.time_str);

            PresentedLibraryItem {
                file_path: entry.file_path.clone(),
                kind: entry.kind,
                display_title,
                date_time,
                eye_label: eye_text.to_string(),
                is_current_session: matches_session,
            }
        })
        .collect()
}

fn parse_filename(stem: &str) -> (Option<String>, Option<String>, Eye, String, String) {
    let parts: Vec<&str> = stem.split('_').collect();

    // Standard pattern: {prenom}_{nom}_{oeil}_{date}_{heure} (5 parts)
    // Anonymous pattern: Iris_{oeil}_{date}_{heure} (4 parts)
    if parts.len() >= 5 {
        let first_name = parts[0];
        let last_name = parts[1];
        let eye = parse_eye(parts[2]);
        let date_str = parts[3].to_string();
        let time_str = parts[4].replace('-', ":");

        let (fn_opt, ln_opt) = if first_name == "Iris" {
            (None, None)
        } else {
            (Some(first_name.to_string()), Some(last_name.to_string()))
        };

        (fn_opt, ln_opt, eye, date_str, time_str)
    } else if parts.len() == 4 && parts[0] == "Iris" {
        let eye = parse_eye(parts[1]);
        let date_str = parts[2].to_string();
        let time_str = parts[3].replace('-', ":");
        (None, None, eye, date_str, time_str)
    } else {
        (None, None, Eye::Unspecified, String::new(), String::new())
    }
}

fn parse_eye(label: &str) -> Eye {
    match label.to_lowercase().as_str() {
        "gauche" | "left" => Eye::Left,
        "droit" | "right" => Eye::Right,
        _ => Eye::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::SystemTime};

    use crate::{
        session::{CaptureSession, Eye},
        storage::CaptureTimestamp,
    };

    use super::{
        CaptureKind, LibraryEntry, LibraryFilter, present_library_items, record_capture_metadata,
        scan_library_directory,
    };

    #[test]
    fn privacy_anonymizes_other_patients() {
        let current_session = CaptureSession::new("Jean", "Dupont", Eye::Right);

        let entries = vec![
            LibraryEntry {
                file_path: PathBuf::from("/captures/Jean_Dupont_Droit_2026-09-20_14-30-00.jpg"),
                kind: CaptureKind::Photo,
                first_name: Some("Jean".to_string()),
                last_name: Some("Dupont".to_string()),
                eye: Eye::Right,
                date_str: "2026-09-20".to_string(),
                time_str: "14:30:00".to_string(),
                modified_time: SystemTime::UNIX_EPOCH,
            },
            LibraryEntry {
                file_path: PathBuf::from("/captures/Marie_Curie_Gauche_2026-09-19_10-15-00.jpg"),
                kind: CaptureKind::Photo,
                first_name: Some("Marie".to_string()),
                last_name: Some("Curie".to_string()),
                eye: Eye::Left,
                date_str: "2026-09-19".to_string(),
                time_str: "10:15:00".to_string(),
                modified_time: SystemTime::UNIX_EPOCH,
            },
        ];

        let presented = present_library_items(&entries, &current_session, LibraryFilter::All);

        assert_eq!(presented.len(), 2);
        // Current session item displays patient name
        assert!(presented[0].display_title.contains("Jean Dupont"));
        assert!(presented[0].is_current_session);

        // Previous patient capture is anonymized
        assert_eq!(presented[1].display_title, "Photo Œil Gauche");
        assert!(!presented[1].display_title.contains("Marie"));
        assert!(!presented[1].display_title.contains("Curie"));
        assert!(!presented[1].is_current_session);
    }

    #[test]
    fn scan_directory_finds_and_sorts_files() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("iris_test_lib_{unique}"));
        std::fs::create_dir_all(&dir).expect("create test dir");

        let file1 = dir.join("Jean_Dupont_Droit_2026-09-20_14-30-00.jpg");
        std::fs::write(&file1, b"test").expect("write test file");

        let entries = scan_library_directory(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].first_name.as_deref(), Some("Jean"));
        assert_eq!(entries[0].last_name.as_deref(), Some("Dupont"));
        assert_eq!(entries[0].eye, Eye::Right);

        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn index_preserves_identity_with_custom_filename() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("iris_test_index_{unique}"));
        std::fs::create_dir_all(&dir).expect("create test dir");

        let file = dir.join("capture-personnalisee-001.jpg");
        std::fs::write(&file, b"test").expect("write test file");
        let session = CaptureSession::new("Jean Pierre", "Du Pont", Eye::Left);
        let timestamp = CaptureTimestamp {
            year: 2026,
            month: 9,
            day: 20,
            hour: 18,
            minute: 45,
            second: 12,
        };

        record_capture_metadata(
            &dir,
            &file,
            &session,
            CaptureKind::Photo,
            timestamp,
        )
        .expect("record capture metadata");

        let entries = scan_library_directory(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].first_name.as_deref(), Some("Jean Pierre"));
        assert_eq!(entries[0].last_name.as_deref(), Some("Du Pont"));
        assert_eq!(entries[0].eye, Eye::Left);
        assert_eq!(entries[0].date_str, "2026-09-20");
        assert_eq!(entries[0].time_str, "18:45:12");

        std::fs::remove_dir_all(dir).expect("remove test directory");
    }

}
