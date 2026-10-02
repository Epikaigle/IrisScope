//! Durable capture publication, with the dossier link prepared before media bytes.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, atomic::Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use super::CaptureKind;
use super::{
    CaptureFileVersion, LIBRARY_INDEX_WRITE_LOCK, LibraryIndex, NEXT_INDEX_WRITE_ID,
    StoredCaptureMetadata, capture_file_version, capture_file_version_fast,
    capture_file_version_fast_from_file, capture_file_version_from_file,
    ensure_local_regular_capture, load_library_index, load_library_index_for_write,
    lock_library_index, normalized_patient_name, recover_library_index_locked, save_library_index,
    sync_directory,
};
use crate::{
    file_validation::{capture_fingerprint, system_time_nanos},
    session::CaptureSession,
    storage::{
        CaptureTimestamp, collision_name, create_private_directory, validate_capture_file_name,
    },
};

const JOURNAL_PREFIX: &str = ".iriscope-pending-capture-";
const TRANSACTION_LOCK_FILE: &str = ".iriscope-publish.lock";
const DESTINATION_STABILITY_TIMEOUT: Duration = Duration::from_millis(1_250);
// A small photo can be rehashed safely during the index commit without paying
// the one-second stability delay. Large media is validated outside that lock.
const MAX_INLINE_VALIDATION_BYTES: u64 = 1024 * 1024;
static TRANSACTION_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct TransactionGuard {
    _thread: MutexGuard<'static, ()>,
    _file: File,
}

pub(super) fn lock_transaction_serial(directory: &Path) -> io::Result<TransactionGuard> {
    let thread = TRANSACTION_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(directory.join(TRANSACTION_LOCK_FILE))?;
    file.lock()?;
    Ok(TransactionGuard {
        _thread: thread,
        _file: file,
    })
}

/// Result of publishing a capture. The media exists even if its index update
/// needs to be retried; a durable journal retains its explicitly selected dossier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureCommit {
    /// Published media file.
    pub file_path: PathBuf,
    /// Version of the same final file handle that received the published bytes.
    pub file_version: CaptureFileVersion,
    /// Failure to update the index. Recovery retries this update automatically.
    pub metadata_warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingCapture {
    version: u32,
    staging_file: String,
    destination_file: String,
    staging_identity: FileIdentity,
    destination_identity: FileIdentity,
    staging_size: u64,
    staging_modified_nanos: Option<u128>,
    metadata: StoredCaptureMetadata,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileIdentity {
    #[serde(default)]
    device: Option<u64>,
    #[serde(default)]
    inode: Option<u64>,
    created_nanos: Option<u128>,
    #[serde(default)]
    file_id: Option<u128>,
}

/// A file created by this transaction. Preparation failures clean up only the
/// same file, never a replacement that appeared at its path in the meantime.
struct PreparedFile {
    path: PathBuf,
    identity: FileIdentity,
    file: Option<File>,
    preserve: bool,
}

impl PreparedFile {
    fn create(path: PathBuf) -> io::Result<Self> {
        let file = private_new_file(&path)?;
        Self::from_created_file(path, file)
    }

    fn from_created_file(path: PathBuf, file: File) -> io::Result<Self> {
        // If identity cannot be queried, preserve the newly created path rather
        // than risk removing a replacement during preparation cleanup.
        let identity = FileIdentity::of(&file)?;
        Ok(Self {
            path,
            identity,
            file: Some(file),
            preserve: false,
        })
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("prepared file is still open")
    }

    fn close(&mut self) {
        drop(self.file.take());
    }
}

impl Drop for PreparedFile {
    fn drop(&mut self) {
        self.close();
        if !self.preserve
            && fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
                metadata.file_type().is_file() && self.identity.matches_path(&self.path)
            })
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl FileIdentity {
    fn of(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: Some(metadata.dev()),
                inode: Some(metadata.ino()),
                created_nanos: metadata.created().ok().and_then(system_time_nanos),
                file_id: None,
            })
        }
        #[cfg(target_os = "windows")]
        {
            let (device, file_id) =
                crate::file_validation::capture_native_file_identity_from_file(file)?;
            Ok(Self {
                device: Some(device),
                inode: None,
                created_nanos: metadata.created().ok().and_then(system_time_nanos),
                file_id: Some(file_id),
            })
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            Ok(Self {
                device: None,
                inode: None,
                created_nanos: metadata.created().ok().and_then(system_time_nanos),
                file_id: None,
            })
        }
    }

    fn matches_path(&self, path: &Path) -> bool {
        File::open(path).is_ok_and(|file| self.matches(&file))
    }

    fn matches(&self, file: &File) -> bool {
        let Ok(actual) = Self::of(file) else {
            return false;
        };
        #[cfg(unix)]
        {
            self.device == actual.device
                && self.inode == actual.inode
                && self
                    .created_nanos
                    .is_none_or(|created| Some(created) == actual.created_nanos)
        }
        #[cfg(target_os = "windows")]
        {
            // Creation times can be reused after a rename. Legacy journals
            // without native identity remain preserved for explicit recovery.
            self.device.is_some()
                && self.file_id.is_some()
                && self.device == actual.device
                && self.file_id == actual.file_id
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            self.created_nanos.is_some() && self.created_nanos == actual.created_nanos
        }
    }
}

