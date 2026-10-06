use crate::app_helpers::{capture_session_from_window, decode_camera_frame_to_rgb8};
use crate::config::{MAX_QUEUED_PHOTOS, NOTICE_ERROR, NOTICE_SUCCESS};
use crate::library_worker::{
    LibraryRefreshMailbox, recording_context_matches, refresh_library_in_background,
};
use crate::playback::show_capture_notice;
use crate::ui::MainWindow;
use iriscope_core::camera::CapturedFrame;
use iriscope_core::capabilities::PixelFormat;
use iriscope_core::library::{CaptureKind, save_indexed_capture};
use iriscope_core::session::CaptureSession;
use iriscope_core::settings::AppSettings;
use iriscope_core::storage::{CaptureNamingPolicy, CaptureTimestamp};
use iriscope_imaging::{
    decode_mjpeg_to_rgb8, encode_rgb8_png, ensure_jpeg_has_dht, resize_rgb8_to_fit,
};
use slint::{ComponentHandle, Rgb8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

pub(super) struct PhotoRequest {
    pub(super) frame: CapturedFrame,
    pub(super) directory: std::path::PathBuf,
    pub(super) filename_template: String,
    pub(super) session: CaptureSession,
    pub(super) timestamp: CaptureTimestamp,
    pub(super) context_generation: u64,
}

pub(super) struct PhotoMailbox {
    byte_budget: usize,
    pub(super) state: Mutex<PhotoMailboxState>,
    pub(super) ready: Condvar,
}

#[derive(Default)]
pub(super) struct PhotoMailboxState {
    pub(super) pending: VecDeque<PhotoRequest>,
    pending_bytes: usize,
    in_flight: usize,
    pub(super) closed: bool,
}

impl Default for PhotoMailbox {
    fn default() -> Self {
        Self::with_byte_budget(crate::config::MAX_PHOTO_QUEUE_BYTES)
    }
}

impl PhotoMailbox {
    pub(super) fn with_byte_budget(byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(PhotoMailboxState::default()),
            ready: Condvar::new(),
            byte_budget,
        }
    }

    pub(super) fn enqueue(&self, request: PhotoRequest) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed
            || state.pending.len() >= MAX_QUEUED_PHOTOS
            || request.frame.data.len() > self.byte_budget.saturating_sub(state.pending_bytes)
        {
            return false;
        }
        state.pending_bytes += request.frame.data.len();
        state.pending.push_back(request);
        self.ready.notify_one();
        true
    }

    pub(super) fn receive(&self) -> Option<PhotoRequest> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.pending.is_empty() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let request = state.pending.pop_front()?;
        state.pending_bytes = state.pending_bytes.saturating_sub(request.frame.data.len());
        state.in_flight += 1;
        Some(request)
    }

    pub(crate) fn work_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending.len() + state.in_flight
    }

    pub(crate) fn complete(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.in_flight = state.in_flight.saturating_sub(1);
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        self.ready.notify_all();
    }
}

pub(super) struct SavedPhoto {
    pub(super) path: std::path::PathBuf,
    pub(super) file_version: iriscope_core::library::CaptureFileVersion,
    pub(super) thumbnail: Option<SharedPixelBuffer<Rgb8Pixel>>,
    pub(super) metadata_warning: Option<String>,
}

