//! Durable library index, recovery, locking, and legacy migration.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    file_validation::{
        CaptureFileVersion, capture_file_version, capture_file_version_cancellable,
        capture_fingerprint, system_time_nanos,
    },
    session::CaptureSession,
    storage::{CaptureTimestamp, create_private_directory},
};

use super::{
    CaptureKind, LIBRARY_INDEX_BACKUP_FILE, LIBRARY_INDEX_FILE, LIBRARY_INDEX_LOCK_FILE,
    LIBRARY_INDEX_WRITE_LOCK, LibraryIndex, NEXT_INDEX_WRITE_ID, StoredCaptureMetadata,
    capture_transactions, normalized_patient_name,
};

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
    // Photo capture and video recording can update the same index on different threads.
    let _write_guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    create_private_directory(directory)?;

    let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture path does not contain a valid UTF-8 filename",
        ));
    };
    ensure_local_regular_capture(directory, file_path)?;

    let _file_lock = lock_library_index(directory)?;
    let mut index = load_library_index_for_write(directory)?;
    if capture_transactions::pending_destination_names(directory)?.contains(file_name) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "capture publication is still in progress",
        ));
    }
    // Do not rescan every historical capture on each save. Explicit maintenance
    // can prune missing files without making capture time proportional to library size.
    if let Some(id) = session.patient_id() {
        let patient = index.patients.get(&id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "selected dossier does not exist",
            )
        })?;
        if normalized_patient_name(&patient.first_name)
            != normalized_patient_name(session.first_name())
            || normalized_patient_name(&patient.last_name)
                != normalized_patient_name(session.last_name())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "selected dossier does not match session names",
            ));
        }
    }
    let file_metadata = fs::metadata(file_path)?;
    let (file_version, content_sha256) = capture_fingerprint(file_path)?;
    index.version = 2;
    let capture_at = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        timestamp.year,
        timestamp.month,
        timestamp.day,
        timestamp.hour,
        timestamp.minute,
        timestamp.second
    );
    if let Some(id) = session.patient_id() {
        let patient = index.patients.get_mut(&id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "selected dossier does not exist",
            )
        })?;
        if patient
            .last_capture
            .as_ref()
            .is_none_or(|last| last < &capture_at)
        {
            patient.last_capture = Some(capture_at);
        }
    }
    index.entries.insert(
        file_name.to_owned(),
        StoredCaptureMetadata {
            patient_id: session.patient_id(),
            first_name: (!session.first_name().is_empty()).then(|| session.first_name().to_owned()),
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
            file_size: Some(file_metadata.len()),
            modified_nanos: file_metadata.modified().ok().and_then(system_time_nanos),
            content_sha256: Some(content_sha256),
            file_version: Some(file_version.clone()),
        },
    );

    if capture_file_version(file_path)? != file_version {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "capture changed before metadata was committed",
        ));
    }
    save_library_index(directory, &index)
}

pub(super) fn ensure_local_regular_capture(directory: &Path, file_path: &Path) -> io::Result<()> {
    if fs::canonicalize(file_path.parent().unwrap_or(Path::new(".")))?
        != fs::canonicalize(directory)?
        || !fs::symlink_metadata(file_path)?.file_type().is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture is not a regular file in this library",
        ));
    }
    Ok(())
}

pub(super) fn lock_library_index(directory: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(directory.join(LIBRARY_INDEX_LOCK_FILE))?;
    file.lock()?;
    Ok(file)
}

pub(super) fn lock_library_index_cancellable(
    directory: &Path,
    cancel: &dyn Fn() -> bool,
) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(directory.join(LIBRARY_INDEX_LOCK_FILE))?;
    super::locks::file(&file, cancel)?;
    Ok(file)
}