/// Saves media and its dossier association as a recoverable transaction.
/// The capture is never associated by guessing its name or matching homonyms.
///
/// # Errors
///
/// Returns an error before publication if preparation or media writing fails.
/// After media publication, index errors are returned in `metadata_warning`.
pub fn save_indexed_capture(
    directory: &Path,
    file_name: &str,
    data: &[u8],
    session: &CaptureSession,
    kind: CaptureKind,
    timestamp: CaptureTimestamp,
) -> io::Result<CaptureCommit> {
    validate_capture_file_name(file_name)?;
    create_private_directory(directory)?;
    let mut staging = prepare_staged_file(directory, |file| {
        file.write_all(data)?;
        file.sync_all()
    })?;
    commit_staged_capture(directory, file_name, &mut staging, session, kind, timestamp)
}

/// Publishes a completed recording and records its explicit dossier association.
/// A private hard link, or streaming copy when unavailable, keeps the completed
/// source recoverable until the index is durably written. No full video is buffered.
/// The original source is removed only after the complete media is published.
///
/// # Errors
///
/// Returns an error for invalid input, preparation or media publication failures.
pub fn publish_indexed_capture(
    source: &Path,
    directory: &Path,
    file_name: &str,
    session: &CaptureSession,
    kind: CaptureKind,
    timestamp: CaptureTimestamp,
) -> io::Result<CaptureCommit> {
    validate_capture_file_name(file_name)?;
    if !fs::symlink_metadata(source)?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture source is not a regular file",
        ));
    }
    create_private_directory(directory)?;
    let mut staging = prepare_staged_source(source, directory)?;
    let result =
        commit_staged_capture(directory, file_name, &mut staging, session, kind, timestamp)?;
    let _ = fs::remove_file(source);
    Ok(result)
}

fn prepare_staged_file(
    directory: &Path,
    prepare: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<PreparedFile> {
    let mut staging = loop {
        match PreparedFile::create(new_staging_path(directory)) {
            Ok(staging) => break staging,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    };
    prepare(staging.file_mut())?;
    staging.close();
    Ok(staging)
}

fn prepare_staged_source(source: &Path, directory: &Path) -> io::Result<PreparedFile> {
    loop {
        let path = new_staging_path(directory);
        match fs::hard_link(source, &path) {
            Ok(()) => {
                // Open the exact new link before handing it to the cleanup guard.
                let file = match OpenOptions::new().read(true).write(true).open(&path) {
                    Ok(file) => file,
                    Err(error) => {
                        let _ = fs::remove_file(path);
                        return Err(error);
                    }
                };
                let mut staging = PreparedFile::from_created_file(path, file)?;
                staging.file_mut().sync_all()?;
                staging.close();
                return Ok(staging);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return prepare_staged_file(directory, |output| {
                    let mut input = File::open(source)?;
                    io::copy(&mut input, output)?;
                    output.sync_all()
                });
            }
        }
    }
}

fn new_staging_path(directory: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = NEXT_INDEX_WRITE_ID.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        "{JOURNAL_PREFIX}{}-{nanos}-{id}.data",
        std::process::id()
    ))
}

fn private_new_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

