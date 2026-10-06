use crate::config::{DE400_PRODUCT_ID, DE400_VENDOR_ID, DecodedFrame, NOTICE_ERROR};
use crate::ui::{AppState, MainWindow};
use iriscope_core::camera::{CameraDescriptor, CameraErrorKind};
use iriscope_core::library::{
    CaptureFileVersion, capture_file_version_fast, capture_file_version_fast_from_file,
};
use iriscope_core::video::AviMjpegReader;
use iriscope_imaging::{decode_mjpeg_to_rgb8, decode_reference_image_to_rgb8, ensure_jpeg_has_dht};
use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use std::{io, thread};

pub(super) const SLOW_VIDEO_VALIDATION_NOTICE: &str = "Ce support ne permet pas la vérification rapide nécessaire à la lecture vidéo. Utilisez « Application du PC » dans la bibliothèque.";
const UNSETTLED_VIDEO_NOTICE: &str =
    "Le fichier vidéo est encore en cours de modification. Réessayez dans un instant.";
const VIDEO_STABILITY_TIMEOUT: Duration = Duration::from_millis(1_200);

fn open_verified_video(
    path: &Path,
    expected: &CaptureFileVersion,
    is_cancelled: impl Fn() -> bool,
) -> io::Result<File> {
    if !expected.supports_fast_validation() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            SLOW_VIDEO_VALIDATION_NOTICE,
        ));
    }
    if is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "viewer request cancelled",
        ));
    }
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(changed_capture_error());
    }
    let file = File::open(path)?;
    let deadline = Instant::now() + VIDEO_STABILITY_TIMEOUT;
    loop {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "viewer request cancelled",
            ));
        }
        match verify_video_metadata(&file, path, expected) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        UNSETTLED_VIDEO_NOTICE,
                    ));
                }
                thread::sleep(remaining.min(Duration::from_millis(20)));
            }
            Err(error) => return Err(error),
        }
    }
    // A digest read while ctime was still recent could have missed a same-tick
    // write. Read it again after stabilization before trusting metadata alone.
    verify_open_capture_cancellable(&file, path, expected, &is_cancelled)?;
    if is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "viewer request cancelled",
        ));
    }
    verify_video_metadata(&file, path, expected)?;
    Ok(file)
}

#[cfg(test)]
pub(super) fn open_verified_capture(
    path: &Path,
    expected: &CaptureFileVersion,
) -> io::Result<File> {
    open_verified_capture_cancellable(path, expected, &|| false)
}

pub(crate) fn open_verified_capture_cancellable(
    path: &Path,
    expected: &CaptureFileVersion,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<File> {
    if !capture_path_matches_cancellable(path, expected, is_cancelled) {
        return Err(changed_capture_error());
    }
    let file = File::open(path)?;
    verify_open_capture_cancellable(&file, path, expected, is_cancelled)?;
    Ok(file)
}

#[cfg(test)]
pub(super) fn verify_open_capture(
    file: &File,
    path: &Path,
    expected: &CaptureFileVersion,
) -> io::Result<()> {
    verify_open_capture_cancellable(file, path, expected, &|| false)
}

pub(crate) fn verify_open_capture_cancellable(
    file: &File,
    path: &Path,
    expected: &CaptureFileVersion,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<()> {
    let current =
        iriscope_core::library::capture_file_version_from_file_cancellable(file, is_cancelled)?;
    if current != *expected || !capture_path_matches_cancellable(path, expected, is_cancelled) {
        return Err(changed_capture_error());
    }
    Ok(())
}

fn capture_path_matches_cancellable(
    path: &Path,
    expected: &CaptureFileVersion,
    is_cancelled: &dyn Fn() -> bool,
) -> bool {
    iriscope_core::library::capture_file_version_cancellable(path, is_cancelled)
        .is_ok_and(|version| version == *expected)
}

fn verify_video_metadata(
    file: &File,
    path: &Path,
    expected: &CaptureFileVersion,
) -> io::Result<()> {
    let handle_version = capture_file_version_fast_from_file(file)?;
    if !handle_version.same_native_metadata(expected)
        || !capture_file_version_fast(path)?.same_native_metadata(expected)
    {
        return Err(changed_capture_error());
    }
    Ok(())
}

fn changed_capture_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "La capture a été modifiée ou n'est plus disponible. Actualisez la bibliothèque.",
    )
}