pub(super) fn load_library_index(directory: &Path) -> io::Result<LibraryIndex> {
    let path = directory.join(LIBRARY_INDEX_FILE);
    let backup = directory.join(LIBRARY_INDEX_BACKUP_FILE);
    let primary = read_index_file(&path);
    match primary {
        Ok(Some(index)) => Ok(index),
        Ok(None) => Ok(read_index_file(&backup)?.unwrap_or_default()),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            match read_index_file(&backup) {
                Ok(Some(index)) => Ok(index),
                _ => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn read_index_file(path: &Path) -> io::Result<Option<LibraryIndex>> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let index: LibraryIndex = serde_json::from_slice(&data).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid library index {}: {error}", path.display()),
        )
    })?;
    if index.version > 2 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("unsupported library index version {}", index.version),
        ));
    }
    Ok(Some(index))
}

pub(super) fn load_library_index_for_write(directory: &Path) -> io::Result<LibraryIndex> {
    recover_library_index_locked(directory)?;
    // Pending media transactions are replayed under the publication lock by
    // explicit recovery. Other index writers must never race an active copy.
    load_library_index(directory)
}

/// Preserves malformed index bytes in a timestamped quarantine file and restores
/// a valid backup when available. Unknown future versions are never rewritten.
/// Returns whether a recovery was necessary.
///
/// # Errors
///
/// Returns an error if neither index copy is valid or recovery cannot be saved.
pub fn recover_library_index(directory: &Path) -> io::Result<bool> {
    recover_library_index_cancellable(directory, &|| false)
}

/// Recovers the library and migrates legacy fingerprints with cooperative cancellation.
///
/// Cancellation stops between legacy entries and SHA blocks, before committing
/// their upgraded index. Journal replay and atomic index writes run to completion
/// once started so an interrupted publication retains its durable recovery state.
/// Waiting for publication and index locks remains cancellable.
///
/// # Errors
///
/// Returns `Interrupted` when cancelled, or an I/O error if recovery or migration fails.
pub fn recover_library_index_cancellable(
    directory: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<bool> {
    check_recovery_cancelled(is_cancelled)?;
    if !directory.exists() {
        return Ok(false);
    }
    let _publication_guard =
        capture_transactions::lock_transaction_serial_cancellable(directory, is_cancelled)?;
    check_recovery_cancelled(is_cancelled)?;
    let _write_guard = super::locks::mutex(&LIBRARY_INDEX_WRITE_LOCK, is_cancelled)?;
    let _file_lock = lock_library_index_cancellable(directory, is_cancelled)?;
    check_recovery_cancelled(is_cancelled)?;
    let restored = recover_library_index_locked(directory)?;
    let mut index = load_library_index(directory)?;
    let resumed = capture_transactions::recover_pending_locked(directory, &mut index)?;
    let upgraded = upgrade_capture_fingerprints_locked(directory, &mut index, is_cancelled)?;
    Ok(restored || resumed > 0 || upgraded > 0)
}

/// Establishes content baselines for historical capture metadata under the index lock.
///
/// Existing dossier choices are retained only when their stored size and mtime
/// still match. Changes made before the first content baseline cannot be detected
/// retrospectively. Files without a previous dossier are never assigned by name.
///
/// # Errors
///
/// Returns an error if hashing or the single upgraded index write fails.
pub fn upgrade_capture_fingerprints(directory: &Path) -> io::Result<usize> {
    if !directory.exists() {
        return Ok(0);
    }
    let _write_guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _file_lock = lock_library_index(directory)?;
    let mut index = load_library_index_for_write(directory)?;
    upgrade_capture_fingerprints_locked(directory, &mut index, &|| false)
}

fn upgrade_capture_fingerprints_locked(
    directory: &Path,
    index: &mut LibraryIndex,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<usize> {
    let mut upgraded = 0;
    for (name, stored) in &mut index.entries {
        check_recovery_cancelled(is_cancelled)?;
        if stored.content_sha256.is_some() {
            continue;
        }
        crate::storage::validate_capture_file_name(name)?;
        let path = directory.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let modified = metadata.modified().ok().and_then(system_time_nanos);
        if stored.file_size != Some(metadata.len()) || stored.modified_nanos != modified {
            continue;
        }
        let version = capture_file_version_cancellable(&path, is_cancelled)?;
        let digest = version
            .content_digest()
            .ok_or_else(|| io::Error::other("complete capture version has no digest"))?;
        if stored
            .file_version
            .as_ref()
            .and_then(CaptureFileVersion::content_digest)
            .is_some_and(|previous| previous != digest)
        {
            // A transitional index may have a verified version but no separate
            // digest. Never rebaseline it after the capture changed.
            continue;
        }
        stored.file_version = Some(version);
        stored.content_sha256 = Some(digest);
        upgraded += 1;
    }
    check_recovery_cancelled(is_cancelled)?;
    if upgraded > 0 {
        save_library_index(directory, index)?;
    }
    Ok(upgraded)
}

fn check_recovery_cancelled(is_cancelled: &dyn Fn() -> bool) -> io::Result<()> {
    if is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "library recovery cancelled",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn recover_library_index_locked(directory: &Path) -> io::Result<bool> {
    let primary = directory.join(LIBRARY_INDEX_FILE);
    let backup = directory.join(LIBRARY_INDEX_BACKUP_FILE);
    let primary_result = read_index_file(&primary);
    if matches!(primary_result.as_ref(), Ok(Some(_))) {
        return Ok(false);
    }
    if let Err(ref error) = primary_result
        && error.kind() != io::ErrorKind::InvalidData
    {
        return Err(io::Error::new(error.kind(), error.to_string()));
    }
    let recovered = match read_index_file(&backup)? {
        Some(index) => index,
        None if primary_result.is_ok() => return Ok(false),
        None => {
            // The index is the only reliable link between a capture and a
            // dossier. Never replace an unrecoverable index with an empty one.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "library index is corrupt and no valid backup exists; original files were preserved",
            ));
        }
    };
    if primary_result.is_err() {
        quarantine_index(&primary)?;
    }
    save_library_index(directory, &recovered)?;
    Ok(true)
}

fn quarantine_index(path: &Path) -> io::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let quarantine = path.with_file_name(format!(
        "{}.corrupt-{}-{nanos}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("index"),
        std::process::id()
    ));
    fs::rename(path, quarantine)
}

