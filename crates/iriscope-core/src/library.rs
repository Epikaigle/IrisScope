//! Capture library indexing and privacy-aware presentation.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, atomic::AtomicU64},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

mod backup;
mod capture_transactions;
mod consultation;
mod index;
mod locks;
mod patients;
mod query;
mod review;
pub use backup::{
    BackupPhase, BackupProgress, BackupReport, backup_library, backup_library_with_progress,
    restore_library, restore_library_with_progress,
};
pub use consultation::{
    ConsultationNotes, consultation_dates, load_consultation_notes, save_consultation_notes,
    search_dossiers,
};
pub use patients::PatientSearchCache;
pub use query::LibraryQuery;
pub use review::{Annotation, AnnotationKind, PhotoReview, load_photo_review, save_photo_review};
mod presentation;
mod scan;
pub use crate::file_validation::capture_file_version_fast_from_file as capture_file_version_from_file_fast;
pub use crate::file_validation::{
    CaptureFileVersion, capture_file_version, capture_file_version_cancellable,
    capture_file_version_fast, capture_file_version_fast_from_file, capture_file_version_from_file,
    capture_file_version_from_file_cancellable,
};
pub use capture_transactions::{
    CaptureCommit, publish_indexed_capture, recover_pending_capture_metadata, save_indexed_capture,
};

use crate::session::Eye;

const LIBRARY_INDEX_FILE: &str = ".iriscope-index.json";
const LIBRARY_INDEX_BACKUP_FILE: &str = ".iriscope-index.json.bak";
const LIBRARY_INDEX_LOCK_FILE: &str = ".iriscope-index.lock";
static LIBRARY_INDEX_WRITE_LOCK: Mutex<()> = Mutex::new(());
static NEXT_INDEX_WRITE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LibraryIndex {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, StoredCaptureMetadata>,
    #[serde(default)]
    patients: HashMap<u64, StoredPatient>,
    #[serde(default)]
    next_patient_id: u64,
    #[serde(default)]
    reviews: HashMap<String, review::StoredReview>,
    #[serde(default)]
    consultations: HashMap<String, ConsultationNotes>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredPatient {
    first_name: String,
    last_name: String,
    #[serde(default)]
    last_capture: Option<String>,
}

/// A patient dossier. Names are searchable but never serve as a unique key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatientRecord {
    /// Stable internal identity used by captures.
    pub id: u64,
    /// Short, human-readable dossier number.
    pub dossier_number: String,
    /// First name as entered by the operator.
    pub first_name: String,
    /// Last name as entered by the operator.
    pub last_name: String,
    /// Date and time of the last indexed capture, when available.
    pub last_capture: Option<String>,
}

impl PatientRecord {
    fn from_stored(id: u64, stored: &StoredPatient) -> Self {
        Self {
            id,
            dossier_number: format!("D-{id:06}"),
            first_name: stored.first_name.clone(),
            last_name: stored.last_name.clone(),
            last_capture: stored.last_capture.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredCaptureMetadata {
    #[serde(default)]
    patient_id: Option<u64>,
    first_name: Option<String>,
    last_name: Option<String>,
    eye: Eye,
    kind: CaptureKind,
    date_str: String,
    time_str: String,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    modified_nanos: Option<u128>,
    #[serde(default)]
    content_sha256: Option<[u8; 32]>,
    #[serde(default)]
    file_version: Option<CaptureFileVersion>,
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
    /// File size observed during the scan, used to invalidate cached entries.
    #[serde(default)]
    pub file_size: u64,
    /// Version observed during the scan, for validating a later preview or action.
    #[serde(default)]
    pub file_version: Option<CaptureFileVersion>,
    /// Stable dossier identity. Legacy captures stay unassigned until reviewed.
    pub patient_id: Option<u64>,
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
    /// Modification timestamp used only if no capture timestamp is available.
    pub modified_time: SystemTime,
}

/// Cheap library row used for sorting and paging before content verification.
///
/// `patient_id_hint` is unverified and must never be displayed as an association.
/// Only `resolve_library_candidates_cancellable` may return a patient-linked entry.
#[derive(Clone, Debug)]
pub struct LibraryScanCandidate {
    pub file_path: PathBuf,
    pub kind: CaptureKind,
    pub file_size: u64,
    pub modified_time: SystemTime,
    pub date_str: String,
    pub time_str: String,
    pub patient_id_hint: Option<u64>,
    /// Unverified eye used for filtering before the displayed page is validated.
    pub eye_hint: Eye,
    native_version: Option<CaptureFileVersion>,
}

impl LibraryScanCandidate {
    /// Capture timestamp, falling back to file modification time when unavailable.
    #[must_use]
    pub fn timestamp(&self) -> chrono::NaiveDateTime {
        scan::sort_time(&self.date_str, &self.time_str, self.modified_time)
    }
}

/// An entry formatted for presentation in the user interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentedLibraryItem {
    /// Full path to the original file on disk.
    pub file_path: PathBuf,
    /// Dossier number for indexed captures, including other patients.
    pub dossier_number: Option<String>,
    /// Type of capture.
    pub kind: CaptureKind,
    /// Display title. If belonging to the selected patient, shows their name.
    /// If belonging to another patient, strictly anonymized to e.g. "Photo Iris Droit".
    pub display_title: String,
    /// Date and time for display.
    pub date_time: String,
    /// Eye label ("Œil Droit", "Œil Gauche", or "Œil non renseigné").
    pub eye_label: String,
    /// Indicates whether this capture belongs to the selected patient.
    pub is_current_patient: bool,
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

pub use patients::{
    assign_capture_to_patient, assign_capture_to_patient_if_unchanged, create_patient, get_patient,
    search_patients, update_patient,
};

use index::{
    ensure_local_regular_capture, load_library_index, load_library_index_for_write,
    lock_library_index, recover_library_index_locked, save_library_index, sync_directory,
};
pub use index::{
    record_capture_metadata, recover_library_index, recover_library_index_cancellable,
    upgrade_capture_fingerprints,
};

use scan::indexed_capture_matches;
pub use scan::{
    resolve_library_candidates_cancellable, scan_library_directory, try_scan_library_directory,
    try_scan_library_directory_metadata, try_scan_library_directory_metadata_cancellable,
};

pub use presentation::present_library_items;
use presentation::{normalized_patient_name, parse_filename};

#[cfg(test)]
mod tests;
