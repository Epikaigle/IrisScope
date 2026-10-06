use crate::config::{DecodedFrame, LIBRARY_PAGE_SIZE, MAX_THUMBNAIL_CACHE_BYTES};
#[cfg(test)]
use crate::library_worker::LibraryRefreshMailbox;
#[cfg(test)]
use crate::photo_worker::{PhotoMailbox, PhotoRequest};
use crate::ui::{LibraryItemData, MainWindow};
use iriscope_core::library::{CaptureKind, LibraryFilter, present_library_items};
use iriscope_core::session::CaptureSession;
use iriscope_core::video::is_iriscope_avi;
use iriscope_imaging::{
    decode_mjpeg_to_rgb8, decode_reference_image_to_rgb8, ensure_jpeg_has_dht, resize_rgb8_to_fit,
};
use slint::{ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) const MAX_REFERENCE_FILE_BYTES: u64 = 32 * 1024 * 1024;
// The map, symbols, viewer and library can request large images at once.
// Serialize their decodes so their temporary RGB buffers do not accumulate.
pub(super) static REFERENCE_DECODE_LOCK: Mutex<()> = Mutex::new(());

fn reference_decode_guard(
    is_current: &dyn Fn() -> bool,
) -> Option<std::sync::MutexGuard<'static, ()>> {
    let guard = loop {
        if !is_current() {
            return None;
        }
        match REFERENCE_DECODE_LOCK.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    is_current().then_some(guard)
}

pub(crate) fn load_reference_cancellable(
    path: &std::path::Path,
    is_current: &dyn Fn() -> bool,
) -> Option<DecodedFrame> {
    use std::io::Read;
    let _decode_guard = reference_decode_guard(is_current)?;
    if !is_current() {
        return None;
    }
    let mut file = std::fs::File::open(path)
        .ok()?
        .take(MAX_REFERENCE_FILE_BYTES + 1);
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        if !is_current() {
            return None;
        }
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() as u64 > MAX_REFERENCE_FILE_BYTES {
            return None;
        }
    }
    if !is_current() {
        return None;
    }
    let decoded = decode_reference_image_to_rgb8(&bytes).ok();
    is_current().then_some(decoded).flatten()
}

pub(super) fn load_video_thumbnail(path: &std::path::Path) -> Option<DecodedFrame> {
    load_video_thumbnail_cancellable(path, &|| true)
}

fn load_video_thumbnail_cancellable(
    path: &std::path::Path,
    is_current: &dyn Fn() -> bool,
) -> Option<DecodedFrame> {
    use std::io::Read;

    let _decode_guard = reference_decode_guard(is_current)?;
    if !is_current() || !is_iriscope_avi(path).ok()? {
        return None;
    }
    // Read only the first frame, in bounded chunks; serialize its decode with
    // reference images and photos so large temporary RGB buffers cannot overlap.
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0_u8; 2_056];
    file.read_exact(&mut header).ok()?;
    let first_size = u32::from_le_bytes(header[2_052..2_056].try_into().ok()?);
    if u64::from(first_size) > MAX_REFERENCE_FILE_BYTES || !is_current() {
        return None;
    }
    let mut jpeg = vec![0_u8; usize::try_from(first_size).ok()?];
    for chunk in jpeg.chunks_mut(8 * 1024) {
        if !is_current() {
            return None;
        }
        file.read_exact(chunk).ok()?;
    }
    if !is_current() {
        return None;
    }
    let jpeg = ensure_jpeg_has_dht(&jpeg);
    let (width, height, rgb) = decode_mjpeg_to_rgb8(&jpeg).ok()?;
    if !is_current() {
        return None;
    }
    let resized = resize_rgb8_to_fit(&rgb, width, height, 240).ok()?;
    is_current().then_some(resized)
}

#[derive(Clone)]
pub(super) struct LibraryItemPayload {
    pub(super) date_time: String,
    pub(super) dossier_number: String,
    pub(super) eye_label: String,
    pub(super) file_path: String,
    pub(super) file_version: String,
    pub(super) id: String,
    pub(super) is_current_patient: bool,
    pub(super) is_video: bool,
    pub(super) can_open_in_app: bool,
    pub(super) thumbnail: Option<SharedPixelBuffer<Rgb8Pixel>>,
    pub(super) title: String,
}