fn commit_staged_capture(
    directory: &Path,
    file_name: &str,
    staging: &mut PreparedFile,
    session: &CaptureSession,
    kind: CaptureKind,
    timestamp: CaptureTimestamp,
) -> io::Result<CaptureCommit> {
    validate_capture_file_name(file_name)?;
    let (stage_version, digest) = capture_fingerprint(&staging.path)?;
    let _publication_guard = lock_transaction_serial(directory)?;
    let write_guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let file_lock = lock_library_index(directory)?;
    let mut index = load_library_index_for_write(directory)?;
    recover_pending_locked(directory, &mut index)?;
    let stage_metadata = fs::metadata(&staging.path)?;
    if !stage_metadata.file_type().is_file() || !staging.identity.matches_path(&staging.path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prepared capture was replaced before publication",
        ));
    }
    let mut metadata = capture_metadata(session, kind, timestamp, &index, &stage_metadata)?;
    metadata.file_version = Some(stage_version);
    metadata.content_sha256 = Some(digest);
    let requested = Path::new(file_name);
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Iris");
    let extension = requested.extension().and_then(|value| value.to_str());
    let mut destination = (1_u32..=u32::MAX)
        .find_map(|number| {
            let name = match collision_name(stem, extension, file_name, number) {
                Ok(name) => name,
                Err(error) => return Some(Err(error)),
            };
            let path = directory.join(name);
            match PreparedFile::create(path) {
                Ok(file) => Some(Ok(file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "capture filename suffix exhausted",
            ))
        })?;
    destination.file_mut().sync_all()?;
    let pending = PendingCapture {
        version: 1,
        staging_file: utf8_file_name(&staging.path)?,
        destination_file: utf8_file_name(&destination.path)?,
        staging_identity: staging.identity.clone(),
        destination_identity: destination.identity.clone(),
        staging_size: stage_metadata.len(),
        staging_modified_nanos: stage_metadata.modified().ok().and_then(system_time_nanos),
        metadata,
    };
    let journal = staging.path.with_extension("json");
    let bytes = serde_json::to_vec(&pending).map_err(io::Error::other)?;
    install_capture_journal(&journal, &bytes)?;
    // Once installed, even an uncertain directory sync must retain every file
    // needed for replay: deleting it could leave a surviving journal incomplete.
    staging.preserve = true;
    destination.preserve = true;
    sync_directory(directory)?;

    // The separate publication lock keeps recovery away from this journal.
    // Dossier/index writers may proceed while a long recording is copied.
    drop(index);
    drop(file_lock);
    drop(write_guard);

    // The journal is durable before the first media byte reaches its visible name.
    let mut input = File::open(&staging.path)?;
    io::copy(&mut input, destination.file_mut())?;
    destination.file_mut().sync_all()?;
    // A recent same-size edit can keep the kernel's change time in the same
    // clock tick. Wait outside the index lock, then establish a fresh SHA
    // baseline. On weak filesystems, the commit falls back to a second SHA
    // while holding the index lock.
    let stable_before_hash = wait_for_destination_stability(destination.file_mut())?;
    let published_version = capture_file_version_from_file(destination.file_mut())?;
    let stable_baseline = stable_before_hash
        && capture_file_version_fast_from_file(destination.file_mut())
            .is_ok_and(|native| native.same_native_metadata(&published_version))
        && capture_file_version_fast(&destination.path)
            .is_ok_and(|native| native.same_native_metadata(&published_version));
    destination.close();
    let result = (|| {
        let _write_guard = LIBRARY_INDEX_WRITE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _file_lock = lock_library_index(directory)?;
        let mut index = load_library_index_for_write(directory)?;
        commit_pending_metadata(
            directory,
            &pending,
            &mut index,
            Some((&published_version, stable_baseline)),
        )
    })();
    let metadata_warning = match result {
        Ok(()) => {
            cleanup_pending(directory, &journal, &staging.path);
            None
        }
        Err(error) => Some(error.to_string()),
    };
    Ok(CaptureCommit {
        file_path: destination.path.clone(),
        file_version: published_version,
        metadata_warning,
    })
}

fn wait_for_destination_stability(file: &File) -> io::Result<bool> {
    if file.metadata()?.len() <= MAX_INLINE_VALIDATION_BYTES {
        // `false` requires a full content check at each commit guard; fresh
        // metadata is never treated as proof that these bytes are unchanged.
        return Ok(false);
    }
    let deadline = Instant::now() + DESTINATION_STABILITY_TIMEOUT;
    loop {
        match capture_file_version_fast_from_file(file) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(false);
                }
                thread::sleep(remaining.min(Duration::from_millis(20)));
            }
            Err(error) => return Err(error),
        }
    }
}

