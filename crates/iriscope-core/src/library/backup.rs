//! Complete, streamed snapshots with verified restores into a new directory.
use super::{LIBRARY_INDEX_LOCK_FILE, LIBRARY_INDEX_WRITE_LOCK, capture_transactions, index};
use crate::{disk_space::ensure_available_space, storage::create_private_directory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MANIFEST: &str = ".iriscope-backup.json";
const MAX_MANIFEST: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
#[derive(Serialize, Deserialize)]
struct BackupManifest {
    version: u32,
    files: Vec<BackupFile>,
}
#[derive(Serialize, Deserialize)]
struct BackupFile {
    name: String,
    bytes: u64,
    sha256: [u8; 32],
}
/// A published backup or restored library. Its directory is always newly created.
#[derive(Debug)]
pub struct BackupReport {
    /// Directory containing the complete snapshot or restored captures.
    pub directory: PathBuf,
    /// Number of copied files, including hidden index and recovery files.
    pub files: usize,
    /// Total media and metadata size.
    pub bytes: u64,
}
/// Current stage of a backup or restore. Completion means the directory was published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupPhase {
    /// Enumerating and checking the input before copying.
    Preparing,
    /// Streaming files and checking their content hashes.
    Copying,
    /// Checking that the source still matches the snapshot.
    Verifying,
    /// Writing metadata and publishing the complete directory.
    Finalizing,
    /// The complete result is available at its final path.
    Complete,
}
/// Progress includes streamed bytes, so a single large video also advances the UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupProgress {
    /// Current stage.
    pub phase: BackupPhase,
    /// Fully copied files.
    pub files_completed: usize,
    /// Number of files to copy.
    pub files_total: usize,
    /// Bytes copied, including the current file.
    pub bytes_completed: u64,
    /// Total number of bytes to copy.
    pub bytes_total: u64,
}
impl BackupProgress {
    fn preparing() -> Self {
        Self {
            phase: BackupPhase::Preparing,
            files_completed: 0,
            files_total: 0,
            bytes_completed: 0,
            bytes_total: 0,
        }
    }
}
fn cancelled(check: &dyn Fn() -> bool) -> io::Result<()> {
    if check() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Opération annulée.",
        ))
    } else {
        Ok(())
    }
}
fn collect_files(
    root: &Path,
    relative: &Path,
    files: &mut Vec<BackupFile>,
    cancel: &dyn Fn() -> bool,
) -> io::Result<()> {
    for entry in fs::read_dir(root.join(relative))? {
        cancelled(cancel)?;
        let entry = entry?;
        let path = relative.join(entry.file_name());
        let kind = entry.file_type()?;
        if relative.as_os_str().is_empty()
            && (entry.file_name() == LIBRARY_INDEX_LOCK_FILE
                || entry.file_name() == ".iriscope-publish.lock")
        {
            continue;
        }
        if kind.is_dir() {
            collect_files(root, &path, files, cancel)?;
        } else if kind.is_file() {
            let name = path
                .to_str()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Un nom de fichier n’est pas pris en charge.",
                    )
                })?
                .replace('\\', "/");
            safe_relative(&name)?;
            if name == MANIFEST {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Choisissez le dossier de captures, pas une sauvegarde.",
                ));
            }
            if files.len() >= MAX_FILES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "La sauvegarde dépasse 100 000 fichiers.",
                ));
            }
            files.push(BackupFile {
                name,
                bytes: entry.metadata()?.len(),
                sha256: [0; 32],
            });
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Le dossier contient un lien ou un fichier spécial. Sauvegarde interrompue sans modifier les captures.",
            ));
        }
    }
    Ok(())
}
fn safe_relative(name: &str) -> io::Result<PathBuf> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains(['\\', ':', '\0'])
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Chemin de sauvegarde invalide.",
        ));
    }
    Ok(path.to_path_buf())
}
fn checked_source(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component.as_os_str());
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Les liens ne sont pas acceptés dans une sauvegarde.",
            ));
        }
    }
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Fichier de sauvegarde absent ou invalide.",
        ));
    }
    Ok(path)
}
fn copy_verified(
    source: &Path,
    destination: &Path,
    expected: u64,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<[u8; 32]> {
    let before = crate::file_validation::capture_file_version_cancellable(source, cancel)?;
    let mut input = File::open(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(destination)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        cancelled(cancel)?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("Fichier trop grand."))?;
        if bytes > expected {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Un fichier a changé pendant la copie.",
            ));
        }
        output.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        progress(count as u64);
    }
    if bytes != expected
        || crate::file_validation::capture_file_version_cancellable(source, cancel)? != before
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Un fichier a changé pendant la copie. Réessayez après la fin des captures.",
        ));
    }
    if let Ok(modified) = input.metadata()?.modified() {
        output.set_times(fs::FileTimes::new().set_modified(modified))?;
    }
    output.sync_all()?;
    Ok(hash.finalize().into())
}
fn new_snapshot(parent: &Path, prefix: &str) -> io::Result<(PathBuf, PathBuf)> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let final_path = parent.join(format!("{prefix}-{stamp}-{}", std::process::id()));
    let staging = parent.join(format!(".{prefix}-{stamp}-{}.en-cours", std::process::id()));
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(&staging)?;
    Ok((staging, final_path))
}
fn publish(staging: &Path, destination: &Path) -> io::Result<()> {
    index::sync_directory(staging)?;
    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Le dossier de destination existe déjà.",
        ));
    }
    fs::rename(staging, destination)?;
    index::sync_directory(
        destination
            .parent()
            .ok_or_else(|| io::Error::other("Destination invalide."))?,
    )
}
fn write_manifest(staging: &Path, manifest: &BackupManifest) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(io::Error::other("Index de sauvegarde trop volumineux."));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(staging.join(MANIFEST))?;
    file.write_all(&bytes)?;
    file.sync_all()
}
fn verify_snapshot_source(
    source: &Path,
    manifest: &BackupManifest,
    versions: &[crate::file_validation::CaptureFileVersion],
    cancel: &dyn Fn() -> bool,
) -> io::Result<()> {
    let mut after = Vec::new();
    collect_files(source, Path::new(""), &mut after, cancel)?;
    after.sort_by(|a, b| a.name.cmp(&b.name));
    if after.len() != manifest.files.len()
        || after
            .iter()
            .zip(&manifest.files)
            .any(|(a, b)| a.name != b.name || a.bytes != b.bytes)
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Le dossier a changé pendant la sauvegarde. Réessayez après la fin des captures.",
        ));
    }
    for (entry, version) in manifest.files.iter().zip(versions) {
        if crate::file_validation::capture_file_version_cancellable(
            &source.join(&entry.name),
            cancel,
        )? != *version
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Un fichier a changé pendant la sauvegarde.",
            ));
        }
    }
    Ok(())
}
/// Saves the entire capture directory, with hidden metadata and unfinished videos.
/// Publication journals are replayed before the consistent snapshot is acquired.
/// # Errors
/// Rejects nested destinations, links, unstable files, and insufficient storage.
pub fn backup_library(
    source: &Path,
    parent: &Path,
    cancel: &dyn Fn() -> bool,
) -> io::Result<BackupReport> {
    backup_library_with_progress(source, parent, cancel, &mut |_| {})
}
/// Backs up the library with byte-level progress and cooperative cancellation.
/// # Errors
/// Has the same validation guarantees as [`backup_library`]. A cancellation before
/// publication removes the staging directory and leaves the captures unchanged.
pub fn backup_library_with_progress(
    source: &Path,
    parent: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(BackupProgress),
) -> io::Result<BackupReport> {
    cancelled(cancel)?;
    progress(BackupProgress::preparing());
    let source = fs::canonicalize(source)?;
    let parent = fs::canonicalize(parent)?;
    if parent.starts_with(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Placez la sauvegarde en dehors du dossier des captures.",
        ));
    }
    index::recover_library_index_cancellable(&source, cancel)?;
    let _publication = capture_transactions::lock_transaction_serial_cancellable(&source, cancel)?;
    let _guard = super::locks::mutex(&LIBRARY_INDEX_WRITE_LOCK, cancel)?;
    let _lock = index::lock_library_index_cancellable(&source, cancel)?;
    let mut library = index::load_library_index(&source)?;
    capture_transactions::recover_pending_locked(&source, &mut library)?;
    let mut manifest = BackupManifest {
        version: 1,
        files: Vec::new(),
    };
    collect_files(&source, Path::new(""), &mut manifest.files, cancel)?;
    manifest.files.sort_by(|a, b| a.name.cmp(&b.name));
    let mut names = HashSet::new();
    if manifest
        .files
        .iter()
        .any(|entry| !names.insert(entry.name.to_lowercase()))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Deux fichiers ont le même nom avec une casse différente. Renommez-les avant la sauvegarde.",
        ));
    }
    let total = manifest.files.iter().try_fold(0_u64, |sum, file| {
        sum.checked_add(file.bytes)
            .ok_or_else(|| io::Error::other("Sauvegarde trop volumineuse."))
    })?;
    ensure_available_space(&parent, total.saturating_add(MAX_MANIFEST))?;
    let versions: Vec<_> = manifest
        .files
        .iter()
        .map(|entry| {
            crate::file_validation::capture_file_version_cancellable(
                &source.join(&entry.name),
                cancel,
            )
        })
        .collect::<io::Result<_>>()?;
    let (staging, destination) = new_snapshot(&parent, "IrisScope-sauvegarde")?;
    let mut status = BackupProgress {
        phase: BackupPhase::Copying,
        files_total: manifest.files.len(),
        bytes_total: total,
        ..BackupProgress::preparing()
    };
    progress(status);
    let result = (|| {
        for entry in &mut manifest.files {
            let relative = safe_relative(&entry.name)?;
            let target = staging.join(&relative);
            create_private_directory(
                target
                    .parent()
                    .ok_or_else(|| io::Error::other("Chemin invalide."))?,
            )?;
            entry.sha256 = copy_verified(
                &checked_source(&source, &relative)?,
                &target,
                entry.bytes,
                cancel,
                &mut |bytes| {
                    status.bytes_completed += bytes;
                    progress(status);
                },
            )?;
            status.files_completed += 1;
            progress(status);
        }
        status.phase = BackupPhase::Verifying;
        progress(status);
        verify_snapshot_source(&source, &manifest, &versions, cancel)?;
        cancelled(cancel)?;
        status.phase = BackupPhase::Finalizing;
        progress(status);
        write_manifest(&staging, &manifest)?;
        cancelled(cancel)?;
        publish(&staging, &destination)?;
        status.phase = BackupPhase::Complete;
        progress(status);
        Ok(BackupReport {
            directory: destination,
            files: manifest.files.len(),
            bytes: total,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}
fn rebase_restored_versions(directory: &Path, manifest: &BackupManifest) -> io::Result<()> {
    let mut library = index::load_library_index(directory)?;
    let mut changed = false;
    for file in &manifest.files {
        if let Some(metadata) = library.entries.get_mut(&file.name)
            && metadata.content_sha256 == Some(file.sha256)
        {
            match crate::file_validation::capture_file_version_fast(&directory.join(&file.name)) {
                Ok(version) => {
                    metadata.file_version = Some(version);
                    changed = true;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Unsupported | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
    if changed {
        index::save_library_index(directory, &library)?;
    }
    Ok(())
}
/// Verifies every saved file and restores to a fresh directory. Existing data is untouched.
/// # Errors
/// Rejects corrupt, incomplete, unsafe or unsupported backups before publication.
pub fn restore_library(
    source: &Path,
    parent: &Path,
    cancel: &dyn Fn() -> bool,
) -> io::Result<BackupReport> {
    restore_library_with_progress(source, parent, cancel, &mut |_| {})
}
/// Restores a backup with byte-level progress into a newly published directory.
/// # Errors
/// Has the same guarantees as [`restore_library`]. Interrupted or invalid results
/// are never published, and an existing capture directory is never replaced.
pub fn restore_library_with_progress(
    source: &Path,
    parent: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(BackupProgress),
) -> io::Result<BackupReport> {
    cancelled(cancel)?;
    progress(BackupProgress::preparing());
    let source = fs::canonicalize(source)?;
    let parent = fs::canonicalize(parent)?;
    if parent.starts_with(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choisissez une destination hors de la sauvegarde.",
        ));
    }
    let manifest_path = checked_source(&source, Path::new(MANIFEST))?;
    let mut bytes = Vec::new();
    File::open(manifest_path)?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Index de sauvegarde trop volumineux.",
        ));
    }
    let manifest: BackupManifest = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if manifest.version != 1 || manifest.files.len() > MAX_FILES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Version de sauvegarde non prise en charge.",
        ));
    }
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for entry in &manifest.files {
        cancelled(cancel)?;
        let relative = safe_relative(&entry.name)?;
        if entry.name == MANIFEST || !names.insert(entry.name.to_lowercase()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Fichier réservé ou doublon dans la sauvegarde.",
            ));
        }
        let _ = checked_source(&source, &relative)?;
        total = total
            .checked_add(entry.bytes)
            .ok_or_else(|| io::Error::other("Sauvegarde trop volumineuse."))?;
    }
    ensure_available_space(&parent, total)?;
    let (staging, destination) = new_snapshot(&parent, "IrisScope-restauree")?;
    let mut status = BackupProgress {
        phase: BackupPhase::Copying,
        files_total: manifest.files.len(),
        bytes_total: total,
        ..BackupProgress::preparing()
    };
    progress(status);
    let result = (|| {
        for entry in &manifest.files {
            let relative = safe_relative(&entry.name)?;
            let target = staging.join(&relative);
            create_private_directory(
                target
                    .parent()
                    .ok_or_else(|| io::Error::other("Chemin invalide."))?,
            )?;
            if copy_verified(
                &checked_source(&source, &relative)?,
                &target,
                entry.bytes,
                cancel,
                &mut |bytes| {
                    status.bytes_completed += bytes;
                    progress(status);
                },
            )? != entry.sha256
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "La sauvegarde est altérée. Aucune capture actuelle n’a été remplacée.",
                ));
            }
            status.files_completed += 1;
            progress(status);
        }
        cancelled(cancel)?;
        status.phase = BackupPhase::Finalizing;
        progress(status);
        // The copied bytes have just been SHA-verified. Retain that proof with
        // the new native file versions, avoiding another full-media migration.
        rebase_restored_versions(&staging, &manifest)?;
        cancelled(cancel)?;
        publish(&staging, &destination)?;
        status.phase = BackupPhase::Complete;
        progress(status);
        Ok(BackupReport {
            directory: destination,
            files: manifest.files.len(),
            bytes: total,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        library::{
            CaptureKind, create_patient, get_patient, save_indexed_capture,
            try_scan_library_directory,
        },
        session::{CaptureSession, Eye},
        storage::CaptureTimestamp,
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "iriscope-backup-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(root.join("captures")).unwrap();
            Self(root)
        }
        fn source(&self) -> PathBuf {
            self.0.join("captures")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn round_trip_preserves_dossier_links_hidden_files_and_partial_recordings() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let patient = create_patient(&source, "Émilie", "Martin").unwrap();
        let mut session = CaptureSession::new("Émilie", "Martin", Eye::Left);
        session.set_patient_id(Some(patient.id));
        save_indexed_capture(
            &source,
            "capture.jpg",
            b"original capture",
            &session,
            CaptureKind::Photo,
            CaptureTimestamp {
                year: 2026,
                month: 10,
                day: 5,
                hour: 12,
                minute: 0,
                second: 0,
            },
        )
        .unwrap();
        fs::write(source.join(".iriscope-recording.part"), b"partial video").unwrap();
        let snapshot = backup_library(&source, &fixture.0, &|| false).unwrap();
        assert!(snapshot.directory.join(".iriscope-index.json").exists());
        let restored = restore_library(&snapshot.directory, &fixture.0, &|| false).unwrap();
        assert_eq!(
            fs::read(restored.directory.join(".iriscope-recording.part")).unwrap(),
            b"partial video"
        );
        assert_eq!(
            get_patient(&restored.directory, patient.id)
                .unwrap()
                .unwrap()
                .first_name,
            "Émilie"
        );
        let entries = try_scan_library_directory(&restored.directory).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].patient_id, Some(patient.id));
        assert_eq!(entries[0].eye, Eye::Left);
        assert_eq!(
            fs::read(source.join("capture.jpg")).unwrap(),
            b"original capture"
        );
    }
    #[test]
    fn corrupt_backup_is_never_published_and_current_captures_are_preserved() {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("capture.jpg"), b"original").unwrap();
        let snapshot = backup_library(&fixture.source(), &fixture.0, &|| false).unwrap();
        fs::write(snapshot.directory.join("capture.jpg"), b"modified").unwrap();
        assert!(restore_library(&snapshot.directory, &fixture.0, &|| false).is_err());
        assert_eq!(
            fs::read(fixture.source().join("capture.jpg")).unwrap(),
            b"original"
        );
        assert!(fs::read_dir(&fixture.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("restauree")
        }));
    }
    #[test]
    fn rejects_traversal_future_versions_nested_destinations_and_cancellation() {
        let fixture = Fixture::new();
        fs::write(fixture.source().join("capture.jpg"), b"original").unwrap();
        assert!(backup_library(&fixture.source(), &fixture.source(), &|| false).is_err());
        assert_eq!(
            backup_library(&fixture.source(), &fixture.0, &|| true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        let snapshot = backup_library(&fixture.source(), &fixture.0, &|| false).unwrap();
        let mut manifest: BackupManifest =
            serde_json::from_slice(&fs::read(snapshot.directory.join(MANIFEST)).unwrap()).unwrap();
        manifest.files[0].name = "../outside.jpg".into();
        fs::write(
            snapshot.directory.join(MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(restore_library(&snapshot.directory, &fixture.0, &|| false).is_err());
        manifest.version = 99;
        fs::write(
            snapshot.directory.join(MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(restore_library(&snapshot.directory, &fixture.0, &|| false).is_err());
        assert!(!fixture.0.join("outside.jpg").exists());
    }
    #[test]
    fn progress_tracks_large_files_and_only_completes_after_publication() {
        let fixture = Fixture::new();
        let source = fixture.source();
        fs::write(source.join("large.avi"), vec![71_u8; 3 * 1024 * 1024]).unwrap();
        let mut updates = Vec::new();
        let report = backup_library_with_progress(&source, &fixture.0, &|| false, &mut |value| {
            updates.push(value);
        })
        .unwrap();
        assert_eq!(updates.first().unwrap().phase, BackupPhase::Preparing);
        assert!(
            updates
                .iter()
                .any(|value| value.phase == BackupPhase::Copying
                    && value.bytes_completed > 0
                    && value.files_completed < value.files_total)
        );
        assert!(
            updates
                .windows(2)
                .all(|pair| pair[0].bytes_completed <= pair[1].bytes_completed)
        );
        let complete = updates.last().unwrap();
        assert_eq!(complete.phase, BackupPhase::Complete);
        assert_eq!(
            (complete.bytes_completed, complete.files_completed),
            (report.bytes, report.files)
        );
        assert!(report.directory.join(MANIFEST).is_file());
    }
    #[test]
    fn cancellation_during_copy_or_finalization_cleans_only_the_staging_directory() {
        use std::cell::Cell;
        for phase in [BackupPhase::Copying, BackupPhase::Finalizing] {
            let fixture = Fixture::new();
            let original = vec![83_u8; 3 * 1024 * 1024];
            fs::write(fixture.source().join("large.avi"), &original).unwrap();
            let cancel = Cell::new(false);
            let result = backup_library_with_progress(
                &fixture.source(),
                &fixture.0,
                &|| cancel.get(),
                &mut |value| {
                    if value.phase == phase
                        && (phase != BackupPhase::Copying || value.bytes_completed > 0)
                    {
                        cancel.set(true);
                    }
                },
            );
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
            assert_eq!(
                fs::read(fixture.source().join("large.avi")).unwrap(),
                original
            );
            assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
        }
    }
    #[test]
    fn cancelled_restore_never_publishes_and_a_later_retry_succeeds() {
        use std::cell::Cell;
        let fixture = Fixture::new();
        let original = vec![17_u8; 3 * 1024 * 1024];
        fs::write(fixture.source().join("large.avi"), &original).unwrap();
        let backup = backup_library(&fixture.source(), &fixture.0, &|| false).unwrap();
        let cancel = Cell::new(false);
        let result = restore_library_with_progress(
            &backup.directory,
            &fixture.0,
            &|| cancel.get(),
            &mut |value| {
                if value.bytes_completed > 0 {
                    cancel.set(true);
                }
            },
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
        let restored = restore_library(&backup.directory, &fixture.0, &|| false).unwrap();
        assert_eq!(
            fs::read(restored.directory.join("large.avi")).unwrap(),
            original
        );
        assert_eq!(
            fs::read(fixture.source().join("large.avi")).unwrap(),
            original
        );
    }
    #[test]
    fn source_disappearance_is_reported_without_publishing_a_partial_backup() {
        let fixture = Fixture::new();
        let source = fixture.source().join("large.avi");
        fs::write(&source, vec![11_u8; 3 * 1024 * 1024]).unwrap();
        let mut removed = false;
        let result =
            backup_library_with_progress(&fixture.source(), &fixture.0, &|| false, &mut |value| {
                if value.bytes_completed > 0 && !removed {
                    fs::remove_file(&source).unwrap();
                    removed = true;
                }
            });
        assert!(result.is_err());
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    }
    #[test]
    fn waiting_for_another_process_lock_can_be_cancelled() {
        let fixture = Fixture::new();
        let path = fixture.source().join(".iriscope-publish.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        file.lock().unwrap();
        let started = std::time::Instant::now();
        let result = backup_library(&fixture.source(), &fixture.0, &|| {
            started.elapsed() >= std::time::Duration::from_millis(100)
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_instead_of_silently_omitting_files() {
        let fixture = Fixture::new();
        std::os::unix::fs::symlink("/etc/passwd", fixture.source().join("outside.jpg")).unwrap();
        assert!(backup_library(&fixture.source(), &fixture.0, &|| false).is_err());
    }
}
