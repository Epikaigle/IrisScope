//! Metadata-first library listing and SHA-verified visible pages.

use std::{
    fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::{
    file_validation::{
        CaptureFileVersion, capture_file_version_cancellable, capture_file_version_fast,
        system_time_nanos,
    },
    session::Eye,
};

use super::{
    CaptureKind, LibraryEntry, LibraryScanCandidate, StoredCaptureMetadata, capture_transactions,
    ensure_local_regular_capture, load_library_index, parse_filename,
};

/// Scans a directory and returns indexed library entries sorted by newest first.
#[must_use]
pub fn scan_library_directory(directory: &Path) -> Vec<LibraryEntry> {
    try_scan_library_directory(directory).unwrap_or_default()
}

/// Scans a directory and reports read errors to the caller. A missing directory is empty.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be read.
pub fn try_scan_library_directory(directory: &Path) -> io::Result<Vec<LibraryEntry>> {
    let candidates = try_scan_library_directory_metadata(directory)?;
    let mut entries = resolve_library_candidates_cancellable(directory, &candidates, &|| false)?;
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(sort_time(
            &entry.date_str,
            &entry.time_str,
            entry.modified_time,
        ))
    });
    Ok(entries)
}

/// Lists sortable, anonymous candidates without reading media contents.
/// Identity hints are only for approximate filtering before SHA verification.
///
/// # Errors
///
/// Returns an I/O error if the index or directory cannot be read.
pub fn try_scan_library_directory_metadata(
    directory: &Path,
) -> io::Result<Vec<LibraryScanCandidate>> {
    try_scan_library_directory_metadata_cancellable(directory, &|| false)
}

/// Lists anonymous metadata candidates, stopping between directory entries.
///
/// # Errors
///
/// Returns `Interrupted` when cancelled, or an I/O error on unreadable metadata.
pub fn try_scan_library_directory_metadata_cancellable(
    directory: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<Vec<LibraryScanCandidate>> {
    let mut candidates = Vec::new();
    let read_dir = match fs::read_dir(directory) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidates),
        Err(error) => return Err(error),
    };
    let index = load_library_index(directory)?;
    let pending_destinations =
        capture_transactions::pending_destination_names_cancellable(directory, is_cancelled)?;

    for entry in read_dir {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "library scan cancelled",
            ));
        }
        let entry = entry?;
        let path = entry.path();
        // Do not follow a symlink out of the capture directory.
        if !entry.file_type()?.is_file() {
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

        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| pending_destinations.contains(name))
        {
            continue;
        }
        let file_metadata = entry.metadata()?;
        let modified_time = file_metadata.modified().unwrap_or(UNIX_EPOCH);
        let stored_metadata = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| index.entries.get(name));
        let (date_str, time_str) = if let Some(stored) = stored_metadata {
            (stored.date_str.clone(), stored.time_str.clone())
        } else {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("");
            let (_, _, _, date, time) = parse_filename(stem);
            (date, time)
        };
        let native_version = match capture_file_version_fast(&path) {
            Ok(version) => Some(version),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Unsupported
                ) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        candidates.push(LibraryScanCandidate {
            file_path: path.clone(),
            kind,
            file_size: file_metadata.len(),
            modified_time,
            date_str,
            time_str,
            patient_id_hint: stored_metadata
                .and_then(|stored| stored.patient_id)
                .filter(|id| index.patients.contains_key(id)),
            eye_hint: stored_metadata.map_or_else(
                || {
                    let stem = path
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or("");
                    parse_filename(stem).2
                },
                |stored| stored.eye,
            ),
            native_version,
        });
    }

    candidates.sort_by_key(|candidate| {
        std::cmp::Reverse(sort_time(
            &candidate.date_str,
            &candidate.time_str,
            candidate.modified_time,
        ))
    });
    Ok(candidates)
}