fn close_changed_capture(win: &MainWindow) {
    win.global::<AppState>().invoke_close_viewer();
    win.set_viewer_open(false);
    win.set_viewer_image(slint::Image::default());
    win.set_viewer_loading(false);
    win.set_viewer_video_playing(false);
    win.set_selected_library_path("".into());
    win.set_selected_library_file_version("".into());
    win.set_selected_library_dossier_number("".into());
    show_capture_notice(
        win,
        "La capture a été modifiée ou n'est plus disponible. La bibliothèque va être actualisée.",
        NOTICE_ERROR,
    );
    win.global::<AppState>().invoke_refresh_library();
}

#[cfg(test)]
fn read_verified_photo(
    path: &Path,
    expected: &CaptureFileVersion,
) -> io::Result<Option<DecodedFrame>> {
    read_verified_photo_cancellable(path, expected, &|| false)
}

pub(super) fn read_verified_photo_cancellable(
    path: &Path,
    expected: &CaptureFileVersion,
    is_cancelled: &dyn Fn() -> bool,
) -> io::Result<Option<DecodedFrame>> {
    let _decode_guard = loop {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "viewer cancelled",
            ));
        }
        match super::library_ui::REFERENCE_DECODE_LOCK.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(20)),
        }
    };
    if is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "viewer cancelled",
        ));
    }
    // Bound the input before weak-filesystem validation streams its digest.
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(changed_capture_error());
    }
    if metadata.len() > super::library_ui::MAX_REFERENCE_FILE_BYTES {
        return Ok(None);
    }
    let file = open_verified_capture_cancellable(path, expected, is_cancelled)?;
    let mut bytes = Vec::new();
    let mut reader = (&file).take(super::library_ui::MAX_REFERENCE_FILE_BYTES + 1);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        if is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "viewer cancelled",
            ));
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    verify_open_capture_cancellable(&file, path, expected, is_cancelled)?;
    if bytes.len() as u64 > super::library_ui::MAX_REFERENCE_FILE_BYTES {
        return Ok(None);
    }
    let decoded = decode_reference_image_to_rgb8(&bytes).ok();
    verify_open_capture_cancellable(&file, path, expected, is_cancelled)?;
    Ok(decoded)
}

pub(super) struct ViewerSeekState {
    pub(super) epoch: u64,
    pub(super) requested_frame: Option<u64>,
}

pub(super) struct ViewerDisplayFrame {
    pub(super) generation: u64,
    pub(super) seek_epoch: u64,
    pub(super) pixels: SharedPixelBuffer<Rgb8Pixel>,
    pub(super) progress: f32,
    pub(super) position: String,
}

impl ViewerDisplayFrame {
    pub(super) fn is_current(&self, generation: u64, seek_epoch: u64) -> bool {
        self.generation == generation && self.seek_epoch == seek_epoch
    }
}

#[derive(Default)]
pub(super) struct ViewerDisplayState {
    pub(super) frame: Option<ViewerDisplayFrame>,
    pub(super) update_pending: bool,
}

/// Keeps the newest decoded playback frame while limiting the UI event queue to one update.
#[derive(Default)]
pub(super) struct ViewerDisplayMailbox(Mutex<ViewerDisplayState>);

impl ViewerDisplayMailbox {
    /// Returns true only when the caller needs to schedule an event-loop update.
    pub(super) fn publish(&self, frame: ViewerDisplayFrame) -> bool {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.frame = Some(frame);
        if state.update_pending {
            return false;
        }
        state.update_pending = true;
        true
    }

    pub(super) fn take_for_ui(&self) -> Option<ViewerDisplayFrame> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.update_pending = false;
        state.frame.take()
    }

    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .frame = None;
    }
}