pub(super) fn save_library_index(directory: &Path, index: &LibraryIndex) -> io::Result<()> {
    let final_path = directory.join(LIBRARY_INDEX_FILE);
    let backup_path = directory.join(LIBRARY_INDEX_BACKUP_FILE);
    let write_id = NEXT_INDEX_WRITE_ID.fetch_add(1, Ordering::Relaxed);
    let temporary_path = directory.join(format!(
        ".iriscope-index.json.{}.{}.tmp",
        std::process::id(),
        write_id
    ));
    let backup_temporary_path = directory.join(format!(
        ".iriscope-index.json.bak.{}.{}.tmp",
        std::process::id(),
        write_id
    ));
    let data = serde_json::to_vec_pretty(index)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut temporary = options.open(&temporary_path)?;
        temporary.write_all(&data)?;
        temporary.sync_all()?;
        drop(temporary);

        // Keep a complete copy of the new snapshot before replacing the
        // primary. A corrupt primary must never lose the latest dossier.
        let mut backup_source = File::open(&temporary_path)?;
        let mut backup_options = OpenOptions::new();
        backup_options.write(true).create_new(true);
        #[cfg(unix)]
        backup_options.mode(0o600);
        let mut backup_temporary = backup_options.open(&backup_temporary_path)?;
        io::copy(&mut backup_source, &mut backup_temporary)?;
        backup_temporary.sync_all()?;
        drop(backup_temporary);
        #[cfg(windows)]
        if backup_path.exists() {
            fs::remove_file(&backup_path)?;
        }
        fs::rename(&backup_temporary_path, &backup_path)?;

        // Unix rename replaces atomically. On Windows, the backup allows us to
        // restore the old primary if the second rename fails.
        #[cfg(not(windows))]
        fs::rename(&temporary_path, &final_path)?;
        #[cfg(windows)]
        {
            if final_path.exists() {
                fs::remove_file(&final_path)?;
            }
            if let Err(error) = fs::rename(&temporary_path, &final_path) {
                if backup_path.exists() {
                    let _ = fs::copy(&backup_path, &final_path);
                }
                return Err(error);
            }
        }
        sync_directory(directory)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
        let _ = fs::remove_file(backup_temporary_path);
    }
    result
}