pub(super) fn save_photo(request: &PhotoRequest) -> Result<SavedPhoto, String> {
    let (extension, bytes, decoded) = match request.frame.pixel_format {
        PixelFormat::Mjpeg => {
            let jpeg = ensure_jpeg_has_dht(&request.frame.data).into_owned();
            let decoded = decode_mjpeg_to_rgb8(&jpeg)
                .map_err(|error| format!("Image JPEG caméra invalide : {error}"))?;
            ("jpg", jpeg, decoded)
        }
        PixelFormat::Yuyv | PixelFormat::Bgra8 | PixelFormat::Nv12 => {
            let (width, height, rgb) = decode_camera_frame_to_rgb8(&request.frame)
                .ok_or_else(|| "Erreur de conversion photo : image caméra invalide".to_owned())?;
            let png = encode_rgb8_png(&rgb, width, height)
                .map_err(|error| format!("Erreur de conversion photo : {error}"))?;
            ("png", png, (width, height, rgb))
        }
        _ => return Err("Le format caméra actif ne peut pas être enregistré en photo.".to_owned()),
    };
    iriscope_core::disk_space::ensure_available_space(&request.directory, bytes.len() as u64)
        .map_err(|error| error.to_string())?;
    let policy = CaptureNamingPolicy::new(&request.filename_template);
    let file_name = policy.filename(&request.session, request.timestamp, extension);
    let committed = save_indexed_capture(
        &request.directory,
        &file_name,
        &bytes,
        &request.session,
        CaptureKind::Photo,
        request.timestamp,
    )
    .map_err(|error| format!("Erreur d'enregistrement : {error}"))?;
    let metadata_warning = committed.metadata_warning.map(|error| {
        format!("Photo enregistrée ; association au dossier en attente de reprise : {error}")
    });
    let (width, height, rgb) = decoded;
    let thumbnail =
        resize_rgb8_to_fit(&rgb, width, height, 240)
            .ok()
            .map(|(width, height, rgb)| {
                SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height)
            });
    Ok(SavedPhoto {
        path: committed.file_path,
        file_version: committed.file_version,
        thumbnail,
        metadata_warning,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_photo_worker(
    mailbox: &PhotoMailbox,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    library_mailbox: &LibraryRefreshMailbox,
    context_generation: &Arc<AtomicU64>,
) {
    while let Some(request) = mailbox.receive() {
        let _work_guard = PhotoWorkGuard(mailbox);
        let saved = save_photo(&request);
        if saved
            .as_ref()
            .is_ok_and(|photo| photo.metadata_warning.is_some())
        {
            library_mailbox.invalidate_thumbnails();
        }
        if saved.is_ok() {
            refresh_library_in_background(library_mailbox, settings, active_session);
        }
        let settings_for_ui = Arc::clone(settings);
        let session_for_ui = Arc::clone(active_session);
        let generation_for_ui = Arc::clone(context_generation);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if generation_for_ui.load(Ordering::Acquire) != request.context_generation
                || !recording_context_matches(
                    &settings_for_ui,
                    &session_for_ui,
                    &request.directory,
                    &request.session,
                    capture_session_from_window(&win).ok().as_ref(),
                )
            {
                return;
            }
            match saved {
                Ok(photo) => {
                    let count = win.get_session_photo_count() + 1;
                    win.set_session_photo_count(count);
                    let progress = win.global::<crate::ui::AppState>();
                    if win.get_selected_eye() == 1 {
                        progress.set_session_left_count(progress.get_session_left_count() + 1);
                    } else if win.get_selected_eye() == 2 {
                        progress.set_session_right_count(progress.get_session_right_count() + 1);
                    }
                    let display_name = photo
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("photo");
                    win.set_has_last_capture_thumbnail(photo.thumbnail.is_some());
                    if let Some(pixels) = photo.thumbnail {
                        win.set_last_capture_thumbnail(slint::Image::from_rgb8(pixels));
                    }
                    win.set_has_last_capture(true);
                    win.set_last_capture_file_name(display_name.into());
                    win.set_last_capture_path(photo.path.to_string_lossy().to_string().into());
                    win.set_last_capture_file_version(photo.file_version.token().into());
                    if let Some(warning) = photo.metadata_warning {
                        show_capture_notice(&win, warning, NOTICE_ERROR);
                    } else {
                        show_capture_notice(
                            &win,
                            format!("Photo #{count} enregistrée : {display_name}"),
                            NOTICE_SUCCESS,
                        );
                    }
                }
                Err(error) => show_capture_notice(&win, error, NOTICE_ERROR),
            }
        });
    }
}

struct PhotoWorkGuard<'a>(&'a PhotoMailbox);
impl Drop for PhotoWorkGuard<'_> {
    fn drop(&mut self) {
        self.0.complete();
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::{PhotoMailbox, PhotoRequest};
    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{PixelFormat, Resolution},
        session::CaptureSession,
        storage::CaptureTimestamp,
    };
    use std::time::Duration;
    fn request() -> PhotoRequest {
        PhotoRequest {
            frame: CapturedFrame {
                sequence_number: 1,
                timestamp: Duration::ZERO,
                pixel_format: PixelFormat::Mjpeg,
                resolution: Resolution::new(1, 1),
                data: vec![1_u8; 3].into(),
            },
            directory: "unused".into(),
            filename_template: "capture".into(),
            session: CaptureSession::default(),
            timestamp: CaptureTimestamp::now(),
            context_generation: 0,
        }
    }
    #[test]
    fn close_waits_for_pending_and_inflight_photos_without_discarding_them() {
        let mailbox = PhotoMailbox::with_byte_budget(8);
        assert!(mailbox.enqueue(request()));
        assert!(mailbox.enqueue(request()));
        assert_eq!(mailbox.work_count(), 2);
        let first = mailbox.receive().unwrap();
        assert_eq!(mailbox.work_count(), 2);
        mailbox.close();
        assert!(!mailbox.enqueue(request()));
        mailbox.complete();
        drop(first);
        assert_eq!(mailbox.work_count(), 1);
        assert!(mailbox.receive().is_some());
        assert_eq!(mailbox.work_count(), 1);
        mailbox.complete();
        assert_eq!(mailbox.work_count(), 0);
        assert!(mailbox.receive().is_none());
    }
}