pub(super) struct ViewerOpenRequest {
    pub(super) path: std::path::PathBuf,
    pub(super) generation: u64,
    pub(super) is_photo: bool,
    pub(super) expected_version: CaptureFileVersion,
}

#[derive(Default)]
pub(super) struct ViewerOpenState {
    pub(super) pending: Option<ViewerOpenRequest>,
    pub(super) closed: bool,
}

/// A single viewer worker handles the newest open request, bounding file I/O and decoding.
#[derive(Default)]
pub(super) struct ViewerOpenMailbox {
    pub(super) state: Mutex<ViewerOpenState>,
    pub(super) ready: Condvar,
}

impl ViewerOpenMailbox {
    pub(super) fn request(&self, request: ViewerOpenRequest) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.closed {
            state.pending = Some(request);
            self.ready.notify_one();
        }
    }

    pub(super) fn receive(&self) -> Option<ViewerOpenRequest> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.pending.is_none() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if state.closed {
            None
        } else {
            state.pending.take()
        }
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.pending = None;
        self.ready.notify_all();
    }
}

pub(super) fn is_de400(descriptor: &CameraDescriptor) -> bool {
    descriptor
        .usb
        .as_ref()
        .is_some_and(|usb| usb.vendor_id == DE400_VENDOR_ID && usb.product_id == DE400_PRODUCT_ID)
}

pub(super) fn camera_error_status(kind: CameraErrorKind) -> &'static str {
    match kind {
        CameraErrorKind::PermissionDenied => "Accès caméra refusé",
        CameraErrorKind::DeviceBusy => "DE400 déjà utilisé",
        CameraErrorKind::BackendUnavailable | CameraErrorKind::Backend => "Caméra indisponible",
        CameraErrorKind::DeviceNotFound | CameraErrorKind::Disconnected => "DE400 non détecté",
        CameraErrorKind::TimedOut => "La caméra ne répond pas",
        CameraErrorKind::Unsupported | CameraErrorKind::InvalidConfiguration => {
            "Flux DE400 indisponible"
        }
        _ => "Erreur caméra",
    }
}