#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
pub(super) fn sync_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(directory)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

#[cfg(test)]
mod cancellation_tests {
    use std::{
        cell::Cell,
        fs, io,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        CaptureKind, LIBRARY_INDEX_BACKUP_FILE, LIBRARY_INDEX_FILE, StoredCaptureMetadata,
        load_library_index, recover_library_index, recover_library_index_cancellable,
        save_library_index, system_time_nanos,
    };
    use crate::library::{create_patient, try_scan_library_directory};
    use crate::session::Eye;

    #[test]
    fn cancelled_legacy_migration_preserves_index_and_resumes_without_reassigning() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "iriscope-cancel-legacy-{}-{unique}",
            std::process::id()
        ));
        let patient = create_patient(&directory, "Jean", "Dupont").expect("patient");
        let mut index = load_library_index(&directory).expect("index");
        index.version = 1;
        for name in ["first.jpg", "second.jpg", "anonymous.jpg"] {
            let path = directory.join(name);
            fs::write(&path, b"historical capture bytes").expect("legacy capture");
            let metadata = fs::metadata(&path).expect("metadata");
            index.entries.insert(
                name.to_owned(),
                StoredCaptureMetadata {
                    patient_id: (name != "anonymous.jpg").then_some(patient.id),
                    first_name: Some("Jean".to_owned()),
                    last_name: Some("Dupont".to_owned()),
                    eye: Eye::Right,
                    kind: CaptureKind::Photo,
                    date_str: "2020-01-02".to_owned(),
                    time_str: "03:04:05".to_owned(),
                    file_size: Some(metadata.len()),
                    modified_nanos: metadata.modified().ok().and_then(system_time_nanos),
                    content_sha256: None,
                    file_version: None,
                },
            );
        }
        save_library_index(&directory, &index).expect("version one index");
        let primary_path = directory.join(LIBRARY_INDEX_FILE);
        let backup_path = directory.join(LIBRARY_INDEX_BACKUP_FILE);
        let primary_before = fs::read(&primary_path).expect("primary bytes");
        let backup_before = fs::read(&backup_path).expect("backup bytes");
        let checks = Cell::new(0);
        let error = recover_library_index_cancellable(&directory, &|| {
            let next = checks.get() + 1;
            checks.set(next);
            next >= 8
        })
        .expect_err("cancel the legacy SHA read");
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read(&primary_path).unwrap(), primary_before);
        assert_eq!(fs::read(&backup_path).unwrap(), backup_before);
        let unchanged = load_library_index(&directory).expect("preserved index");
        assert_eq!(unchanged.version, 1);
        assert!(
            unchanged
                .entries
                .values()
                .all(|entry| { entry.content_sha256.is_none() && entry.file_version.is_none() })
        );

        assert!(
            recover_library_index_cancellable(&directory, &|| false).expect("resume migration")
        );
        let entries = try_scan_library_directory(&directory).expect("verified captures");
        assert_eq!(entries.len(), 3);
        for entry in entries {
            let anonymous = entry.file_path.file_name().unwrap() == "anonymous.jpg";
            assert_eq!(entry.patient_id, (!anonymous).then_some(patient.id));
        }
        assert!(!recover_library_index(&directory).expect("idempotent recovery"));
        fs::remove_dir_all(directory).expect("cleanup");
    }
}
