//! Patient dossier queries and explicit capture assignment.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use chrono::DateTime;

use crate::{
    file_validation::{
        CaptureFileVersion, capture_file_version, capture_fingerprint, system_time_nanos,
    },
    session::Eye,
    storage::create_private_directory,
};

use super::{
    CaptureKind, LIBRARY_INDEX_WRITE_LOCK, PatientRecord, StoredCaptureMetadata, StoredPatient,
    capture_transactions, ensure_local_regular_capture, indexed_capture_matches,
    load_library_index, load_library_index_for_write, lock_library_index, normalized_patient_name,
    parse_filename, save_library_index,
};

/// Finds existing dossiers by the typed names. Multiple exact homonyms are
/// returned separately; no dossier is selected automatically.
///
/// # Errors
///
/// Returns an I/O error if the library index cannot be read.
pub fn search_patients(
    directory: &Path,
    first_name: &str,
    last_name: &str,
) -> io::Result<Vec<PatientRecord>> {
    PatientSearchCache::default().search(directory, first_name, last_name, usize::MAX)
}

type IndexVersions = (Option<CaptureFileVersion>, Option<CaptureFileVersion>);

/// Reuses parsed dossiers and normalized names until either index copy changes.
/// It owns only one directory snapshot and never caches failed reads.
#[derive(Default)]
pub struct PatientSearchCache {
    directory: PathBuf,
    versions: Option<IndexVersions>,
    patients: Vec<(PatientRecord, String, String)>,
}

impl PatientSearchCache {
    /// Finds up to `limit` matching dossiers, ordered by stable dossier number.
    ///
    /// # Errors
    /// Returns errors when the library index cannot be read.
    pub fn search(
        &mut self,
        directory: &Path,
        first_name: &str,
        last_name: &str,
        limit: usize,
    ) -> io::Result<Vec<PatientRecord>> {
        let first = normalized_patient_name(first_name);
        let last = normalized_patient_name(last_name);
        if (first.is_empty() && last.is_empty()) || limit == 0 {
            return Ok(Vec::new());
        }
        let versions = index_versions(directory).ok();
        // A failed version probe disables caching. A readable primary index
        // must remain usable even if the optional backup is inaccessible.
        if self.directory != directory || versions.is_none() || self.versions != versions {
            self.versions = None;
            let index = load_library_index(directory)?;
            self.patients = index
                .patients
                .iter()
                .map(|(&id, patient)| {
                    (
                        PatientRecord::from_stored(id, patient),
                        normalized_patient_name(&patient.first_name),
                        normalized_patient_name(&patient.last_name),
                    )
                })
                .collect();
            self.patients.sort_by_key(|(patient, _, _)| patient.id);
            self.directory = directory.to_path_buf();
            self.versions =
                versions.filter(|before| index_versions(directory).ok().as_ref() == Some(before));
        }
        Ok(self
            .patients
            .iter()
            .filter(|(_, given, family)| {
                (first.is_empty() || given.starts_with(&first))
                    && (last.is_empty() || family.starts_with(&last))
            })
            .take(limit)
            .map(|(patient, _, _)| patient.clone())
            .collect())
    }
}

fn index_versions(directory: &Path) -> io::Result<IndexVersions> {
    let read = |name: &str| match capture_file_version(&directory.join(name)) {
        Ok(version) => Ok(Some(version)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    Ok((
        read(super::LIBRARY_INDEX_FILE)?,
        read(super::LIBRARY_INDEX_BACKUP_FILE)?,
    ))
}

/// Returns one dossier by its stable internal number.
///
/// # Errors
///
/// Returns an I/O error if the library index cannot be read.
pub fn get_patient(directory: &Path, id: u64) -> io::Result<Option<PatientRecord>> {
    Ok(load_library_index(directory)?
        .patients
        .get(&id)
        .map(|patient| PatientRecord::from_stored(id, patient)))
}

/// Creates a new dossier, including when another dossier has identical names.
/// Both names are required to avoid attaching captures to an incomplete identity.
///
/// # Errors
///
/// Returns an error for incomplete names or if the index cannot be updated.
pub fn create_patient(
    directory: &Path,
    first_name: &str,
    last_name: &str,
) -> io::Result<PatientRecord> {
    let first_name = first_name.trim();
    let last_name = last_name.trim();
    if first_name.is_empty() || last_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a dossier requires a first name and a last name",
        ));
    }
    let _write_guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    create_private_directory(directory)?;
    let _file_lock = lock_library_index(directory)?;
    let mut index = load_library_index_for_write(directory)?;
    let id = index
        .next_patient_id
        .max(index.patients.keys().copied().max().unwrap_or(0))
        .checked_add(1)
        .ok_or_else(|| io::Error::other("dossier numbers exhausted"))?;
    let patient = StoredPatient {
        first_name: first_name.to_owned(),
        last_name: last_name.to_owned(),
        last_capture: None,
    };
    index.version = 2;
    index.next_patient_id = id;
    index.patients.insert(id, patient.clone());
    save_library_index(directory, &index)?;
    Ok(PatientRecord::from_stored(id, &patient))
}