pub(super) struct CachedThumbnail {
    version: iriscope_core::library::CaptureFileVersion,
    pub(super) image: SharedPixelBuffer<Rgb8Pixel>,
    pub(super) last_used: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct LibraryFileVersion {
    length: u64,
    modified: Option<std::time::SystemTime>,
}

impl LibraryFileVersion {
    fn read(path: &std::path::Path) -> Option<Self> {
        std::fs::symlink_metadata(path).ok().map(|metadata| Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

#[derive(Eq, PartialEq)]
struct LibraryDirectoryVersion {
    directory: Option<LibraryFileVersion>,
    index: Option<iriscope_core::library::CaptureFileVersion>,
}

impl LibraryDirectoryVersion {
    fn read_cancellable(
        directory: &std::path::Path,
        is_current: &dyn Fn() -> bool,
    ) -> std::io::Result<Self> {
        if !is_current() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "library request cancelled",
            ));
        }
        let index = match iriscope_core::library::capture_file_version_cancellable(
            &directory.join(".iriscope-index.json"),
            &|| !is_current(),
        ) {
            Ok(version) => Some(version),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Err(error),
            Err(_) => None,
        };
        Ok(Self {
            directory: LibraryFileVersion::read(directory),
            index,
        })
    }
}

struct LibrarySnapshot {
    directory: std::path::PathBuf,
    version: LibraryDirectoryVersion,
    scanned_at: Instant,
    entries: Vec<iriscope_core::library::LibraryScanCandidate>,
    selection: Option<(i32, Option<u64>, Vec<usize>)>,
}

impl LibrarySnapshot {
    fn page(
        &mut self,
        filter: i32,
        patient_id: Option<u64>,
        requested_page: usize,
        query: &iriscope_core::library::LibraryQuery,
    ) -> (
        Vec<iriscope_core::library::LibraryScanCandidate>,
        usize,
        usize,
    ) {
        let selected_patient = if filter == 3 { patient_id } else { None };
        if self
            .selection
            .as_ref()
            .is_none_or(|(old_filter, old_patient, _)| {
                (*old_filter, *old_patient) != (filter, selected_patient)
            })
        {
            let indices = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let matches = match filter {
                        1 => entry.kind == CaptureKind::Photo,
                        2 => entry.kind == CaptureKind::Video,
                        3 => selected_patient.is_some_and(|id| entry.patient_id_hint == Some(id)),
                        _ => true,
                    };
                    (matches && query.matches(entry)).then_some(index)
                })
                .collect();
            let mut indices: Vec<usize> = indices;
            if query.oldest_first {
                indices.reverse();
            }
            self.selection = Some((filter, selected_patient, indices));
        }
        let indices = &self
            .selection
            .as_ref()
            .expect("selection was initialized")
            .2;
        let total = indices.len();
        let page = requested_page.min(total.saturating_sub(1) / LIBRARY_PAGE_SIZE);
        let entries = indices
            .iter()
            .skip(page * LIBRARY_PAGE_SIZE)
            .take(LIBRARY_PAGE_SIZE)
            .map(|&index| self.entries[index].clone())
            .collect();
        (entries, total, page)
    }
}

#[derive(Default)]
pub(super) struct ThumbnailCache {
    pub(super) entries: HashMap<std::path::PathBuf, CachedThumbnail>,
    pub(super) bytes: usize,
    pub(super) clock: u64,
    snapshot: Option<LibrarySnapshot>,
    query: iriscope_core::library::LibraryQuery,
}

impl ThumbnailCache {
    pub(super) fn set_query(&mut self, query: iriscope_core::library::LibraryQuery) {
        if self.query != query {
            self.query = query;
            if let Some(snapshot) = self.snapshot.as_mut() {
                snapshot.selection = None;
            }
        }
    }
    pub(super) fn invalidate(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.snapshot = None;
    }