/// Resolves one visible page using full SHA versions before releasing identity.
/// Cancellation is checked between files and between hash blocks.
///
/// # Errors
///
/// Returns `Interrupted` if cancelled or a candidate changed since scanning.
pub fn resolve_library_candidates_cancellable(
    directory: &Path,
    candidates: &[LibraryScanCandidate],
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<Vec<LibraryEntry>> {
    if candidates.is_empty() {
        return if is_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "library scan cancelled",
            ))
        } else {
            Ok(Vec::new())
        };
    }
    let index = load_library_index(directory)?;
    // A journal can be installed after the metadata scan selected a path.
    // Never resolve a destination while its media copy is still in progress.
    let pending_destinations =
        capture_transactions::pending_destination_names_cancellable(directory, is_cancelled)?;
    let mut entries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "library scan cancelled",
            ));
        }
        let path = &candidate.file_path;
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| pending_destinations.contains(name))
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "capture publication is still in progress",
            ));
        }
        let (file_metadata, file_version) =
            validate_scanned_candidate(directory, candidate, is_cancelled)?;
        let stored_metadata = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| index.entries.get(name));
        let indexed_metadata = match stored_metadata {
            Some(stored) if indexed_capture_matches(stored, &file_metadata, &file_version)? => {
                Some(stored)
            }
            _ => None,
        };

        let (patient_id, first_name, last_name, eye, date_str, time_str, indexed_kind) =
            if let Some(metadata) = indexed_metadata {
                (
                    metadata
                        .patient_id
                        .filter(|id| index.patients.contains_key(id)),
                    metadata.first_name.clone(),
                    metadata.last_name.clone(),
                    metadata.eye,
                    metadata.date_str.clone(),
                    metadata.time_str.clone(),
                    Some(metadata.kind),
                )
            } else if stored_metadata.is_some() {
                // A file with the same name replaced an indexed capture. Do not
                // assign the previous patient's identity or parse it from the name.
                (
                    None,
                    None,
                    None,
                    Eye::Unspecified,
                    String::new(),
                    String::new(),
                    None,
                )
            } else {
                let file_stem = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("");
                let (first_name, last_name, eye, date_str, time_str) = parse_filename(file_stem);
                (None, first_name, last_name, eye, date_str, time_str, None)
            };

        entries.push(LibraryEntry {
            file_path: path.clone(),
            file_size: file_metadata.len(),
            file_version: Some(file_version),
            patient_id,
            kind: indexed_kind.unwrap_or(candidate.kind),
            first_name,
            last_name,
            eye,
            date_str,
            time_str,
            modified_time: candidate.modified_time,
        });
    }
    Ok(entries)
}

fn validate_scanned_candidate(
    directory: &Path,
    candidate: &LibraryScanCandidate,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<(fs::Metadata, CaptureFileVersion)> {
    let path = &candidate.file_path;
    ensure_local_regular_capture(directory, path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.len() != candidate.file_size
        || metadata.modified().unwrap_or(UNIX_EPOCH) != candidate.modified_time
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "capture changed since metadata scan",
        ));
    }
    let version = capture_file_version_cancellable(path, is_cancelled)?;
    if candidate
        .native_version
        .as_ref()
        .is_some_and(|previous| !previous.same_native_metadata(&version))
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "capture changed since metadata scan",
        ));
    }
    Ok((metadata, version))
}

pub(super) fn indexed_capture_matches(
    stored: &StoredCaptureMetadata,
    metadata: &fs::Metadata,
    current_version: &CaptureFileVersion,
) -> io::Result<bool> {
    if stored.file_size.is_some_and(|size| size != metadata.len()) {
        return Ok(false);
    }
    if let Some(expected_digest) = stored.content_sha256 {
        // The current version already includes the SHA of the opened media.
        // Byte-identical backups retain their dossier even if native metadata differs.
        return Ok(current_version.content_digest() == Some(expected_digest));
    }
    let legacy_matches = stored
        .modified_nanos
        .is_none_or(|stamp| metadata.modified().ok().and_then(system_time_nanos) == Some(stamp));
    if legacy_matches && stored.patient_id.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Les captures liées à un dossier nécessitent la mise à niveau de leur empreinte. Ouvrez une bibliothèque accessible en écriture puis actualisez-la.",
        ));
    }
    Ok(legacy_matches)
}

pub(super) fn sort_time(date: &str, time: &str, modified_time: SystemTime) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M:%S")
        .unwrap_or_else(|_| DateTime::<Utc>::from(modified_time).naive_utc())
}