/// Corrects dossier names without changing its number or renaming media files.
/// The expected names protect against overwriting another application's edit.
/// # Errors
/// Returns an error for empty names, a changed dossier, or failed durable writes.
pub fn update_patient(
    directory: &Path,
    id: u64,
    expected_first: &str,
    expected_last: &str,
    first: &str,
    last: &str,
) -> io::Result<PatientRecord> {
    let first = first.trim();
    let last = last.trim();
    if first.is_empty() || last.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Le prénom et le nom sont requis.",
        ));
    }
    let _guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _file_lock = lock_library_index(directory)?;
    let mut index = load_library_index_for_write(directory)?;
    let patient = index
        .patients
        .get_mut(&id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Dossier introuvable."))?;
    if patient.first_name != expected_first || patient.last_name != expected_last {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Ce dossier a changé. Rouvrez-le avant de le modifier.",
        ));
    }
    first.clone_into(&mut patient.first_name);
    last.clone_into(&mut patient.last_name);
    let result = PatientRecord::from_stored(id, patient);
    for metadata in index
        .entries
        .values_mut()
        .filter(|entry| entry.patient_id == Some(id))
    {
        metadata.first_name = Some(first.to_owned());
        metadata.last_name = Some(last.to_owned());
    }
    save_library_index(directory, &index)?;
    Ok(result)
}

/// Explicitly assigns one unlinked legacy capture to a selected dossier.
/// No automatic association by name is made, including for a unique match.
///
/// # Errors
///
/// Returns an error if the capture is outside the library, already assigned,
/// the dossier does not exist, or the index cannot be updated.
pub fn assign_capture_to_patient(
    directory: &Path,
    file_path: &Path,
    patient_id: u64,
) -> io::Result<()> {
    assign_capture_to_patient_checked(directory, file_path, patient_id, None)
}

/// Assigns a reviewed capture only if it is still the file displayed to the user.
///
/// # Errors
///
/// Returns an error if the version changed, the file cannot be fingerprinted,
/// the dossier is invalid, or the index cannot be saved.
pub fn assign_capture_to_patient_if_unchanged(
    directory: &Path,
    file_path: &Path,
    patient_id: u64,
    expected_version: &CaptureFileVersion,
) -> io::Result<()> {
    assign_capture_to_patient_checked(directory, file_path, patient_id, Some(expected_version))
}

#[allow(clippy::too_many_lines)] // Keep validation and one index commit within the same lock scope.
fn assign_capture_to_patient_checked(
    directory: &Path,
    file_path: &Path,
    patient_id: u64,
    expected_version: Option<&CaptureFileVersion>,
) -> io::Result<()> {
    ensure_local_regular_capture(directory, file_path)?;
    let name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "capture filename is not UTF-8")
        })?;
    let kind = match file_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg" | "png") => CaptureKind::Photo,
        Some("avi" | "mp4" | "mkv") => CaptureKind::Video,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported capture type",
            ));
        }
    };
    let _write_guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _file_lock = lock_library_index(directory)?;
    let mut index = load_library_index_for_write(directory)?;
    if capture_transactions::pending_destination_names(directory)?.contains(name) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "capture publication is still in progress",
        ));
    }
    let patient = index
        .patients
        .get(&patient_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "dossier does not exist"))?
        .clone();
    let metadata = fs::metadata(file_path)?;
    let (file_version, content_sha256) = capture_fingerprint(file_path)?;
    if expected_version.is_some_and(|expected| *expected != file_version) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "capture changed since it was displayed",
        ));
    }
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    let existing = index.entries.get(name);
    let valid_existing = existing
        .map(|existing| indexed_capture_matches(existing, &metadata, &file_version))
        .transpose()?
        .unwrap_or(false);
    if valid_existing && existing.is_some_and(|entry| entry.patient_id.is_some()) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "capture already belongs to a dossier",
        ));
    }
    let (eye, date_str, time_str) = if let Some(existing) = existing.filter(|_| valid_existing) {
        (
            existing.eye,
            existing.date_str.clone(),
            existing.time_str.clone(),
        )
    } else if existing.is_some() {
        // A different file under a historical name cannot inherit its old eye/date.
        let local: DateTime<chrono::Local> = modified.into();
        (
            Eye::Unspecified,
            local.format("%Y-%m-%d").to_string(),
            local.format("%H:%M:%S").to_string(),
        )
    } else {
        let stem = file_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let (_, _, eye, date, time) = parse_filename(stem);
        if date.is_empty() || time.is_empty() {
            let local: DateTime<chrono::Local> = modified.into();
            (
                eye,
                local.format("%Y-%m-%d").to_string(),
                local.format("%H:%M:%S").to_string(),
            )
        } else {
            (eye, date, time)
        }
    };
    let capture_at = format!("{date_str} {time_str}");
    let patient_last = &mut index
        .patients
        .get_mut(&patient_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "dossier does not exist"))?
        .last_capture;
    if patient_last.as_ref().is_none_or(|last| last < &capture_at) {
        *patient_last = Some(capture_at);
    }
    index.version = 2;
    index.entries.insert(
        name.to_owned(),
        StoredCaptureMetadata {
            patient_id: Some(patient_id),
            first_name: Some(patient.first_name),
            last_name: Some(patient.last_name),
            eye,
            kind,
            date_str,
            time_str,
            file_size: Some(metadata.len()),
            modified_nanos: system_time_nanos(modified),
            content_sha256: Some(content_sha256),
            file_version: Some(file_version.clone()),
        },
    );
    if capture_file_version(file_path)? != file_version {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "capture changed before its dossier was committed",
        ));
    }
    save_library_index(directory, &index)
}