fn install_capture_journal(journal: &Path, bytes: &[u8]) -> io::Result<()> {
    // Library writers hold the process and directory locks here. Refuse an
    // existing journal instead of replacing another transaction's dossier.
    require_absent(journal)?;
    let mut temporary = PreparedFile::create(journal.with_extension("json.tmp"))?;
    temporary.file_mut().write_all(bytes)?;
    temporary.file_mut().sync_all()?;
    temporary.close();
    require_absent(journal)?;
    fs::rename(&temporary.path, journal)
}

fn require_absent(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "capture journal already exists",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn utf8_file_name(path: &Path) -> io::Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "capture filename is not UTF-8"))
}

fn capture_metadata(
    session: &CaptureSession,
    kind: CaptureKind,
    timestamp: CaptureTimestamp,
    index: &LibraryIndex,
    file: &fs::Metadata,
) -> io::Result<StoredCaptureMetadata> {
    let metadata = StoredCaptureMetadata {
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
        file_size: Some(file.len()),
        modified_nanos: None,
        content_sha256: None,
        file_version: None,
    };
    validate_dossier(&metadata, index)?;
    Ok(metadata)
}

fn validate_dossier(metadata: &StoredCaptureMetadata, index: &LibraryIndex) -> io::Result<()> {
    if metadata.patient_id.is_none()
        && (metadata.first_name.is_some() || metadata.last_name.is_some())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "named capture requires an explicitly selected dossier",
        ));
    }
    if let Some(id) = metadata.patient_id {
        let patient = index.patients.get(&id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "selected dossier does not exist",
            )
        })?;
        if normalized_patient_name(&patient.first_name)
            != normalized_patient_name(metadata.first_name.as_deref().unwrap_or(""))
            || normalized_patient_name(&patient.last_name)
                != normalized_patient_name(metadata.last_name.as_deref().unwrap_or(""))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "selected dossier does not match capture names",
            ));
        }
    }
    Ok(())
}

fn commit_pending_metadata(
    directory: &Path,
    pending: &PendingCapture,
    index: &mut LibraryIndex,
    verified_version: Option<(&CaptureFileVersion, bool)>,
) -> io::Result<()> {
    validate_dossier(&pending.metadata, index)?;
    if index
        .entries
        .get(&pending.destination_file)
        .is_some_and(|existing| existing.patient_id != pending.metadata.patient_id)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "capture index already contains another dossier; journal preserved",
        ));
    }
    let path = directory.join(&pending.destination_file);
    ensure_local_regular_capture(directory, &path)?;
    let file_metadata = fs::metadata(&path)?;
    if !pending.destination_identity.matches_path(&path)
        || file_metadata.len() != pending.staging_size
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published capture was replaced or is incomplete; dossier journal preserved",
        ));
    }
    let mut metadata = pending.metadata.clone();
    let file_version = if let Some((verified, stable_baseline)) = verified_version {
        if !published_version_matches(&path, verified, stable_baseline)? {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "published capture changed after content validation",
            ));
        }
        verified.clone()
    } else {
        capture_file_version(&path)?
    };
    let content_sha256 = file_version.content_digest().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "capture SHA version is incomplete",
        )
    })?;
    if metadata
        .content_sha256
        .is_some_and(|expected| expected != content_sha256)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published media bytes differ from the dossier journal; journal preserved",
        ));
    }
    metadata.file_size = Some(file_metadata.len());
    metadata.modified_nanos = file_metadata.modified().ok().and_then(system_time_nanos);
    metadata.content_sha256 = Some(content_sha256);
    metadata.file_version = Some(file_version.clone());
    if let Some(patient_id) = metadata.patient_id {
        let capture_at = format!("{} {}", metadata.date_str, metadata.time_str);
        let patient = index
            .patients
            .get_mut(&patient_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "dossier does not exist"))?;
        if patient
            .last_capture
            .as_ref()
            .is_none_or(|last| last < &capture_at)
        {
            patient.last_capture = Some(capture_at);
        }
    }
    index.version = 2;
    index
        .entries
        .insert(pending.destination_file.clone(), metadata);
    if !published_version_matches(
        &path,
        &file_version,
        verified_version.is_some_and(|(_, stable)| stable),
    )? {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "published capture changed before metadata commit",
        ));
    }
    save_library_index(directory, index)
}

