//! Content fingerprints preserve dossier links across byte-identical backups,
//! while filesystem versions make unchanged large recordings cheap to validate.

use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::{LazyLock, Mutex},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_VALIDATION_CACHE_ITEMS: usize = 8_192;
const MAX_VALIDATION_QUEUE_ITEMS: usize = MAX_VALIDATION_CACHE_ITEMS * 4;
// Even nanosecond timestamps can reuse the kernel's current clock tick. A
// baseline read during that interval must never enter the metadata digest cache.
const METADATA_STABILITY_WINDOW: Duration = Duration::from_secs(1);

/// Observable filesystem version of a regular capture file.
///
/// Full versions include a content digest as well as native identity and change
/// time. Their tokens do not change when a file becomes old enough to validate
/// cheaply. Metadata-only versions returned by the fast APIs omit the digest.
// Deserialized values are only compared as metadata. They never provide the
// native pointer/handle used by the Windows query.
#[allow(clippy::unsafe_derive_deserialize)]
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct CaptureFileVersion {
    length: u64,
    modified_nanos: Option<u128>,
    created_nanos: Option<u128>,
    change_time: Option<i128>,
    device: Option<u64>,
    file_id: Option<u128>,
    #[serde(default)]
    weak_digest: Option<[u8; 32]>,
}

impl CaptureFileVersion {
    /// Whether identity and change time allow validation without reading media bytes.
    ///
    /// A false value requires a streaming content hash, suitable for background
    /// workers but unsuitable for an interactive video player's per-frame checks.
    #[must_use]
    pub fn supports_fast_validation(&self) -> bool {
        self.is_strong()
    }

    /// Compares native metadata with a previously verified content version.
    ///
    /// Only use this with a successful fast API result, after validating the full
    /// content version. It deliberately does not replace full-version equality.
    #[must_use]
    pub fn same_native_metadata(&self, other: &Self) -> bool {
        self.length == other.length
            && self.modified_nanos == other.modified_nanos
            && self.created_nanos == other.created_nanos
            && self.change_time == other.change_time
            && self.device == other.device
            && self.file_id == other.file_id
    }

    /// Encodes a version for passing the displayed capture through UI callbacks.
    #[must_use]
    #[allow(clippy::missing_panics_doc)] // Serializing these scalar fields is infallible.
    pub fn token(&self) -> String {
        serde_json::to_string(self).expect("filesystem version has only serializable scalar fields")
    }