    fn library_page(
        &mut self,
        directory: &std::path::Path,
        patient_id: Option<u64>,
        filter: i32,
        requested_page: usize,
        is_current: &dyn Fn() -> bool,
    ) -> std::io::Result<(Vec<iriscope_core::library::LibraryEntry>, usize, usize)> {
        // Navigation reuses the sorted directory and filter selection. External
        // in-place edits are checked on the visible page; a periodic rescan also
        // discovers changes elsewhere that did not update the directory mtime.
        let mut mutation_retries = 0;
        let mut check_expiry = true;
        while mutation_retries < 3 {
            let version = LibraryDirectoryVersion::read_cancellable(directory, is_current)?;
            let refresh = self.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.directory != directory
                    || snapshot.version != version
                    || (check_expiry && snapshot.scanned_at.elapsed() >= Duration::from_secs(30))
            });
            check_expiry = false;
            if refresh {
                let entries =
                    iriscope_core::library::try_scan_library_directory_metadata_cancellable(
                        directory,
                        &|| !is_current(),
                    )?;
                if LibraryDirectoryVersion::read_cancellable(directory, is_current)? != version {
                    self.snapshot = None;
                    mutation_retries += 1;
                    continue;
                }
                self.retain_paths(
                    &entries
                        .iter()
                        .map(|entry| entry.file_path.clone())
                        .collect(),
                );
                self.snapshot = Some(LibrarySnapshot {
                    directory: directory.to_path_buf(),
                    version,
                    scanned_at: Instant::now(),
                    entries,
                    selection: None,
                });
            }
            let snapshot = self.snapshot.as_mut().expect("snapshot was initialized");
            let result = snapshot.page(filter, patient_id, requested_page, &self.query);
            let entries = match iriscope_core::library::resolve_library_candidates_cancellable(
                directory,
                &result.0,
                &|| !is_current(),
            ) {
                Ok(entries) => entries,
                Err(error)
                    if is_current()
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::NotFound
                        ) =>
                {
                    self.snapshot = None;
                    mutation_retries += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let mut corrected_hint = false;
            for entry in &entries {
                if let Some(candidate) = snapshot
                    .entries
                    .iter_mut()
                    .find(|candidate| candidate.file_path == entry.file_path)
                    && (candidate.patient_id_hint != entry.patient_id
                        || candidate.eye_hint != entry.eye
                        || candidate.date_str != entry.date_str
                        || candidate.time_str != entry.time_str)
                {
                    candidate.patient_id_hint = entry.patient_id;
                    candidate.eye_hint = entry.eye;
                    candidate.date_str.clone_from(&entry.date_str);
                    candidate.time_str.clone_from(&entry.time_str);
                    corrected_hint = true;
                }
            }
            if corrected_hint {
                snapshot
                    .entries
                    .sort_by_key(|candidate| std::cmp::Reverse(candidate.timestamp()));
                snapshot.selection = None;
                // A verified row may move out of the applied date/eye/dossier filter.
                // Rebuild this page from the corrected hints before displaying it.
                continue;
            }
            let unchanged = entries
                .iter()
                .all(|entry| library_entry_unchanged_cancellable(entry, is_current));
            if unchanged
                && LibraryDirectoryVersion::read_cancellable(directory, is_current)?
                    == snapshot.version
            {
                return Ok((entries, result.1, result.2));
            }
            self.snapshot = None;
            mutation_retries += 1;
        }
        Err(std::io::Error::other(
            "La bibliothèque a changé pendant sa lecture. Actualisez-la.",
        ))
    }

    #[cfg(test)]
    pub(super) fn get_or_load(
        &mut self,
        path: &std::path::Path,
        load: impl FnOnce(&std::path::Path) -> Option<DecodedFrame>,
    ) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
        self.get_or_load_cancellable(path, load, &|| true)
    }

    fn get_or_load_cancellable(
        &mut self,
        path: &std::path::Path,
        load: impl FnOnce(&std::path::Path) -> Option<DecodedFrame>,
        is_current: &dyn Fn() -> bool,
    ) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
        let version =
            iriscope_core::library::capture_file_version_cancellable(path, &|| !is_current())
                .ok()?;

        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(path)
            && entry.version == version
        {
            entry.last_used = self.clock;
            return Some(entry.image.clone());
        }
        self.remove(path);
        let (width, height, rgb) = load(path)?;
        if iriscope_core::library::capture_file_version_cancellable(path, &|| !is_current())
            .ok()
            .as_ref()
            != Some(&version)
        {
            return None;
        }
        let image = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
        let image_bytes = image.as_bytes().len();
        if image_bytes <= MAX_THUMBNAIL_CACHE_BYTES {
            while self.bytes.saturating_add(image_bytes) > MAX_THUMBNAIL_CACHE_BYTES {
                let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, item)| item.last_used)
                    .map(|(path, _)| path.clone())
                else {
                    break;
                };
                self.remove(&oldest);
            }
            self.bytes += image_bytes;
            self.entries.insert(
                path.to_path_buf(),
                CachedThumbnail {
                    version,
                    image: image.clone(),
                    last_used: self.clock,
                },
            );
        }
        Some(image)
    }

    pub(super) fn remove(&mut self, path: &std::path::Path) {
        if let Some(removed) = self.entries.remove(path) {
            self.bytes = self.bytes.saturating_sub(removed.image.as_bytes().len());
        }
    }

    pub(super) fn retain_paths(&mut self, paths: &std::collections::HashSet<std::path::PathBuf>) {
        self.entries.retain(|path, _| paths.contains(path));
        self.bytes = self
            .entries
            .values()
            .map(|entry| entry.image.as_bytes().len())
            .sum();
    }
}

fn library_entry_unchanged_cancellable(
    entry: &iriscope_core::library::LibraryEntry,
    is_current: &dyn Fn() -> bool,
) -> bool {
    entry.file_version.as_ref().is_some_and(|expected| {
        iriscope_core::library::capture_file_version_cancellable(&entry.file_path, &|| {
            !is_current()
        })
        .is_ok_and(|current| &current == expected)
    })
}

pub(super) struct LibraryPagePayloads {
    pub(super) items: Vec<LibraryItemPayload>,
    pub(super) total: usize,
    pub(super) page: usize,
    pub(super) error: Option<String>,
}

#[cfg(test)]
pub(super) fn load_library_payloads(
    dir: &std::path::Path,
    active_session: &CaptureSession,
    filter: i32,
    requested_page: usize,
    cache: &mut ThumbnailCache,
    is_current: impl Fn() -> bool,
) -> Option<LibraryPagePayloads> {
    load_library_payloads_inner(
        dir,
        active_session,
        filter,
        requested_page,
        cache,
        is_current,
        true,
    )
}

pub(super) fn load_library_metadata(
    dir: &std::path::Path,
    active_session: &CaptureSession,
    filter: i32,
    requested_page: usize,
    cache: &mut ThumbnailCache,
    is_current: impl Fn() -> bool,
) -> Option<LibraryPagePayloads> {
    load_library_payloads_inner(
        dir,
        active_session,
        filter,
        requested_page,
        cache,
        is_current,
        false,
    )
}