fn published_version_matches(
    path: &Path,
    expected: &CaptureFileVersion,
    stable_baseline: bool,
) -> io::Result<bool> {
    if stable_baseline {
        match capture_file_version_fast(path) {
            Ok(native) => return Ok(native.same_native_metadata(expected)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Unsupported
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(capture_file_version(path)? == *expected)
}

/// Replays interrupted capture transactions without guessing any patient link.
/// Replaced files or staging files are preserved and cause a visible error.
///
/// # Errors
///
/// Returns an error if a journal cannot be trusted or the index cannot be saved.
pub fn recover_pending_capture_metadata(directory: &Path) -> io::Result<usize> {
    if !directory.exists() {
        return Ok(0);
    }
    let _publication_guard = lock_transaction_serial(directory)?;
    let _write_guard = LIBRARY_INDEX_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _file_lock = lock_library_index(directory)?;
    recover_library_index_locked(directory)?;
    let mut index = load_library_index(directory)?;
    recover_pending_locked(directory, &mut index)
}

// These are private protocol filenames emitted with an exact lowercase suffix.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub(super) fn recover_pending_locked(
    directory: &Path,
    index: &mut LibraryIndex,
) -> io::Result<usize> {
    let entries = fs::read_dir(directory)?.collect::<io::Result<Vec<_>>>()?;
    let mut journals: Vec<_> = entries
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(JOURNAL_PREFIX) && name.ends_with(".json"))
        })
        .collect();
    journals.sort();
    let mut recovered = 0;
    for journal in journals {
        ensure_local_regular_capture(directory, &journal)?;
        let mut data = Vec::new();
        File::open(&journal)?
            .take(64 * 1024 + 1)
            .read_to_end(&mut data)?;
        if data.len() > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "capture journal exceeds its size limit",
            ));
        }
        let pending: PendingCapture = serde_json::from_slice(&data)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if pending.version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported capture journal version",
            ));
        }
        validate_capture_file_name(&pending.staging_file)?;
        validate_capture_file_name(&pending.destination_file)?;
        let staging = directory.join(&pending.staging_file);
        let destination = directory.join(&pending.destination_file);
        ensure_local_regular_capture(directory, &staging)?;
        ensure_local_regular_capture(directory, &destination)?;
        let stage_metadata = fs::metadata(&staging)?;
        let destination_metadata = fs::metadata(&destination)?;
        if !pending.staging_identity.matches_path(&staging)
            || stage_metadata.len() != pending.staging_size
            || stage_metadata.modified().ok().and_then(system_time_nanos)
                != pending.staging_modified_nanos
            || !pending.destination_identity.matches_path(&destination)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "capture transaction files were replaced; dossier journal preserved",
            ));
        }
        validate_dossier(&pending.metadata, index)?;
        // Complete a partial copy only into the exact file reserved by this transaction.
        if !same_contents(&staging, &destination)? {
            if destination_metadata.len() >= stage_metadata.len()
                || !is_staging_prefix(&staging, &destination)?
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "capture transaction destination was edited; original media and dossier journal preserved",
                ));
            }
            let mut output = OpenOptions::new().write(true).open(&destination)?;
            if !pending.destination_identity.matches(&output) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "capture destination changed during recovery",
                ));
            }
            output.set_len(0)?;
            let mut input = File::open(&staging)?;
            io::copy(&mut input, &mut output)?;
            output.sync_all()?;
        }
        commit_pending_metadata(directory, &pending, index, None)?;
        cleanup_pending(directory, &journal, &staging);
        recovered += 1;
    }
    Ok(recovered)
}

pub(super) fn pending_destination_names(directory: &Path) -> io::Result<HashSet<String>> {
    pending_destination_names_cancellable(directory, &|| false)
}

// These are private protocol filenames emitted with an exact lowercase suffix.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub(super) fn pending_destination_names_cancellable(
    directory: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<HashSet<String>> {
    let mut names = HashSet::new();
    for entry in fs::read_dir(directory)? {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "library scan cancelled",
            ));
        }
        let entry = entry?;
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(JOURNAL_PREFIX) && name.ends_with(".json"))
        {
            continue;
        }
        ensure_local_regular_capture(directory, &path)?;
        let mut data = Vec::new();
        File::open(path)?
            .take(64 * 1024 + 1)
            .read_to_end(&mut data)?;
        if data.len() > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "capture journal exceeds its size limit",
            ));
        }
        let pending: PendingCapture = serde_json::from_slice(&data)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        validate_capture_file_name(&pending.destination_file)?;
        names.insert(pending.destination_file);
    }
    Ok(names)
}