pub(super) fn show_capture_notice(
    win: &MainWindow,
    message: impl Into<slint::SharedString>,
    tone: i32,
) {
    let revision = win.get_capture_notice_revision().wrapping_add(1);
    win.set_capture_notice_revision(revision);
    win.set_capture_notice_tone(tone);
    win.set_last_capture_message(message.into());
    win.set_show_last_capture(true);
    let weak = win.as_weak();
    slint::Timer::single_shot(Duration::from_secs(4), move || {
        if let Some(win) = weak.upgrade()
            && win.get_capture_notice_revision() == revision
        {
            win.set_show_last_capture(false);
        }
    });
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn format_playback_time(seconds: f64) -> String {
    let total_seconds = seconds.max(0.0).floor() as u64;
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn video_time_seconds(frame_index: usize, fps: f64) -> f64 {
    if fps.is_finite() && fps > 0.0 {
        frame_index as f64 / fps
    } else {
        0.0
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn video_progress(frame_index: usize, frame_count: usize) -> f32 {
    if frame_count <= 1 {
        return 0.0;
    }

    (frame_index as f32 / (frame_count - 1) as f32) * 1_000.0
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub(super) fn video_frame_for_progress(progress: f32, frame_count: u64) -> u64 {
    if frame_count <= 1 {
        return 0;
    }

    let normalized = progress.clamp(0.0, 1_000.0) / 1_000.0;
    (normalized * frame_count.saturating_sub(1) as f32).round() as u64
}

pub(super) struct ViewerRuntime {
    pub(super) weak: slint::Weak<MainWindow>,
    pub(super) generation: Arc<AtomicU64>,
    pub(super) playing: Arc<AtomicBool>,
    pub(super) frame_count: Arc<AtomicU64>,
    pub(super) seek: Arc<Mutex<ViewerSeekState>>,
    pub(super) display: Arc<ViewerDisplayMailbox>,
}

impl ViewerRuntime {
    pub(super) fn run(&self, mailbox: &ViewerOpenMailbox) {
        while let Some(request) = mailbox.receive() {
            if self.generation.load(Ordering::Acquire) != request.generation {
                continue;
            }
            if request.is_photo {
                self.open_photo(&request);
            } else {
                self.open_video(&request);
            }
        }
    }

    pub(super) fn open_photo(&self, request: &ViewerOpenRequest) {
        let generation = request.generation;
        let Ok(decoded) =
            read_verified_photo_cancellable(&request.path, &request.expected_version, &|| {
                self.generation.load(Ordering::Acquire) != generation
            })
        else {
            self.schedule_changed_capture(generation);
            return;
        };
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let review = iriscope_core::library::load_photo_review(
            &request.path,
            &request.expected_version,
            &|| self.generation.load(Ordering::Acquire) != generation,
        );
        let pixels = decoded.map(|(width, height, rgb)| {
            SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height)
        });
        let generation_state = Arc::clone(&self.generation);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if generation_state.load(Ordering::Acquire) != generation {
                return;
            }
            // `decoded` is the verified snapshot read by the worker. The UI
            // only checks cancellation; filesystem hashing stays off this thread.
            if let Some(pixels) = pixels {
                viewer.set_viewer_image(slint::Image::from_rgb8(pixels));
                crate::photo_tools::loaded(&viewer, review);
                viewer.set_viewer_loading(false);
                viewer.set_viewer_is_video(false);
                viewer.set_viewer_video_playing(false);
                viewer.set_viewer_video_progress(0.0);
                viewer.set_viewer_video_position("00:00".into());
                viewer.set_viewer_video_duration("00:00".into());
                viewer.set_viewer_open(true);
            } else {
                viewer.set_viewer_loading(false);
                viewer.global::<AppState>().invoke_close_viewer();
                viewer.set_viewer_open(false);
                show_capture_notice(&viewer, "Image illisible ou indisponible.", NOTICE_ERROR);
            }
        });
    }

    #[allow(clippy::too_many_lines)] // One playback loop owns the verified handle and cancellation state.
    pub(super) fn open_video(&self, request: &ViewerOpenRequest) {
        let generation = request.generation;
        if !request.expected_version.supports_fast_validation() {
            self.schedule_slow_video_validation(generation);
            return;
        }
        let file = match open_verified_video(&request.path, &request.expected_version, || {
            self.generation.load(Ordering::Acquire) != generation
        }) {
            Ok(file) => file,
            Err(_) if self.generation.load(Ordering::Acquire) != generation => return,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.schedule_unsettled_video(generation);
                return;
            }
            Err(_) => {
                self.schedule_changed_capture(generation);
                return;
            }
        };
        let Ok(mut reader) = AviMjpegReader::from_file(file) else {
            if capture_path_matches_cancellable(&request.path, &request.expected_version, &|| {
                self.generation.load(Ordering::Acquire) != generation
            }) {
                self.schedule_video_open_error(generation);
            } else {
                self.schedule_changed_capture(generation);
            }
            return;
        };

        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        if !video_reader_matches(&reader, request) {
            self.schedule_changed_capture(generation);
            return;
        }
        let frame_count = reader.frame_count();
        let fps = reader.frame_rate().frames_per_second();
        let first_frame = reader.read_frame(0).ok().and_then(|jpeg| {
            let jpeg = ensure_jpeg_has_dht(&jpeg);
            decode_mjpeg_to_rgb8(&jpeg).ok()
        });
        if !video_reader_matches(&reader, request) {
            self.schedule_changed_capture(generation);
            return;
        }
        let Some(first_frame) = first_frame else {
            self.schedule_video_open_error(generation);
            return;
        };
        self.schedule_video_start(generation, frame_count, fps);
        let initial_seek_epoch = self
            .seek
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .epoch;
        self.publish_video_frame(ViewerDisplayFrame {
            generation,
            seek_epoch: initial_seek_epoch,
            pixels: SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                &first_frame.2,
                first_frame.0,
                first_frame.1,
            ),
            progress: 0.0,
            position: "00:00".to_owned(),
        });
        drop(first_frame);

        let frame_duration = Duration::from_secs_f64(1.0 / fps);
        let mut frame_index = 1 % frame_count;
        let mut frame_attempted = true;
        while self.generation.load(Ordering::Acquire) == generation {
            if !video_reader_matches(&reader, request) {
                self.schedule_changed_capture(generation);
                return;
            }
            let (requested_seek, seek_epoch) = {
                let mut seek = self
                    .seek
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (seek.requested_frame.take(), seek.epoch)
            };
            if let Some(requested_seek) = requested_seek {
                frame_index = usize::try_from(requested_seek)
                    .unwrap_or(usize::MAX)
                    .min(frame_count.saturating_sub(1));
            }

            let playing = self.playing.load(Ordering::Acquire);
            if !playing && requested_seek.is_none() && frame_attempted {
                thread::sleep(Duration::from_millis(20));
                continue;
            }

            let frame_started = Instant::now();
            let Ok(jpeg) = reader.read_frame(frame_index) else {
                if video_reader_matches(&reader, request) {
                    self.schedule_video_open_error(generation);
                } else {
                    self.schedule_changed_capture(generation);
                }
                return;
            };
            let jpeg = ensure_jpeg_has_dht(&jpeg);
            let decoded = decode_mjpeg_to_rgb8(&jpeg);
            if !video_reader_matches(&reader, request) {
                self.schedule_changed_capture(generation);
                return;
            }
            if let Ok((width, height, rgb)) = decoded {
                if self.generation.load(Ordering::Acquire) != generation {
                    break;
                }
                let frame = ViewerDisplayFrame {
                    generation,
                    seek_epoch,
                    pixels: SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height),
                    progress: video_progress(frame_index, frame_count),
                    position: format_playback_time(video_time_seconds(frame_index, fps)),
                };
                self.publish_video_frame(frame);
            }
            frame_attempted = true;

            if playing {
                frame_index = (frame_index + 1) % frame_count;
                let remaining = frame_duration.saturating_sub(frame_started.elapsed());
                // Bound the delay before a newly opened file can replace this playback.
                let deadline = Instant::now() + remaining;
                while self.generation.load(Ordering::Acquire) == generation {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    thread::sleep(remaining.min(Duration::from_millis(20)));
                }
            } else {
                thread::sleep(Duration::from_millis(20));
            }
        }

        self.schedule_video_finish(generation);
    }

    pub(super) fn schedule_video_open_error(&self, generation: u64) {
        self.playing.store(false, Ordering::Release);
        self.display.clear();
        let generation_state = Arc::clone(&self.generation);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if generation_state.load(Ordering::Acquire) == generation {
                viewer.global::<AppState>().invoke_close_viewer();
                viewer.set_viewer_loading(false);
                viewer.set_viewer_open(false);
                show_capture_notice(
                    &viewer,
                    "Vidéo AVI illisible ou non compatible.",
                    NOTICE_ERROR,
                );
            }
        });
    }

    fn schedule_changed_capture(&self, generation: u64) {
        self.playing.store(false, Ordering::Release);
        self.display.clear();
        let generation_state = Arc::clone(&self.generation);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if generation_state.load(Ordering::Acquire) == generation {
                close_changed_capture(&viewer);
            }
        });
    }

    fn schedule_slow_video_validation(&self, generation: u64) {
        let generation_state = Arc::clone(&self.generation);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if generation_state.load(Ordering::Acquire) == generation {
                viewer.global::<AppState>().invoke_close_viewer();
                viewer.set_viewer_open(false);
                show_capture_notice(&viewer, SLOW_VIDEO_VALIDATION_NOTICE, NOTICE_ERROR);
            }
        });
    }

    fn schedule_unsettled_video(&self, generation: u64) {
        let generation_state = Arc::clone(&self.generation);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if generation_state.load(Ordering::Acquire) == generation {
                viewer.global::<AppState>().invoke_close_viewer();
                viewer.set_viewer_open(false);
                viewer.set_viewer_loading(false);
                show_capture_notice(&viewer, UNSETTLED_VIDEO_NOTICE, NOTICE_ERROR);
                let weak = viewer.as_weak();
                slint::Timer::single_shot(Duration::from_millis(1_200), move || {
                    if let Some(viewer) = weak.upgrade() {
                        viewer.global::<AppState>().invoke_refresh_library();
                    }
                });
            }
        });
    }

    pub(super) fn schedule_video_start(&self, generation: u64, frame_count: usize, fps: f64) {
        let frame_count_u64 = u64::try_from(frame_count).unwrap_or(u64::MAX);
        let setup_generation = Arc::clone(&self.generation);
        let setup_playing = Arc::clone(&self.playing);
        let setup_frame_count = Arc::clone(&self.frame_count);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if setup_generation.load(Ordering::Acquire) != generation {
                return;
            }
            setup_frame_count.store(frame_count_u64, Ordering::Release);
            setup_playing.store(true, Ordering::Release);
            viewer.set_viewer_loading(false);
            viewer.set_viewer_is_video(true);
            viewer.set_viewer_video_playing(true);
            viewer.set_viewer_video_progress(0.0);
            viewer.set_viewer_video_position("00:00".into());
            viewer.set_viewer_video_duration(
                format_playback_time(video_time_seconds(frame_count, fps)).into(),
            );
            viewer.set_viewer_open(true);
        });
    }

    pub(super) fn publish_video_frame(&self, frame: ViewerDisplayFrame) {
        if !self.display.publish(frame) {
            return;
        }
        let ui_mailbox = Arc::clone(&self.display);
        let ui_generation = Arc::clone(&self.generation);
        let ui_seek = Arc::clone(&self.seek);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            let Some(frame) = ui_mailbox.take_for_ui() else {
                return;
            };
            let current_seek_epoch = ui_seek
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .epoch;
            if !frame.is_current(ui_generation.load(Ordering::Acquire), current_seek_epoch) {
                return;
            }
            // Pixels were verified in the worker before publication. Displaying
            // that immutable snapshot cannot load a replacement from the path.
            viewer.set_viewer_image(slint::Image::from_rgb8(frame.pixels));
            viewer.set_viewer_video_progress(frame.progress);
            viewer.set_viewer_video_position(frame.position.into());
        });
    }

    pub(super) fn schedule_video_finish(&self, generation: u64) {
        let finish_generation = Arc::clone(&self.generation);
        let finish_playing = Arc::clone(&self.playing);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if finish_generation.load(Ordering::Acquire) == generation {
                finish_playing.store(false, Ordering::Release);
                viewer.set_viewer_video_playing(false);
            }
        });
    }
}