    /// Decodes a version previously supplied by the library.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not a valid serialized file version.
    pub fn from_token(token: &str) -> io::Result<Self> {
        if token.len() > 2_048 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file version token is too long",
            ));
        }
        serde_json::from_str(token)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    }

    fn from_file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "capture is not a regular file",
            ));
        }
        #[allow(unused_mut)]
        let mut version = Self::from_metadata(&metadata);
        #[cfg(target_os = "windows")]
        version.read_windows_identity(file);
        Ok(version)
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[allow(unused_mut)]
        let mut version = Self {
            length: metadata.len(),
            modified_nanos: metadata.modified().ok().and_then(system_time_nanos),
            created_nanos: metadata.created().ok().and_then(system_time_nanos),
            change_time: None,
            device: None,
            file_id: None,
            weak_digest: None,
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            // Filesystems exposing only whole-second change times cannot rule
            // out an in-place rewrite within that second. Hash those versions.
            version.change_time = (metadata.ctime_nsec() != 0).then(|| {
                i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec())
            });
            version.device = Some(metadata.dev());
            version.file_id = Some(u128::from(metadata.ino()));
        }
        version
    }

    #[cfg(target_os = "windows")]
    fn read_windows_identity(&mut self, file: &File) {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo,
                GetFileInformationByHandleEx,
            },
        };

        let handle = HANDLE(file.as_raw_handle());
        let mut basic = FILE_BASIC_INFO::default();
        let mut identity = FILE_ID_INFO::default();
        // SAFETY: the File owns a live handle, and each buffer has the exact
        // Windows structure and size required by its information class.
        unsafe {
            if GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                std::ptr::from_mut(&mut basic).cast(),
                u32::try_from(std::mem::size_of::<FILE_BASIC_INFO>())
                    .expect("small Windows structure"),
            )
            .is_ok()
                && basic.ChangeTime != 0
            {
                self.change_time = Some(i128::from(basic.ChangeTime));
            }
            if GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                std::ptr::from_mut(&mut identity).cast(),
                u32::try_from(std::mem::size_of::<FILE_ID_INFO>())
                    .expect("small Windows structure"),
            )
            .is_ok()
            {
                self.device = Some(identity.VolumeSerialNumber);
                self.file_id = Some(u128::from_le_bytes(identity.FileId.Identifier));
            }
        }
    }

    pub(crate) fn is_strong(&self) -> bool {
        self.change_time.is_some() && self.device.is_some() && self.file_id.is_some()
    }

    fn metadata_is_stable(&self, now: SystemTime) -> bool {
        if !self.is_strong() {
            return false;
        }
        let Some(change_time) = self.change_time else {
            return false;
        };
        // Windows FILE_BASIC_INFO uses 100 ns units since 1601; Unix uses
        // nanoseconds since 1970. Keep the serialized native values unchanged.
        #[cfg(target_os = "windows")]
        let change_nanos = (change_time - 116_444_736_000_000_000) * 100;
        #[cfg(not(target_os = "windows"))]
        let change_nanos = change_time;
        let Some(now_nanos) = system_time_nanos(now).and_then(|time| i128::try_from(time).ok())
        else {
            return false;
        };
        now_nanos.checked_sub(change_nanos).is_some_and(|age| {
            age >= i128::try_from(METADATA_STABILITY_WINDOW.as_nanos())
                .expect("small stability window")
        })
    }

    pub(crate) fn content_digest(&self) -> Option<[u8; 32]> {
        self.weak_digest
    }
}

/// Reads the filesystem version of an already opened capture.
///
/// # Errors
///
/// Returns an error if its handle cannot be queried or is not a regular file.
pub fn capture_file_version_from_file(file: &File) -> io::Result<CaptureFileVersion> {
    capture_file_version_from_file_cancellable(file, &|| false)
}

/// Computes a complete SHA version of an open capture, checking cancellation
/// between media blocks while retaining the caller's file cursor.
///
/// # Errors
///
/// Returns `Interrupted` when cancelled, or an I/O error on invalid media.
pub fn capture_file_version_from_file_cancellable(
    file: &File,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<CaptureFileVersion> {
    check_cancelled(is_cancelled)?;
    let mut version = CaptureFileVersion::from_file(file)?;
    let cacheable = version.metadata_is_stable(SystemTime::now());
    if cacheable && let Some(digest) = cached_digest(&version) {
        check_cancelled(is_cancelled)?;
        if CaptureFileVersion::from_file(file)? != version {
            return Err(changed_capture());
        }
        version.weak_digest = Some(digest);
        return Ok(version);
    }
    let mut reader = file.try_clone()?;
    let original_position = reader.stream_position()?;
    let result = (|| {
        reader.seek(SeekFrom::Start(0))?;
        let digest = hash_reader(&mut reader, is_cancelled)?;
        if CaptureFileVersion::from_file(file)? != version {
            return Err(changed_capture());
        }
        Ok(digest)
    })();
    // Duplicated handles can share their seek cursor. Restore it even on error.
    let restore = reader.seek(SeekFrom::Start(original_position));
    let digest = result?;
    restore?;
    // A hash started before the window elapsed must be read again after it;
    // merely waiting cannot prove that no same-tick edit changed its bytes.
    if cacheable {
        cache_digest(version.clone(), digest);
    }
    version.weak_digest = Some(digest);
    Ok(version)
}

/// Reads only native metadata from an opened file, without hashing media bytes.
///
/// # Errors
///
/// Returns `WouldBlock` until its last change is at least one second old,
/// `Unsupported` when identity/change time are unavailable, or an I/O error.
pub fn capture_file_version_fast_from_file(file: &File) -> io::Result<CaptureFileVersion> {
    require_fast_version(CaptureFileVersion::from_file(file)?)
}

/// Reads only native metadata from a regular capture path, without a content hash.
///
/// # Errors
///
/// Returns `WouldBlock` for changes less than one second old, `Unsupported` for
/// weak filesystem versions, or an I/O error for missing/non-regular paths.
pub fn capture_file_version_fast(path: &Path) -> io::Result<CaptureFileVersion> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        require_fast_version(CaptureFileVersion::from_metadata(&metadata))
    }
    #[cfg(not(unix))]
    {
        let version = capture_file_version_fast_from_file(&File::open(path)?)?;
        if !fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(changed_capture());
        }
        Ok(version)
    }
}