fn same_contents(first: &Path, second: &Path) -> io::Result<bool> {
    let mut first = File::open(first)?;
    let mut second = File::open(second)?;
    if first.metadata()?.len() != second.metadata()?.len() {
        return Ok(false);
    }
    let mut left = [0_u8; 8 * 1024];
    let mut right = [0_u8; 8 * 1024];
    loop {
        let count = first.read(&mut left)?;
        second.read_exact(&mut right[..count])?;
        if left[..count] != right[..count] {
            return Ok(false);
        }
        if count == 0 {
            return Ok(true);
        }
    }
}

fn cleanup_pending(directory: &Path, journal: &Path, staging: &Path) {
    // Remove the journal first: a remaining staging file is harmless, whereas a
    // surviving journal without its source would prevent safe recovery.
    if fs::remove_file(journal).is_ok() && sync_directory(directory).is_ok() {
        let _ = fs::remove_file(staging);
    }
}

fn is_staging_prefix(staging: &Path, destination: &Path) -> io::Result<bool> {
    let mut staging = File::open(staging)?;
    let mut destination = File::open(destination)?;
    let mut left = [0_u8; 8 * 1024];
    let mut right = [0_u8; 8 * 1024];
    loop {
        let count = destination.read(&mut right)?;
        staging.read_exact(&mut left[..count])?;
        if left[..count] != right[..count] {
            return Ok(false);
        }
        if count == 0 {
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        library::{
            LIBRARY_INDEX_BACKUP_FILE, assign_capture_to_patient, create_patient,
            record_capture_metadata, resolve_library_candidates_cancellable,
            try_scan_library_directory, try_scan_library_directory_metadata,
        },
        session::Eye,
    };

    fn test_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope-transaction-{label}-{unique}"));
        fs::create_dir_all(&path).expect("create directory");
        path
    }

    fn selected_session(directory: &Path) -> CaptureSession {
        let first = create_patient(directory, "Jean", "Dupont").expect("first homonym");
        let selected = create_patient(directory, "Jean", "Dupont").expect("second homonym");
        assert_ne!(first.id, selected.id);
        let mut session = CaptureSession::new("Jean", "Dupont", Eye::Left);
        session.set_patient_id(Some(selected.id));
        session
    }

    fn pending_fixture(
        directory: &Path,
        session: &CaptureSession,
        complete: bool,
    ) -> (PathBuf, PathBuf) {
        let index = load_library_index(directory).expect("index");
        let staging = new_staging_path(directory);
        fs::write(&staging, b"complete original media bytes").expect("staging");
        let stage_metadata = fs::metadata(&staging).expect("metadata");
        let destination = directory.join("custom.jpg");
        fs::write(
            &destination,
            if complete {
                b"complete original media bytes".as_slice()
            } else {
                b"complete original".as_slice()
            },
        )
        .expect("interrupted copy");
        let pending = PendingCapture {
            version: 1,
            staging_file: utf8_file_name(&staging).expect("name"),
            destination_file: utf8_file_name(&destination).expect("name"),
            staging_identity: FileIdentity::of(&File::open(&staging).expect("staging file"))
                .expect("staging identity"),
            destination_identity: FileIdentity::of(
                &File::open(&destination).expect("destination file"),
            )
            .expect("destination identity"),
            staging_size: stage_metadata.len(),
            staging_modified_nanos: stage_metadata.modified().ok().and_then(system_time_nanos),
            metadata: capture_metadata(
                session,
                CaptureKind::Photo,
                CaptureTimestamp::now(),
                &index,
                &stage_metadata,
            )
            .expect("explicit dossier"),
        };
        let journal = staging.with_extension("json");
        fs::write(&journal, serde_json::to_vec(&pending).expect("serialize")).expect("journal");
        (journal, destination)
    }

    #[test]
    fn transaction_preserves_existing_capture_and_explicit_homonym() {
        let directory = test_directory("collision");
        let session = selected_session(&directory);
        fs::write(directory.join("custom.jpg"), b"existing media").expect("existing");
        let result = save_indexed_capture(
            &directory,
            "custom.jpg",
            b"new media",
            &session,
            CaptureKind::Photo,
            CaptureTimestamp::now(),
        )
        .expect("save");
        assert_eq!(
            result.file_path.file_name().and_then(|name| name.to_str()),
            Some("custom_2.jpg")
        );
        assert_eq!(result.metadata_warning, None);
        assert_eq!(
            fs::read(directory.join("custom.jpg")).expect("existing bytes"),
            b"existing media"
        );
        let entries = try_scan_library_directory(&directory).expect("scan");
        let capture = entries
            .iter()
            .find(|entry| entry.file_path == result.file_path)
            .expect("new media");
        assert_eq!(capture.patient_id, session.patient_id());
        assert_eq!(
            recover_pending_capture_metadata(&directory).expect("no pending"),
            0
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn replay_finishes_interrupted_copy_and_keeps_explicit_dossier() {
        let directory = test_directory("interrupted");
        let session = selected_session(&directory);
        let (journal, destination) = pending_fixture(&directory, &session, false);
        assert_eq!(
            recover_pending_capture_metadata(&directory).expect("replay"),
            1
        );
        assert_eq!(
            fs::read(destination).expect("complete bytes"),
            b"complete original media bytes"
        );
        assert_eq!(
            try_scan_library_directory(&directory).expect("scan")[0].patient_id,
            session.patient_id()
        );
        assert!(!journal.exists());
        assert_eq!(
            recover_pending_capture_metadata(&directory).expect("idempotent replay"),
            0
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn pending_destination_cannot_be_indexed_or_resolved_by_other_writers() {
        let directory = test_directory("pending-guards");
        let session = selected_session(&directory);
        let destination = directory.join("custom.jpg");
        fs::write(&destination, b"complete original media bytes").expect("prior candidate");
        let candidates = try_scan_library_directory_metadata(&directory).expect("candidate");
        assert_eq!(candidates.len(), 1);
        let (_, destination) = pending_fixture(&directory, &session, false);
        let metadata_result = record_capture_metadata(
            &directory,
            &destination,
            &session,
            CaptureKind::Photo,
            CaptureTimestamp::now(),
        );
        assert!(matches!(metadata_result, Err(error) if error.kind() == io::ErrorKind::WouldBlock));
        let assign_result = assign_capture_to_patient(
            &directory,
            &destination,
            session.patient_id().expect("selected dossier"),
        );
        assert!(matches!(assign_result, Err(error) if error.kind() == io::ErrorKind::WouldBlock));

        // A candidate collected before the journal appeared must also be
        // refused when its visible page is resolved.
        let resolved = resolve_library_candidates_cancellable(&directory, &candidates, &|| false);
        assert!(matches!(resolved, Err(error) if error.kind() == io::ErrorKind::WouldBlock));
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn replay_never_links_a_replaced_or_edited_capture() {
        let directory = test_directory("edited");
        let session = selected_session(&directory);
        let (journal, destination) = pending_fixture(&directory, &session, true);
        fs::write(&destination, b"another person's media bytes!").expect("external edit");
        assert!(recover_pending_capture_metadata(&directory).is_err());
        assert_eq!(
            fs::read(destination).expect("preserved edit"),
            b"another person's media bytes!"
        );
        assert!(journal.exists());
        assert!(
            try_scan_library_directory(&directory)
                .expect("pending capture hidden")
                .is_empty()
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn index_write_error_keeps_published_media_and_replayable_dossier() {
        let directory = test_directory("index-error");
        let session = selected_session(&directory);
        let backup = directory.join(LIBRARY_INDEX_BACKUP_FILE);
        fs::remove_file(&backup).expect("remove backup");
        fs::create_dir(&backup).expect("block backup publication");
        let result = save_indexed_capture(
            &directory,
            "custom.jpg",
            b"published media",
            &session,
            CaptureKind::Photo,
            CaptureTimestamp::now(),
        )
        .expect("media save succeeds");
        assert!(result.metadata_warning.is_some());
        assert_eq!(
            fs::read(result.file_path).expect("published media survives"),
            b"published media"
        );
        fs::remove_dir(backup).expect("unblock index");
        assert_eq!(
            recover_pending_capture_metadata(&directory).expect("replay after index available"),
            1
        );
        assert_eq!(
            try_scan_library_directory(&directory).expect("scan")[0].patient_id,
            session.patient_id()
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn named_capture_without_dossier_is_rejected_before_publication() {
        let directory = test_directory("unselected");
        let session = CaptureSession::new("Jean", "Dupont", Eye::Left);
        assert!(
            save_indexed_capture(
                &directory,
                "custom.jpg",
                b"media",
                &session,
                CaptureKind::Photo,
                CaptureTimestamp::now()
            )
            .is_err()
        );
        assert!(!directory.join("custom.jpg").exists());
        assert_no_transaction_files(&directory);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    fn assert_no_transaction_files(directory: &Path) {
        assert!(
            fs::read_dir(directory)
                .expect("scan directory")
                .all(|entry| {
                    !entry
                        .expect("entry")
                        .file_name()
                        .to_string_lossy()
                        .starts_with(JOURNAL_PREFIX)
                })
        );
    }

    #[test]
    fn interrupted_staging_write_removes_the_partial_file() {
        let directory = test_directory("stage-write-error");
        let result = prepare_staged_file(&directory, |file| {
            file.write_all(b"partial bytes")?;
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "interrupted media write",
            ))
        });
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::WriteZero));
        assert_no_transaction_files(&directory);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn rejected_recording_preserves_source_and_removes_uncommitted_stage() {
        let directory = test_directory("rejected-recording");
        let source = directory.join("source.part");
        fs::write(&source, b"completed original recording").expect("source");
        let session = CaptureSession::new("Jean", "Dupont", Eye::Left);
        assert!(
            publish_indexed_capture(
                &source,
                &directory,
                "custom.avi",
                &session,
                CaptureKind::Video,
                CaptureTimestamp::now(),
            )
            .is_err()
        );
        assert_eq!(
            fs::read(source).expect("source kept"),
            b"completed original recording"
        );
        assert!(!directory.join("custom.avi").exists());
        assert_no_transaction_files(&directory);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn blocked_journal_temp_removes_the_reserved_visible_file() {
        let directory = test_directory("blocked-journal-temp");
        let mut staging = prepare_staged_file(&directory, |file| {
            file.write_all(b"completed media")?;
            file.sync_all()
        })
        .expect("stage media");
        let staging_path = staging.path.clone();
        let temporary = staging_path.with_extension("json.tmp");
        fs::create_dir(&temporary).expect("block temporary journal");
        let result = commit_staged_capture(
            &directory,
            "custom.jpg",
            &mut staging,
            &CaptureSession::default(),
            CaptureKind::Photo,
            CaptureTimestamp::now(),
        );
        assert!(result.is_err());
        drop(staging);
        assert!(!directory.join("custom.jpg").exists());
        assert!(!staging_path.exists());
        assert!(!staging_path.with_extension("json").exists());
        assert!(temporary.is_dir());
        assert!(
            pending_destination_names(&directory)
                .expect("ignored temp")
                .is_empty()
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn journal_collision_preserves_the_existing_transaction() {
        let directory = test_directory("journal-collision");
        let journal = directory.join(format!("{JOURNAL_PREFIX}existing.json"));
        fs::write(&journal, b"existing dossier journal").expect("existing journal");
        let result = install_capture_journal(&journal, b"new dossier journal");
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::AlreadyExists));
        assert_eq!(
            fs::read(&journal).expect("preserved"),
            b"existing dossier journal"
        );
        assert!(!journal.with_extension("json.tmp").exists());
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn preparation_cleanup_never_removes_a_replacement_file() {
        let directory = test_directory("stage-replacement");
        let staging = prepare_staged_file(&directory, |file| file.write_all(b"owned media"))
            .expect("stage media");
        let path = staging.path.clone();
        fs::rename(&path, directory.join("original.data")).expect("move owned media");
        fs::write(&path, b"external replacement").expect("external replacement");
        drop(staging);
        assert_eq!(
            fs::read(path).expect("replacement kept"),
            b"external replacement"
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_identity_rejects_replacement_even_with_identical_creation_time() {
        let directory = test_directory("same-created-replacement");
        let path = directory.join("original.data");
        fs::write(&path, b"identical bytes").expect("original");
        let mut identity = FileIdentity::of(&File::open(&path).expect("original file"))
            .expect("original identity");
        fs::rename(&path, directory.join("moved.data")).expect("move original");
        fs::write(&path, b"identical bytes").expect("replacement");
        let replacement = File::open(&path).expect("replacement file");
        let actual = FileIdentity::of(&replacement).expect("replacement identity");
        identity.created_nanos = actual.created_nanos;
        assert!(!identity.matches(&replacement));
        // A timestamp-only journal must not authorize destructive recovery.
        identity.file_id = None;
        identity.device = None;
        assert!(!identity.matches(&replacement));
        drop(replacement);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn prepared_file_cleanup_removes_its_own_file_after_writing() {
        let directory = test_directory("owned-stage-cleanup");
        let mut staging = PreparedFile::create(directory.join("owned.data")).expect("stage");
        staging
            .file_mut()
            .write_all(b"completed bytes")
            .expect("write");
        let path = staging.path.clone();
        drop(staging);
        assert!(!path.exists());
        fs::remove_dir_all(directory).expect("cleanup");
    }
}