fn video_reader_matches(reader: &AviMjpegReader, request: &ViewerOpenRequest) -> bool {
    request.expected_version.supports_fast_validation()
        && reader
            .file_version_fast()
            .is_ok_and(|version| version.same_native_metadata(&request.expected_version))
        && capture_file_version_fast(&request.path)
            .is_ok_and(|version| version.same_native_metadata(&request.expected_version))
}

#[cfg(test)]
mod playback_tests {
    use super::{
        ViewerDisplayFrame, ViewerDisplayMailbox, ViewerOpenMailbox, ViewerOpenRequest,
        format_playback_time, open_verified_capture, open_verified_video, read_verified_photo,
        verify_open_capture, video_frame_for_progress, video_progress, video_reader_matches,
        video_time_seconds,
    };
    use iriscope_core::{
        capabilities::FrameRate,
        library::{CaptureFileVersion, capture_file_version},
        video::{AviMjpegReader, AviMjpegWriter},
    };
    use std::{
        fs::{self, File, FileTimes},
        io::{Seek, SeekFrom, Write},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn fixture(label: &str) -> (PathBuf, PathBuf, CaptureFileVersion) {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-viewer-{label}-{id}"));
        fs::create_dir(&directory).expect("directory");
        let path = directory.join("capture.jpg");
        fs::write(&path, b"originaldata!").expect("original capture");
        let version = capture_file_version(&path).expect("displayed version");
        (directory, path, version)
    }

    pub(super) fn display_frame(position: &str) -> ViewerDisplayFrame {
        ViewerDisplayFrame {
            generation: 7,
            seek_epoch: 11,
            pixels: slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1, 1),
            progress: 0.0,
            position: position.to_owned(),
        }
    }