pub(super) fn load_library_payloads_inner(
    dir: &std::path::Path,
    active_session: &CaptureSession,
    filter: i32,
    requested_page: usize,
    cache: &mut ThumbnailCache,
    is_current: impl Fn() -> bool,
    load_thumbnails: bool,
) -> Option<LibraryPagePayloads> {
    if !is_current() {
        return None;
    }
    let (page_entries, total, page) = match cache.library_page(
        dir,
        active_session.patient_id(),
        filter,
        requested_page,
        &is_current,
    ) {
        Ok(page) => page,
        Err(_) if !is_current() => return None,
        Err(error) => {
            return Some(LibraryPagePayloads {
                items: Vec::new(),
                total: 0,
                page: 0,
                error: Some(format!(
                    "Impossible de lire le dossier des captures : {error}"
                )),
            });
        }
    };
    if !is_current() {
        return None;
    }
    let start = page * LIBRARY_PAGE_SIZE;
    let presented = present_library_items(&page_entries, active_session, LibraryFilter::All);
    let mut payloads = Vec::with_capacity(total.saturating_sub(start).min(LIBRARY_PAGE_SIZE));
    for (idx, item) in presented.into_iter().enumerate() {
        if filter == 3 && !item.is_current_patient {
            continue;
        }
        if !is_current() {
            return None;
        }
        let thumbnail = if load_thumbnails {
            load_library_thumbnail(item.kind, &item.file_path, cache, &is_current)
        } else {
            None
        };

        payloads.push(LibraryItemPayload {
            date_time: item.date_time,
            dossier_number: item.dossier_number.unwrap_or_default(),
            eye_label: item.eye_label,
            file_path: item.file_path.to_string_lossy().to_string(),
            file_version: page_entries[idx].file_version.as_ref().map_or_else(
                String::new,
                iriscope_core::library::CaptureFileVersion::token,
            ),
            id: (start + idx).to_string(),
            is_current_patient: item.is_current_patient,
            is_video: matches!(item.kind, CaptureKind::Video),
            can_open_in_app: !matches!(item.kind, CaptureKind::Video)
                || (page_entries[idx].file_version.as_ref().is_some_and(
                    iriscope_core::library::CaptureFileVersion::supports_fast_validation,
                ) && is_iriscope_avi(&item.file_path).unwrap_or(false)),
            thumbnail,
            title: item.display_title,
        });
    }
    // Decode may take long enough for another process to replace an earlier
    // file in this page. Reject the whole result before presenting an image
    // alongside a stale dossier. A new request will reload the snapshot.
    if !page_entries
        .iter()
        .all(|entry| library_entry_unchanged_cancellable(entry, &is_current))
        || cache.snapshot.as_ref().is_none_or(|snapshot| {
            LibraryDirectoryVersion::read_cancellable(dir, &is_current)
                .ok()
                .as_ref()
                != Some(&snapshot.version)
        })
    {
        cache.invalidate();
        return Some(LibraryPagePayloads {
            items: Vec::new(),
            total: 0,
            page: 0,
            error: Some(
                "La bibliothèque a changé pendant son chargement. Actualisez-la.".to_owned(),
            ),
        });
    }
    Some(LibraryPagePayloads {
        items: payloads,
        total,
        page,
        error: None,
    })
}

fn load_library_thumbnail(
    kind: CaptureKind,
    path: &std::path::Path,
    cache: &mut ThumbnailCache,
    is_current: &dyn Fn() -> bool,
) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
    cache.get_or_load_cancellable(
        path,
        |path| match kind {
            CaptureKind::Video => load_video_thumbnail_cancellable(path, is_current),
            CaptureKind::Photo => {
                let (width, height, rgb) = load_reference_cancellable(path, is_current)?;
                resize_rgb8_to_fit(&rgb, width, height, 240).ok()
            }
        },
        is_current,
    )
}

pub(super) fn load_payload_thumbnail(
    item: &LibraryItemPayload,
    cache: &mut ThumbnailCache,
    is_current: &dyn Fn() -> bool,
) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
    let path = std::path::Path::new(&item.file_path);
    let matches = || {
        is_current()
            && iriscope_core::library::capture_file_version_cancellable(path, &|| !is_current())
                .is_ok_and(|version| version.token() == item.file_version)
    };
    if !matches() {
        return None;
    }
    let kind = if item.is_video {
        CaptureKind::Video
    } else {
        CaptureKind::Photo
    };
    let pixels = load_library_thumbnail(kind, path, cache, is_current)?;
    matches().then_some(pixels)
}

pub(super) fn present_library_payloads(payloads: Vec<LibraryItemPayload>) -> Vec<LibraryItemData> {
    payloads
        .into_iter()
        .map(|item| {
            let has_thumbnail = item.thumbnail.is_some();
            let thumbnail = item
                .thumbnail
                .map_or_else(slint::Image::default, slint::Image::from_rgb8);
            LibraryItemData {
                date_time: item.date_time.into(),
                dossier_number: item.dossier_number.into(),
                eye_label: item.eye_label.into(),
                file_path: item.file_path.into(),
                file_version: item.file_version.into(),
                has_thumbnail,
                id: item.id.into(),
                is_current_patient: item.is_current_patient,
                is_video: item.is_video,
                can_open_in_app: item.can_open_in_app,
                thumbnail,
                title: item.title.into(),
            }
        })
        .collect()
}