fn require_fast_version(version: CaptureFileVersion) -> io::Result<CaptureFileVersion> {
    if !version.is_strong() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem version requires a content hash",
        ));
    }
    if !version.metadata_is_stable(SystemTime::now()) {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "capture change time has not settled yet",
        ));
    }
    Ok(version)
}

/// Reads the current version without following a symbolic link to a capture.
///
/// # Errors
///
/// Returns an error if the path is missing, is a symlink or is not a regular file.
pub fn capture_file_version(path: &Path) -> io::Result<CaptureFileVersion> {
    capture_file_version_cancellable(path, &|| false)
}

/// Computes a full content version from a path, with cooperative cancellation.
///
/// # Errors
///
/// Returns `Interrupted` when cancelled, or an I/O error on changed media.
pub fn capture_file_version_cancellable(
    path: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<CaptureFileVersion> {
    check_cancelled(is_cancelled)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture is not a regular file",
        ));
    }
    let file = File::open(path)?;
    let version = capture_file_version_from_file_cancellable(&file, is_cancelled)?;
    check_cancelled(is_cancelled)?;
    let after = fs::symlink_metadata(path)?;
    if !after.file_type().is_file() {
        return Err(changed_capture());
    }
    #[cfg(unix)]
    if !CaptureFileVersion::from_metadata(&after).same_native_metadata(&version) {
        return Err(changed_capture());
    }
    #[cfg(not(unix))]
    if !CaptureFileVersion::from_file(&File::open(path)?)?.same_native_metadata(&version) {
        return Err(changed_capture());
    }
    Ok(version)
}

#[derive(Default)]
struct ValidationCache {
    clock: u64,
    entries: HashMap<CaptureFileVersion, ([u8; 32], u64)>,
    order: VecDeque<(CaptureFileVersion, u64)>,
}

static VALIDATION_CACHE: LazyLock<Mutex<ValidationCache>> =
    LazyLock::new(|| Mutex::new(ValidationCache::default()));

pub(crate) fn capture_fingerprint(path: &Path) -> io::Result<(CaptureFileVersion, [u8; 32])> {
    let version = capture_file_version(path)?;
    let digest = version
        .weak_digest
        .expect("complete versions contain a digest");
    Ok((version, digest))
}

fn cached_digest(version: &CaptureFileVersion) -> Option<[u8; 32]> {
    let mut cache = VALIDATION_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.clock = cache.clock.wrapping_add(1);
    let clock = cache.clock;
    let (digest, last_used) = cache.entries.get_mut(version)?;
    *last_used = clock;
    let digest = *digest;
    cache.order.push_back((version.clone(), clock));
    compact_cache_queue(&mut cache);
    Some(digest)
}