    #[test]
    pub(super) fn formats_video_time_for_short_and_long_clips() {
        assert_eq!(format_playback_time(65.9), "01:05");
        assert_eq!(format_playback_time(3_661.2), "01:01:01");
    }

    #[test]
    pub(super) fn preserves_slow_recording_timing() {
        assert!((video_time_seconds(2, 0.25) - 8.0).abs() < f64::EPSILON);
        assert_eq!(format_playback_time(video_time_seconds(3, 0.25)), "00:12");
    }

    #[test]
    pub(super) fn converts_timeline_progress_to_frame_index() {
        assert_eq!(video_frame_for_progress(0.0, 101), 0);
        assert_eq!(video_frame_for_progress(500.0, 101), 50);
        assert_eq!(video_frame_for_progress(1_000.0, 101), 100);
        assert_eq!(video_frame_for_progress(1_500.0, 101), 100);
    }

    #[test]
    pub(super) fn converts_frame_index_to_timeline_progress() {
        let progress = video_progress(50, 101);
        assert!((progress - 500.0).abs() < f32::EPSILON);
    }

    #[test]
    pub(super) fn playback_mailbox_keeps_latest_frame_with_one_pending_ui_update() {
        let mailbox = ViewerDisplayMailbox::default();
        assert!(mailbox.publish(display_frame("first")));
        assert!(!mailbox.publish(display_frame("latest")));
        assert_eq!(mailbox.take_for_ui().unwrap().position, "latest");

        assert!(mailbox.publish(display_frame("before close")));
        mailbox.clear();
        assert!(mailbox.take_for_ui().is_none());
        assert!(mailbox.publish(display_frame("after close")));
        assert_eq!(mailbox.take_for_ui().unwrap().position, "after close");
    }