pub(super) fn clear_library_view(win: &MainWindow) {
    win.set_library_items(ModelRc::new(VecModel::from(Vec::new())));
    win.set_library_total_count(0);
    win.set_library_page_summary("".into());
    win.set_library_error("".into());
    win.set_library_loading(true);
}

#[cfg(test)]
mod photo_library_tests {
    use std::{
        cell::Cell,
        path::PathBuf,
        sync::Arc,
        time::{Duration, SystemTime},
    };

    use crate::photo_worker::save_photo;
    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{FrameRate, PixelFormat, Resolution},
        library::scan_library_directory,
        session::{CaptureSession, Eye},
        storage::CaptureTimestamp,
        video::AviMjpegWriter,
    };
    use iriscope_imaging::encode_rgb8_jpeg;

    use super::{
        LIBRARY_PAGE_SIZE, LibraryRefreshMailbox, PhotoMailbox, PhotoRequest, ThumbnailCache,
        load_library_payloads, load_video_thumbnail,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "iriscope-photo-test-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create isolated test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(super) fn request(directory: &std::path::Path) -> PhotoRequest {
        let patient = iriscope_core::library::create_patient(directory, "Ada", "Lovelace")
            .expect("test dossier");
        let mut session = CaptureSession::new("Ada", "Lovelace", Eye::Right);
        session.set_patient_id(Some(patient.id));
        PhotoRequest {
            frame: CapturedFrame {
                sequence_number: 1,
                timestamp: Duration::ZERO,
                pixel_format: PixelFormat::Yuyv,
                resolution: Resolution::new(2, 2),
                data: Arc::from([128_u8; 8]),
            },
            directory: directory.to_path_buf(),
            filename_template: "{prenom}_{nom}_{oeil}_{date}_{heure}".to_owned(),
            session,
            timestamp: CaptureTimestamp {
                year: 2026,
                month: 9,
                day: 23,
                hour: 10,
                minute: 11,
                second: 12,
            },
            context_generation: 0,
        }
    }

    #[test]
    fn metadata_is_presented_without_decoding_and_thumbnail_rejects_replacement() {
        let directory = TestDirectory::new();
        let path = directory.0.join("photo.jpg");
        std::fs::write(&path, encode_rgb8_jpeg(&[80; 12], 2, 2, 90).unwrap()).unwrap();
        let mut cache = ThumbnailCache::default();
        let page = super::load_library_metadata(
            &directory.0,
            &CaptureSession::default(),
            0,
            0,
            &mut cache,
            || true,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].thumbnail.is_none());
        assert_eq!(
            cache.bytes, 0,
            "metadata loading must not decode offscreen images"
        );
        assert!(super::load_payload_thumbnail(&page.items[0], &mut cache, &|| true).is_some());
        std::fs::write(&path, encode_rgb8_jpeg(&[160; 12], 2, 2, 90).unwrap()).unwrap();
        assert!(
            super::load_payload_thumbnail(&page.items[0], &mut cache, &|| true).is_none(),
            "a replacement must never appear under the previous dossier metadata"
        );
    }

    #[test]
    fn reference_decode_cancels_while_its_shared_lock_is_occupied() {
        let guard = super::REFERENCE_DECODE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let checks = Cell::new(0);
        let decoded =
            super::load_reference_cancellable(std::path::Path::new("missing-reference"), &|| {
                checks.set(checks.get() + 1);
                checks.get() < 2
            });
        assert!(decoded.is_none());
        drop(guard);
    }

    #[test]
    fn video_thumbnail_cancels_while_reference_decode_lock_is_occupied() {
        let guard = super::REFERENCE_DECODE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let checks = Cell::new(0);
        let result =
            super::load_video_thumbnail_cancellable(std::path::Path::new("missing.avi"), &|| {
                checks.set(checks.get() + 1);
                checks.get() < 2
            });
        assert!(result.is_none());
        assert_eq!(checks.get(), 2);
        drop(guard);
    }

    #[test]
    pub(super) fn photo_worker_input_is_bounded_and_drains_on_close() {
        let mailbox = PhotoMailbox::default();
        let directory = TestDirectory::new();
        assert!(mailbox.enqueue(request(&directory.0)));
        assert!(mailbox.enqueue(request(&directory.0)));
        assert!(!mailbox.enqueue(request(&directory.0)));
        mailbox.close();
        assert!(mailbox.receive().is_some());
        assert!(mailbox.receive().is_some());
        assert!(mailbox.receive().is_none());
    }

    #[test]
    fn photo_queue_saturates_by_bytes_and_releases_consumed_budget() {
        let directory = TestDirectory::new();
        let mut photo = request(&directory.0);
        photo.frame.data = vec![0_u8; 3].into();
        let mailbox = PhotoMailbox::with_byte_budget(5);
        assert!(mailbox.enqueue(photo));
        let mut second = request(&directory.0);
        second.frame.data = vec![0_u8; 3].into();
        assert!(!mailbox.enqueue(second));
        assert!(mailbox.receive().is_some());
        let mut third = request(&directory.0);
        third.frame.data = vec![0_u8; 3].into();
        assert!(mailbox.enqueue(third));
        mailbox.close();
        assert!(mailbox.receive().is_some());
        assert!(mailbox.receive().is_none());
    }

    #[test]
    pub(super) fn photo_save_writes_png_and_index_metadata() {
        let directory = TestDirectory::new();
        let saved = save_photo(&request(&directory.0)).expect("photo saved");
        assert_eq!(
            saved.path.extension().and_then(|value| value.to_str()),
            Some("png")
        );
        assert!(saved.thumbnail.is_some());
        let entries = scan_library_directory(&directory.0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].first_name.as_deref(), Some("Ada"));
        assert_eq!(entries[0].last_name.as_deref(), Some("Lovelace"));
        assert_eq!(entries[0].eye, Eye::Right);
    }

    #[test]
    pub(super) fn thumbnail_cache_reuses_unchanged_file_and_invalidates_changed_file() {
        let directory = TestDirectory::new();
        let path = directory.0.join("thumbnail.jpg");
        std::fs::write(&path, b"one").expect("write source");
        let mut cache = ThumbnailCache::default();
        let loads = Cell::new(0);
        let loader = |_: &std::path::Path| {
            loads.set(loads.get() + 1);
            Some((1, 1, vec![0, 1, 2]))
        };
        assert!(cache.get_or_load(&path, loader).is_some());
        assert!(cache.get_or_load(&path, loader).is_some());
        assert_eq!(loads.get(), 1);
        std::fs::write(&path, b"changed").expect("change source");
        assert!(cache.get_or_load(&path, loader).is_some());
        assert_eq!(loads.get(), 2);
        cache.retain_paths(&std::collections::HashSet::new());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn thumbnail_cache_rejects_a_file_changed_while_loading() {
        let directory = TestDirectory::new();
        let path = directory.0.join("thumbnail.jpg");
        std::fs::write(&path, b"original").expect("source");
        let mut cache = ThumbnailCache::default();
        let loaded = cache.get_or_load(&path, |source| {
            std::fs::write(source, b"replacement").expect("concurrent replacement");
            Some((1, 1, vec![1, 2, 3]))
        });
        assert!(loaded.is_none());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    pub(super) fn explicit_refresh_reloads_a_thumbnail_with_unchanged_file_metadata() {
        let directory = TestDirectory::new();
        let path = directory.0.join("thumbnail.jpg");
        std::fs::write(&path, b"same metadata").expect("write source");
        let mut cache = ThumbnailCache::default();
        let loads = Cell::new(0_u8);
        let loader = |_: &std::path::Path| {
            loads.set(loads.get() + 1);
            Some((1, 1, vec![loads.get(); 3]))
        };

        assert_eq!(
            cache.get_or_load(&path, loader).unwrap().as_bytes(),
            &[1; 3]
        );
        assert_eq!(
            cache.get_or_load(&path, loader).unwrap().as_bytes(),
            &[1; 3]
        );
        assert_eq!(loads.get(), 1);

        cache.invalidate();
        assert_eq!(cache.bytes, 0);
        assert_eq!(
            cache.get_or_load(&path, loader).unwrap().as_bytes(),
            &[2; 3]
        );
        assert_eq!(loads.get(), 2);
    }

    #[test]
    pub(super) fn library_refresh_keeps_only_latest_request() {
        let mailbox = LibraryRefreshMailbox::default();
        let directory = TestDirectory::new();
        mailbox.request(directory.0.join("old"), CaptureSession::default());
        mailbox.set_filter(2);
        mailbox.set_page(3);
        mailbox.request(directory.0.join("new"), CaptureSession::default());
        let request = mailbox.receive().expect("latest request");
        assert!(request.directory.ends_with("new"));
        assert_eq!(request.filter, 2);
        assert_eq!(request.page, 3);
        assert!(mailbox.is_current(request.revision));
        mailbox.close();
        assert!(!mailbox.is_current(request.revision));
        assert!(mailbox.receive().is_none());
    }

    #[test]
    pub(super) fn resolved_page_is_used_by_later_refreshes_but_stale_results_are_ignored() {
        let mailbox = LibraryRefreshMailbox::default();
        let directory = TestDirectory::new();
        let session = CaptureSession::default();
        mailbox.set_page(1);
        mailbox.request(directory.0.clone(), session.clone());
        let first = mailbox.receive().expect("first request");
        assert_eq!(first.page, 1);

        mailbox.request(directory.0.clone(), session.clone());
        let current = mailbox.receive().expect("current request");
        assert!(!mailbox.set_resolved_page(first.revision, 0));
        assert!(mailbox.set_resolved_page(current.revision, 0));

        mailbox.request(directory.0.clone(), session);
        assert_eq!(mailbox.receive().expect("later refresh").page, 0);
    }

    #[test]
    pub(super) fn only_explicit_refresh_invalidates_cached_thumbnails() {
        let mailbox = LibraryRefreshMailbox::default();
        let directory = TestDirectory::new();
        let session = CaptureSession::default();
        mailbox.request(directory.0.clone(), session.clone());
        let initial = mailbox.receive().expect("initial request");

        mailbox.set_page(1);
        mailbox.request(directory.0.clone(), session.clone());
        let navigation = mailbox.receive().expect("navigation request");
        assert_eq!(navigation.cache_revision, initial.cache_revision);

        mailbox.invalidate_thumbnails();
        mailbox.request(directory.0.clone(), session);
        let explicit_refresh = mailbox.receive().expect("explicit refresh request");
        assert_ne!(explicit_refresh.cache_revision, navigation.cache_revision);
    }

    #[test]
    pub(super) fn library_page_limits_loaded_items_and_keeps_later_captures_accessible() {
        let directory = TestDirectory::new();
        for index in 0..(LIBRARY_PAGE_SIZE + 5) {
            std::fs::write(
                directory.0.join(format!("capture-{index}.jpg")),
                b"invalid jpg",
            )
            .expect("write capture placeholder");
        }
        let mut cache = ThumbnailCache::default();
        let session = CaptureSession::default();
        let first = load_library_payloads(&directory.0, &session, 0, 0, &mut cache, || true)
            .expect("first page");
        assert_eq!(first.total, LIBRARY_PAGE_SIZE + 5);
        assert_eq!(first.items.len(), LIBRARY_PAGE_SIZE);
        assert_eq!(first.page, 0);
        let first_scan = cache.snapshot.as_ref().expect("snapshot").scanned_at;

        let last = load_library_payloads(&directory.0, &session, 0, 99, &mut cache, || true)
            .expect("last page");
        assert_eq!(last.items.len(), 5);
        assert_eq!(last.page, 1);
        assert_eq!(
            cache.snapshot.as_ref().expect("snapshot").scanned_at,
            first_scan
        );
    }

    #[test]
    fn date_eye_and_dossier_query_reorders_cached_pages_and_rejects_replaced_media() {
        use iriscope_core::library::{CaptureKind, LibraryQuery, save_indexed_capture};
        let directory = TestDirectory::new();
        let patient =
            iriscope_core::library::create_patient(&directory.0, "Ada", "Lovelace").unwrap();
        let mut session = CaptureSession::new("Ada", "Lovelace", Eye::Left);
        session.set_patient_id(Some(patient.id));
        for day in 1..=3 {
            let mut timestamp = CaptureTimestamp::now();
            timestamp.year = 2026;
            timestamp.month = 10;
            timestamp.day = day;
            save_indexed_capture(
                &directory.0,
                &format!("Iris_Gauche_2020-01-0{day}_00-00-00.jpg"),
                b"original media",
                &session,
                CaptureKind::Photo,
                timestamp,
            )
            .unwrap();
        }
        let mut cache = ThumbnailCache::default();
        cache.set_query(
            LibraryQuery::parse(&patient.dossier_number, "2026-10-01", "2026-10-03", 1, 1).unwrap(),
        );
        let (entries, total, _) = cache
            .library_page(&directory.0, None, 0, 0, &|| true)
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(entries[0].date_str, "2026-10-01");
        assert_eq!(entries[2].date_str, "2026-10-03");
        let original_scan = cache.snapshot.as_ref().unwrap().scanned_at;
        cache.set_query(LibraryQuery::parse("", "2026-10-01", "2026-10-03", 1, 0).unwrap());
        let (entries, _, _) = cache
            .library_page(&directory.0, None, 0, 0, &|| true)
            .unwrap();
        assert_eq!(entries[0].date_str, "2026-10-03");
        assert_eq!(cache.snapshot.as_ref().unwrap().scanned_at, original_scan);
        std::fs::write(&entries[0].file_path, b"replaced content").unwrap();
        let (entries, total, _) = cache
            .library_page(&directory.0, None, 0, 0, &|| true)
            .unwrap();
        assert_eq!(
            total, 2,
            "replacement loses its trusted date and falls outside the applied period"
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.date_str.as_str() >= "2026-10-01")
        );
        cache.set_query(LibraryQuery::parse("", "", "", 2, 0).unwrap());
        assert_eq!(
            cache
                .library_page(&directory.0, None, 0, 0, &|| true)
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn library_snapshot_detects_new_deleted_and_replaced_captures() {
        let directory = TestDirectory::new();
        let patient = iriscope_core::library::create_patient(&directory.0, "Ada", "Lovelace")
            .expect("patient");
        let mut photo = request(&directory.0);
        photo.session.set_patient_id(Some(patient.id));
        let saved = save_photo(&photo).expect("photo");
        let mut cache = ThumbnailCache::default();
        let (entries, total, _) = cache
            .library_page(&directory.0, Some(patient.id), 3, 0, &|| true)
            .expect("initial scan");
        assert_eq!(total, 1);
        assert_eq!(entries[0].patient_id, Some(patient.id));

        // Replacing the contents while preserving mtime must still remove the
        // old identity. Directory and index metadata have not changed here.
        let original_mtime = std::fs::metadata(&saved.path).unwrap().modified().unwrap();
        let original_size = std::fs::metadata(&saved.path).unwrap().len();
        std::fs::write(
            &saved.path,
            vec![0; usize::try_from(original_size).unwrap()],
        )
        .expect("replace photo with same length");
        std::fs::File::options()
            .write(true)
            .open(&saved.path)
            .unwrap()
            .set_modified(original_mtime)
            .expect("restore mtime");
        let (_, total, _) = cache
            .library_page(&directory.0, Some(patient.id), 3, 0, &|| true)
            .expect("rescan replacement");
        assert_eq!(total, 0);
        let (entries, total, _) = cache
            .library_page(&directory.0, None, 0, 0, &|| true)
            .expect("all captures");
        assert_eq!(total, 1);
        assert_eq!(entries[0].patient_id, None);

        let added = directory.0.join("external.jpg");
        std::fs::write(&added, b"external").expect("add capture");
        assert_eq!(
            cache
                .library_page(&directory.0, None, 0, 0, &|| true)
                .unwrap()
                .1,
            2
        );
        std::fs::remove_file(added).expect("delete capture");
        assert_eq!(
            cache
                .library_page(&directory.0, None, 0, 0, &|| true)
                .unwrap()
                .1,
            1
        );
    }

    #[test]
    fn library_payload_does_not_present_old_dossier_after_replacement() {
        let directory = TestDirectory::new();
        let mut photo = request(&directory.0);
        let patient = iriscope_core::library::create_patient(&directory.0, "Ada", "Lovelace")
            .expect("patient");
        photo.session.set_patient_id(Some(patient.id));
        let saved = save_photo(&photo).expect("photo");
        let mut cache = ThumbnailCache::default();
        let (entries, _, _) = cache
            .library_page(&directory.0, Some(patient.id), 0, 0, &|| true)
            .expect("original page");
        assert_eq!(entries[0].patient_id, Some(patient.id));
        std::fs::write(&saved.path, b"replacement after original page validation")
            .expect("replace source");
        let result = load_library_payloads(&directory.0, &photo.session, 0, 0, &mut cache, || true)
            .expect("page result");
        assert!(
            result
                .items
                .iter()
                .all(|item| item.dossier_number.is_empty() && !item.is_current_patient)
        );
    }

    #[test]
    fn dossier_filter_reaches_valid_capture_after_three_hundred_stale_hints() {
        let directory = TestDirectory::new();
        let photo = request(&directory.0);
        let patient_id = photo.session.patient_id().expect("patient dossier");
        let valid = save_photo(&photo).expect("indexed capture");
        for index in 0..300 {
            std::fs::write(
                directory.0.join(format!("replacement-{index}.jpg")),
                b"replacement",
            )
            .expect("replacement capture");
        }
        let mut candidates =
            iriscope_core::library::try_scan_library_directory_metadata(&directory.0)
                .expect("metadata scan");
        // Model an old index's unverified associations without manufacturing
        // verified identities: only the original photo actually belongs here.
        for candidate in &mut candidates {
            candidate.patient_id_hint = Some(patient_id);
        }
        candidates.sort_by_key(|candidate| candidate.file_path == valid.path);
        let snapshot = super::LibrarySnapshot {
            directory: directory.0.clone(),
            version: super::LibraryDirectoryVersion::read_cancellable(&directory.0, &|| true)
                .expect("directory version"),
            scanned_at: std::time::Instant::now(),
            entries: candidates,
            selection: None,
        };
        let mut cache = ThumbnailCache {
            snapshot: Some(snapshot),
            ..ThumbnailCache::default()
        };
        let (entries, total, page) = cache
            .library_page(&directory.0, Some(patient_id), 3, 0, &|| true)
            .expect("stale hints must not exhaust mutation retries");
        assert_eq!(total, 1);
        assert_eq!(page, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_path, valid.path);
        assert_eq!(entries[0].patient_id, Some(patient_id));
    }

    #[test]
    fn library_snapshot_recomputes_filter_when_selected_dossier_changes() {
        let directory = TestDirectory::new();
        let first = iriscope_core::library::create_patient(&directory.0, "Ada", "Lovelace")
            .expect("first patient");
        let second = iriscope_core::library::create_patient(&directory.0, "Ada", "Lovelace")
            .expect("homonym");
        let mut photo = request(&directory.0);
        photo.session.set_patient_id(Some(first.id));
        save_photo(&photo).expect("first patient's capture");
        let mut cache = ThumbnailCache::default();
        assert_eq!(
            cache
                .library_page(&directory.0, Some(first.id), 3, 0, &|| true)
                .unwrap()
                .1,
            1
        );
        let scanned_at = cache.snapshot.as_ref().unwrap().scanned_at;
        assert_eq!(
            cache
                .library_page(&directory.0, Some(second.id), 3, 0, &|| true)
                .unwrap()
                .1,
            0
        );
        assert_eq!(cache.snapshot.as_ref().unwrap().scanned_at, scanned_at);
        assert_eq!(
            cache
                .library_page(&directory.0, None, 0, 0, &|| true)
                .unwrap()
                .1,
            1
        );
    }

    #[test]
    pub(super) fn video_thumbnail_reads_a_valid_first_frame() {
        let directory = TestDirectory::new();
        let path = directory.0.join("capture.avi");
        let jpeg = encode_rgb8_jpeg(&[255, 0, 0], 1, 1, 90).expect("encode JPEG");
        let mut writer =
            AviMjpegWriter::create(&path, 1, 1, FrameRate::new(8, 1).expect("frame rate"))
                .expect("create AVI");
        writer.write_frame(&jpeg).expect("write frame");
        writer.finish().expect("finalize AVI");
        assert_eq!(
            load_video_thumbnail(&path).map(|frame| (frame.0, frame.1)),
            Some((240, 240))
        );
    }
}