fn cache_digest(version: CaptureFileVersion, digest: [u8; 32]) {
    let mut cache = VALIDATION_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.clock = cache.clock.wrapping_add(1);
    let clock = cache.clock;
    while cache.entries.len() >= MAX_VALIDATION_CACHE_ITEMS {
        let Some((oldest, generation)) = cache.order.pop_front() else {
            break;
        };
        if cache
            .entries
            .get(&oldest)
            .is_some_and(|(_, current)| *current == generation)
        {
            cache.entries.remove(&oldest);
        }
    }
    cache.entries.insert(version.clone(), (digest, clock));
    cache.order.push_back((version, clock));
    compact_cache_queue(&mut cache);
}

fn compact_cache_queue(cache: &mut ValidationCache) {
    if cache.order.len() <= MAX_VALIDATION_QUEUE_ITEMS {
        return;
    }
    let mut order: Vec<_> = cache
        .entries
        .iter()
        .map(|(version, (_, generation))| (version.clone(), *generation))
        .collect();
    order.sort_unstable_by_key(|(_, generation)| *generation);
    cache.order = order.into_iter().collect();
}

fn hash_reader(reader: &mut File, is_cancelled: &dyn Fn() -> bool) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        check_cancelled(is_cancelled)?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..count]);
    }
}

pub(crate) fn system_time_nanos(time: SystemTime) -> Option<u128> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_nanos())
}

fn check_cancelled(is_cancelled: &dyn Fn() -> bool) -> io::Result<()> {
    if is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "capture validation cancelled",
        ))
    } else {
        Ok(())
    }
}

fn changed_capture() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "capture changed while its fingerprint was read",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs,
        io::{Seek, SeekFrom},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{capture_file_version, capture_file_version_from_file_cancellable};

    #[test]
    fn immediate_equal_length_edit_changes_full_version_even_with_restored_mtime() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope-version-{unique}.jpg"));
        fs::write(&path, b"AAAAAAAAAAAAA").expect("first thirteen bytes");
        let first_modified = fs::metadata(&path)
            .expect("metadata")
            .modified()
            .expect("mtime");
        let first = capture_file_version(&path).expect("first version");
        fs::write(&path, b"BBBBBBBBBBBBB").expect("second thirteen bytes");
        fs::File::options()
            .write(true)
            .open(&path)
            .expect("open replacement")
            .set_modified(first_modified)
            .expect("restore mtime");
        let second = capture_file_version(&path).expect("second version");
        assert_eq!(first.length, second.length);
        assert_eq!(first.modified_nanos, second.modified_nanos);
        assert_ne!(first.content_digest(), second.content_digest());
        assert_ne!(first, second);
        fs::remove_file(path).expect("remove test file");
    }

    #[cfg(unix)]
    #[test]
    fn same_tick_metadata_cannot_be_used_as_cached_content() {
        use std::time::Duration;

        use super::CaptureFileVersion;

        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let change = i128::try_from(now.duration_since(UNIX_EPOCH).expect("time").as_nanos())
            .expect("small time");
        let version = CaptureFileVersion {
            length: 13,
            modified_nanos: Some(u128::try_from(change).expect("positive")),
            created_nanos: None,
            change_time: Some(change),
            device: Some(1),
            file_id: Some(1),
            weak_digest: None,
        };
        assert!(!version.metadata_is_stable(now));
        assert!(!version.metadata_is_stable(now + Duration::from_millis(999)));
        assert!(version.metadata_is_stable(now + Duration::from_secs(1)));
    }

    #[test]
    fn cancelling_hash_restores_open_file_cursor() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope-cancel-version-{unique}.jpg"));
        fs::write(&path, vec![42_u8; 256 * 1024]).expect("capture");
        let mut file = fs::File::open(&path).expect("open capture");
        file.seek(SeekFrom::Start(37)).expect("seek");
        let checks = Cell::new(0);
        let error = capture_file_version_from_file_cancellable(&file, &|| {
            checks.set(checks.get() + 1);
            checks.get() >= 4
        })
        .expect_err("cancelled hash");
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(file.stream_position().expect("cursor"), 37);
        fs::remove_file(path).expect("cleanup");
    }
}