    #[test]
    pub(super) fn playback_frame_expires_after_reopen_or_seek() {
        let frame = display_frame("frame");
        assert!(frame.is_current(7, 11));
        assert!(!frame.is_current(8, 11));
        assert!(!frame.is_current(7, 12));
    }

    #[test]
    pub(super) fn viewer_open_mailbox_keeps_latest_request_and_stops_cleanly() {
        let (directory, _, version) = fixture("mailbox");
        let mailbox = ViewerOpenMailbox::default();
        for generation in 1..=1_000 {
            mailbox.request(ViewerOpenRequest {
                path: format!("capture-{generation}.avi").into(),
                generation,
                is_photo: false,
                expected_version: version.clone(),
            });
        }
        let request = mailbox.receive().expect("latest request");
        assert_eq!(request.generation, 1_000);
        assert_eq!(request.path.to_string_lossy(), "capture-1000.avi");

        mailbox.request(ViewerOpenRequest {
            path: "stale.jpg".into(),
            generation: 1_001,
            is_photo: true,
            expected_version: version,
        });
        mailbox.close();
        assert!(mailbox.receive().is_none());
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn stale_card_cannot_open_a_capture_replaced_under_the_same_name() {
        let (directory, path, version) = fixture("replaced");
        let replacement = directory.join("replacement.jpg");
        fs::write(&replacement, b"originaldata!").expect("replacement has identical bytes");
        fs::remove_file(&path).expect("remove previous path");
        fs::rename(replacement, &path).expect("replace under old card name");
        assert!(open_verified_capture(&path, &version).is_err());
        assert!(read_verified_photo(&path, &version).is_err());
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn verified_handle_rejects_same_size_edits_even_if_mtime_is_restored() {
        let (directory, path, version) = fixture("edited");
        let reader = open_verified_capture(&path, &version).expect("original file opens");
        let old_modified = reader
            .metadata()
            .expect("metadata")
            .modified()
            .expect("mtime");
        let mut writer = File::options()
            .write(true)
            .open(&path)
            .expect("edit handle");
        writer.write_all(b"modifieddata!").expect("same-size edit");
        writer
            .set_times(FileTimes::new().set_modified(old_modified))
            .expect("restore mtime");
        drop(writer);
        assert!(verify_open_capture(&reader, &path, &version).is_err());
        drop(reader);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn card_path_replaced_with_symlink_is_rejected_even_when_target_is_original() {
        let (directory, path, version) = fixture("symlink");
        let moved = directory.join("original.jpg");
        fs::rename(&path, &moved).expect("move original");
        std::os::unix::fs::symlink(&moved, &path).expect("new symlink at card path");
        assert!(open_verified_capture(&path, &version).is_err());
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn verified_photo_reader_still_loads_the_displayed_image() {
        let (directory, path, _) = fixture("photo");
        let jpeg = iriscope_imaging::encode_rgb8_jpeg(&[15, 30, 45], 1, 1, 90).expect("jpeg");
        fs::write(&path, jpeg).expect("photo");
        let version = capture_file_version(&path).expect("displayed version");
        let (width, height, _) = read_verified_photo(&path, &version)
            .expect("verified read")
            .expect("decoded");
        assert_eq!((width, height), (1, 1));
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn playback_checks_detect_mutation_of_the_open_video_handle() {
        let (directory, _, _) = fixture("video");
        let path = directory.join("capture.avi");
        let jpeg = iriscope_imaging::encode_rgb8_jpeg(&[15, 30, 45], 1, 1, 90).expect("jpeg");
        let mut writer = AviMjpegWriter::create(&path, 1, 1, FrameRate::new(8, 1).expect("rate"))
            .expect("create AVI");
        writer.write_frame(&jpeg).expect("frame");
        writer.finish().expect("finish AVI");
        drop(writer);
        let version = capture_file_version(&path).expect("displayed version");
        let file = open_verified_video(&path, &version, || false)
            .expect("worker waits for stable metadata and verifies content");
        assert_eq!(
            capture_file_version(&path).expect("settled content version"),
            version,
            "waiting for metadata stability must not invalidate the card token"
        );
        let reader = AviMjpegReader::from_file(file).expect("verified AVI reader");
        let request = ViewerOpenRequest {
            path: path.clone(),
            generation: 1,
            is_photo: false,
            expected_version: version,
        };
        assert!(video_reader_matches(&reader, &request));
        let mut edit = File::options().write(true).open(&path).expect("edit AVI");
        edit.seek(SeekFrom::Start(2_058))
            .expect("JPEG payload offset");
        edit.write_all(&[0]).expect("mutate same open file");
        drop(edit);
        assert!(!video_reader_matches(&reader, &request));
        drop(reader);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn weak_video_version_is_rejected_before_opening_or_hashing_a_file() {
        let weak =
            CaptureFileVersion::from_token(r#"{"length":1}"#).expect("weak filesystem version");
        let error = open_verified_video(std::path::Path::new("missing-video.avi"), &weak, || false)
            .expect_err("weak video cannot start playback");
        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    }

    #[test]
    fn video_open_is_cancelled_before_a_frame_can_be_published() {
        let (directory, path, version) = fixture("cancelled-video");
        if version.supports_fast_validation() {
            let calls = std::cell::Cell::new(0);
            let error = open_verified_video(&path, &version, || {
                let next = calls.get() + 1;
                calls.set(next);
                next >= 3
            })
            .expect_err("superseded viewer request must stop during preparation");
            assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        }
        fs::remove_dir_all(directory).expect("cleanup");
    }
}
