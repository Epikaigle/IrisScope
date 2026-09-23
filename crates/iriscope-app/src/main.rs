use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use iriscope_core::{
    camera::{CameraDescriptor, CameraErrorKind, CameraEvent, CapturedFrame, StreamConfiguration},
    capabilities::{
        CameraCapabilities, CameraControlDescriptor, CameraControlId, CameraControlKind,
        CameraControlValue, FrameRate, PixelFormat,
    },
    capture::LatestFrame,
    library::{
        CaptureKind, LibraryFilter, present_library_items, record_capture_metadata,
        scan_library_directory,
    },
    session::{CaptureSession, Eye},
    settings::{AppSettings, PhysicalButtonBehavior},
    storage::{
        CaptureNamingPolicy, CaptureTimestamp, DEFAULT_FILENAME_TEMPLATE,
        filename_template_preserves_identity, save_new_capture,
    },
    video::{AviMjpegReader, AviMjpegWriter},
};
use iriscope_imaging::{
    convert_bgra8_to_rgb8, convert_nv12_to_rgb8, convert_yuyv_to_rgb8, decode_image_to_rgb8,
    decode_mjpeg_to_rgb8, encode_rgb8_jpeg, encode_rgb8_png, ensure_jpeg_has_dht,
    resize_rgb8_to_fit,
};
use slint::{ComponentHandle, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};

#[cfg(target_os = "linux")]
use iriscope_camera_linux as platform_camera;
#[cfg(target_os = "macos")]
use iriscope_camera_macos as platform_camera;
#[cfg(target_os = "windows")]
use iriscope_camera_windows as platform_camera;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("IrisScope currently supports Linux, Windows, and macOS");

slint::include_modules!();

const DE400_VENDOR_ID: u16 = 0x21cd;
const DE400_PRODUCT_ID: u16 = 0x603b;
const MAX_QUEUED_RECORDING_FRAMES: usize = 8;
const MAX_QUEUED_PHOTOS: usize = 2;
const MAX_THUMBNAIL_CACHE_BYTES: usize = 128 * 1024 * 1024;
const LIBRARY_PAGE_SIZE: usize = 100;
type DecodedFrame = (u32, u32, Vec<u8>);

struct ViewerSeekState {
    epoch: u64,
    requested_frame: Option<u64>,
}

struct ViewerDisplayFrame {
    generation: u64,
    seek_epoch: u64,
    pixels: DecodedFrame,
    progress: f32,
    position: String,
}

impl ViewerDisplayFrame {
    fn is_current(&self, generation: u64, seek_epoch: u64) -> bool {
        self.generation == generation && self.seek_epoch == seek_epoch
    }
}

#[derive(Default)]
struct ViewerDisplayState {
    frame: Option<ViewerDisplayFrame>,
    update_pending: bool,
}

/// Keeps the newest decoded playback frame while limiting the UI event queue to one update.
#[derive(Default)]
struct ViewerDisplayMailbox(Mutex<ViewerDisplayState>);

impl ViewerDisplayMailbox {
    /// Returns true only when the caller needs to schedule an event-loop update.
    fn publish(&self, frame: ViewerDisplayFrame) -> bool {
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

    fn take_for_ui(&self) -> Option<ViewerDisplayFrame> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.update_pending = false;
        state.frame.take()
    }

    fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .frame = None;
    }
}

struct ViewerOpenRequest {
    path: std::path::PathBuf,
    generation: u64,
    is_photo: bool,
}

#[derive(Default)]
struct ViewerOpenState {
    pending: Option<ViewerOpenRequest>,
    closed: bool,
}

/// A single viewer worker handles the newest open request, bounding file I/O and decoding.
#[derive(Default)]
struct ViewerOpenMailbox {
    state: Mutex<ViewerOpenState>,
    ready: Condvar,
}

impl ViewerOpenMailbox {
    fn request(&self, request: ViewerOpenRequest) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.closed {
            state.pending = Some(request);
            self.ready.notify_one();
        }
    }

    fn receive(&self) -> Option<ViewerOpenRequest> {
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

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.pending = None;
        self.ready.notify_all();
    }
}

struct PhotoRequest {
    frame: CapturedFrame,
    directory: std::path::PathBuf,
    filename_template: String,
    session: CaptureSession,
    timestamp: CaptureTimestamp,
    context_generation: u64,
}

#[derive(Default)]
struct PhotoMailbox {
    state: Mutex<PhotoMailboxState>,
    ready: Condvar,
}

#[derive(Default)]
struct PhotoMailboxState {
    pending: VecDeque<PhotoRequest>,
    closed: bool,
}

impl PhotoMailbox {
    fn enqueue(&self, request: PhotoRequest) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.pending.len() >= MAX_QUEUED_PHOTOS {
            return false;
        }
        state.pending.push_back(request);
        self.ready.notify_one();
        true
    }

    fn receive(&self) -> Option<PhotoRequest> {
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
        state.pending.pop_front()
    }

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        self.ready.notify_all();
    }
}

fn is_de400(descriptor: &CameraDescriptor) -> bool {
    descriptor
        .usb
        .as_ref()
        .is_some_and(|usb| usb.vendor_id == DE400_VENDOR_ID && usb.product_id == DE400_PRODUCT_ID)
}

fn camera_error_status(kind: CameraErrorKind) -> &'static str {
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

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_playback_time(seconds: f64) -> String {
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
fn video_time_seconds(frame_index: usize, fps: f64) -> f64 {
    frame_index as f64 / fps.max(0.5)
}

#[allow(clippy::cast_precision_loss)]
fn video_progress(frame_index: usize, frame_count: usize) -> f32 {
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
fn video_frame_for_progress(progress: f32, frame_count: u64) -> u64 {
    if frame_count <= 1 {
        return 0;
    }

    let normalized = progress.clamp(0.0, 1_000.0) / 1_000.0;
    (normalized * frame_count.saturating_sub(1) as f32).round() as u64
}

struct ViewerRuntime {
    weak: slint::Weak<MainWindow>,
    generation: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    frame_count: Arc<AtomicU64>,
    seek: Arc<Mutex<ViewerSeekState>>,
    display: Arc<ViewerDisplayMailbox>,
}

impl ViewerRuntime {
    fn run(&self, mailbox: &ViewerOpenMailbox) {
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

    fn open_photo(&self, request: &ViewerOpenRequest) {
        let generation = request.generation;
        let bytes = std::fs::read(&request.path).ok();
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let decoded = bytes.and_then(|bytes| decode_image_to_rgb8(&bytes).ok());
        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let generation_state = Arc::clone(&self.generation);
        let _ = self.weak.upgrade_in_event_loop(move |viewer| {
            if generation_state.load(Ordering::Acquire) != generation {
                return;
            }
            if let Some((width, height, rgb)) = decoded {
                let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
                viewer.set_viewer_image(slint::Image::from_rgb8(pixels));
                viewer.set_viewer_is_video(false);
                viewer.set_viewer_video_playing(false);
                viewer.set_viewer_video_progress(0.0);
                viewer.set_viewer_video_position("00:00".into());
                viewer.set_viewer_video_duration("00:00".into());
                viewer.set_viewer_open(true);
            } else {
                viewer.set_last_capture_message("Image illisible ou indisponible.".into());
                viewer.set_show_last_capture(true);
            }
        });
    }

    fn open_video(&self, request: &ViewerOpenRequest) {
        let generation = request.generation;
        let Ok(mut reader) = AviMjpegReader::open(&request.path) else {
            let generation_state = Arc::clone(&self.generation);
            let _ = self.weak.upgrade_in_event_loop(move |viewer| {
                if generation_state.load(Ordering::Acquire) == generation {
                    viewer
                        .set_last_capture_message("Vidéo AVI illisible ou non compatible.".into());
                    viewer.set_show_last_capture(true);
                }
            });
            return;
        };

        if self.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let frame_count = reader.frame_count();
        let fps = reader.frame_rate().frames_per_second().max(0.5);
        self.schedule_video_start(generation, frame_count, fps);

        let frame_duration = Duration::from_secs_f64(1.0 / fps);
        let mut frame_index = 0_usize;
        let mut frame_attempted = false;
        while self.generation.load(Ordering::Acquire) == generation {
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
                break;
            };
            let jpeg = ensure_jpeg_has_dht(&jpeg);

            if let Ok((width, height, rgb)) = decode_mjpeg_to_rgb8(&jpeg) {
                if self.generation.load(Ordering::Acquire) != generation {
                    break;
                }
                let frame = ViewerDisplayFrame {
                    generation,
                    seek_epoch,
                    pixels: (width, height, rgb),
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

    fn schedule_video_start(&self, generation: u64, frame_count: usize, fps: f64) {
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

    fn publish_video_frame(&self, frame: ViewerDisplayFrame) {
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
            let (width, height, rgb) = frame.pixels;
            let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
            viewer.set_viewer_image(slint::Image::from_rgb8(pixels));
            viewer.set_viewer_video_progress(frame.progress);
            viewer.set_viewer_video_position(frame.position.into());
        });
    }

    fn schedule_video_finish(&self, generation: u64) {
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

#[cfg(test)]
mod playback_tests {
    use super::{
        ViewerDisplayFrame, ViewerDisplayMailbox, ViewerOpenMailbox, ViewerOpenRequest,
        format_playback_time, video_frame_for_progress, video_progress,
    };

    fn display_frame(position: &str) -> ViewerDisplayFrame {
        ViewerDisplayFrame {
            generation: 7,
            seek_epoch: 11,
            pixels: (1, 1, vec![0, 0, 0]),
            progress: 0.0,
            position: position.to_owned(),
        }
    }

    #[test]
    fn formats_video_time_for_short_and_long_clips() {
        assert_eq!(format_playback_time(65.9), "01:05");
        assert_eq!(format_playback_time(3_661.2), "01:01:01");
    }

    #[test]
    fn converts_timeline_progress_to_frame_index() {
        assert_eq!(video_frame_for_progress(0.0, 101), 0);
        assert_eq!(video_frame_for_progress(500.0, 101), 50);
        assert_eq!(video_frame_for_progress(1_000.0, 101), 100);
        assert_eq!(video_frame_for_progress(1_500.0, 101), 100);
    }

    #[test]
    fn converts_frame_index_to_timeline_progress() {
        let progress = video_progress(50, 101);
        assert!((progress - 500.0).abs() < f32::EPSILON);
    }

    #[test]
    fn playback_mailbox_keeps_latest_frame_with_one_pending_ui_update() {
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
    fn playback_frame_expires_after_reopen_or_seek() {
        let frame = display_frame("frame");
        assert!(frame.is_current(7, 11));
        assert!(!frame.is_current(8, 11));
        assert!(!frame.is_current(7, 12));
    }

    #[test]
    fn viewer_open_mailbox_keeps_latest_request_and_stops_cleanly() {
        let mailbox = ViewerOpenMailbox::default();
        for generation in 1..=1_000 {
            mailbox.request(ViewerOpenRequest {
                path: format!("capture-{generation}.avi").into(),
                generation,
                is_photo: false,
            });
        }
        let request = mailbox.receive().expect("latest request");
        assert_eq!(request.generation, 1_000);
        assert_eq!(request.path.to_string_lossy(), "capture-1000.avi");

        mailbox.request(ViewerOpenRequest {
            path: "stale.jpg".into(),
            generation: 1_001,
            is_photo: true,
        });
        mailbox.close();
        assert!(mailbox.receive().is_none());
    }
}

#[derive(Default)]
struct ControlCommandMailbox {
    state: Mutex<ControlCommandState>,
}

#[derive(Default)]
struct ControlCommandState {
    updates: VecDeque<(CameraControlId, CameraControlValue)>,
    reset: bool,
    stop: bool,
}

struct ControlCommandBatch {
    updates: VecDeque<(CameraControlId, CameraControlValue)>,
    reset: bool,
    stop: bool,
}

impl ControlCommandMailbox {
    fn set_control(&self, id: CameraControlId, value: CameraControlValue) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.stop {
            if let Some(previous) = state.updates.iter().position(|(key, _)| *key == id) {
                state.updates.remove(previous);
            }
            state.updates.push_back((id, value));
        }
    }

    fn reset_controls(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.updates.clear();
        state.reset = true;
    }

    fn stop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stop = true;
        state.reset = false;
        state.updates.clear();
    }

    fn take(&self) -> ControlCommandBatch {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ControlCommandBatch {
            updates: std::mem::take(&mut state.updates),
            reset: std::mem::take(&mut state.reset),
            stop: state.stop,
        }
    }
}

#[cfg(test)]
mod control_command_tests {
    use iriscope_core::capabilities::{CameraControlId, CameraControlValue, StandardCameraControl};

    use super::ControlCommandMailbox;

    #[test]
    fn slider_burst_keeps_only_the_newest_value_and_stop_wins() {
        let mailbox = ControlCommandMailbox::default();
        let id = CameraControlId::Standard(StandardCameraControl::Brightness);
        for value in 0..10_000 {
            mailbox.set_control(id.clone(), CameraControlValue::Integer(value));
        }
        let batch = mailbox.take();
        assert_eq!(batch.updates.len(), 1);
        assert_eq!(
            batch.updates.front(),
            Some(&(id.clone(), CameraControlValue::Integer(9_999)))
        );

        mailbox.set_control(id.clone(), CameraControlValue::Integer(1));
        mailbox.reset_controls();
        mailbox.set_control(id.clone(), CameraControlValue::Integer(2));
        let batch = mailbox.take();
        assert!(batch.reset);
        assert_eq!(
            batch.updates.front(),
            Some(&(id.clone(), CameraControlValue::Integer(2)))
        );

        mailbox.set_control(id, CameraControlValue::Integer(3));
        mailbox.stop();
        let batch = mailbox.take();
        assert!(batch.stop);
        assert!(batch.updates.is_empty());
    }

    #[test]
    fn dependent_controls_keep_the_order_of_the_latest_changes() {
        let mailbox = ControlCommandMailbox::default();
        let automatic = CameraControlId::Standard(StandardCameraControl::ExposureMode);
        let manual = CameraControlId::Standard(StandardCameraControl::Exposure);
        mailbox.set_control(manual.clone(), CameraControlValue::Integer(20));
        mailbox.set_control(automatic.clone(), CameraControlValue::Menu(1));
        mailbox.set_control(manual.clone(), CameraControlValue::Integer(30));

        let updates = mailbox.take().updates.into_iter().collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                (automatic, CameraControlValue::Menu(1)),
                (manual, CameraControlValue::Integer(30))
            ]
        );
    }
}

struct RecordingRequest {
    directory: std::path::PathBuf,
    file_name: String,
    session: CaptureSession,
    timestamp: CaptureTimestamp,
    width: u32,
    height: u32,
    frame_rate: FrameRate,
}

#[derive(Clone, Copy)]
enum RecordingStopReason {
    User,
    Disconnected,
    Interrupted,
}

enum RecordingCommand {
    Start(u64, RecordingRequest),
    Frame(u64, CapturedFrame),
    Stop(u64, RecordingStopReason, u64),
}

#[derive(Default)]
struct RecordingMailbox {
    state: Mutex<RecordingMailboxState>,
    ready: Condvar,
}

#[derive(Default)]
struct RecordingMailboxState {
    commands: VecDeque<RecordingCommand>,
    active_generation: Option<u64>,
    next_generation: u64,
    queued_frames: usize,
    dropped_frames: u64,
    closed: bool,
}

impl RecordingMailbox {
    fn active_generation(&self) -> Option<u64> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active_generation
    }

    fn start(&self, request: RecordingRequest) -> Option<u64> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.active_generation.is_some() {
            return None;
        }
        state.next_generation = state.next_generation.saturating_add(1);
        let generation = state.next_generation;
        state.active_generation = Some(generation);
        state.dropped_frames = 0;
        state
            .commands
            .push_back(RecordingCommand::Start(generation, request));
        self.ready.notify_one();
        Some(generation)
    }

    fn publish_frame(&self, frame: CapturedFrame) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(generation) = state.active_generation else {
            return;
        };
        if state.queued_frames >= MAX_QUEUED_RECORDING_FRAMES {
            state.dropped_frames = state.dropped_frames.saturating_add(1);
            return;
        }
        state
            .commands
            .push_back(RecordingCommand::Frame(generation, frame));
        state.queued_frames += 1;
        self.ready.notify_one();
    }

    fn stop(&self, reason: RecordingStopReason) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(generation) = state.active_generation.take() else {
            return false;
        };
        let dropped = state.dropped_frames;
        state
            .commands
            .push_back(RecordingCommand::Stop(generation, reason, dropped));
        self.ready.notify_one();
        true
    }

    fn abort(&self, generation: u64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active_generation == Some(generation) {
            state.active_generation = None;
            return true;
        }
        false
    }

    fn receive(&self) -> Option<RecordingCommand> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(command) = state.commands.pop_front() {
                if matches!(command, RecordingCommand::Frame(..)) {
                    state.queued_frames -= 1;
                }
                return Some(command);
            }
            if state.closed {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(generation) = state.active_generation.take() {
            let dropped = state.dropped_frames;
            state.commands.push_back(RecordingCommand::Stop(
                generation,
                RecordingStopReason::Interrupted,
                dropped,
            ));
        }
        state.closed = true;
        self.ready.notify_all();
    }
}

#[derive(Debug, Default)]
struct DecodeMailbox {
    state: Mutex<DecodeMailboxState>,
    ready: Condvar,
}

#[derive(Debug, Default)]
struct DecodeMailboxState {
    frame: Option<QueuedDecodeFrame>,
    closed: bool,
}

#[derive(Debug)]
struct QueuedDecodeFrame {
    generation: u64,
    frame: CapturedFrame,
}

impl QueuedDecodeFrame {
    fn is_current(&self, generation: &AtomicU64) -> bool {
        self.generation == generation.load(Ordering::Acquire)
    }
}

impl DecodeMailbox {
    fn publish(&self, generation: u64, frame: CapturedFrame) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return false;
        }
        let replaced = state
            .frame
            .replace(QueuedDecodeFrame { generation, frame })
            .is_some();
        self.ready.notify_one();
        replaced
    }

    fn receive(&self) -> Option<QueuedDecodeFrame> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.frame.is_none() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.frame.take()
    }

    fn clear(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .frame = None;
    }

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.frame = None;
        self.ready.notify_all();
    }
}

fn settings_file_path() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    if let Ok(base) = std::env::var("APPDATA") {
        return std::path::PathBuf::from(base)
            .join("IrisScope")
            .join("settings.json");
    }

    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("IrisScope")
            .join("settings.json");
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(base) = std::env::var("XDG_CONFIG_HOME") {
            return std::path::PathBuf::from(base)
                .join("IrisScope")
                .join("settings.json");
        }
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home)
                .join(".config")
                .join("IrisScope")
                .join("settings.json");
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("iriscope-settings.json")
}

fn settings_snapshot(settings: &Arc<Mutex<AppSettings>>) -> AppSettings {
    settings
        .lock()
        .map_or_else(|_| AppSettings::default(), |guard| guard.clone())
}

fn persist_settings(settings: &Arc<Mutex<AppSettings>>, path: &std::path::Path) {
    if let Ok(guard) = settings.lock() {
        let _ = guard.save_to_file(path);
    }
}

fn physical_button_mode_index(behavior: PhysicalButtonBehavior) -> i32 {
    match behavior {
        PhysicalButtonBehavior::FollowMode => 0,
        PhysicalButtonBehavior::AlwaysPhoto => 1,
        PhysicalButtonBehavior::AlwaysVideo => 2,
    }
}

fn capture_session_from_window(win: &MainWindow) -> Result<CaptureSession, &'static str> {
    let first_name = win.get_patient_first_name().to_string();
    let last_name = win.get_patient_last_name().to_string();

    if first_name.trim().is_empty() || last_name.trim().is_empty() {
        return Err("Renseignez le prénom et le nom avant la capture.");
    }

    let eye = match win.get_selected_eye() {
        1 => Eye::Left,
        2 => Eye::Right,
        _ => return Err("Sélectionnez l'œil gauche ou droit avant la capture."),
    };

    Ok(CaptureSession::new(first_name, last_name, eye))
}

fn dispatch_hardware_button(win: &MainWindow, behavior: PhysicalButtonBehavior) {
    match behavior {
        PhysicalButtonBehavior::FollowMode => {
            if win.get_is_video_mode() {
                win.invoke_toggle_recording();
            } else {
                win.invoke_trigger_capture();
            }
        }
        PhysicalButtonBehavior::AlwaysPhoto => win.invoke_trigger_capture(),
        PhysicalButtonBehavior::AlwaysVideo => win.invoke_toggle_recording(),
    }
}

fn decode_camera_frame_to_rgb8(frame: &CapturedFrame) -> Option<(u32, u32, Vec<u8>)> {
    match frame.pixel_format {
        PixelFormat::Mjpeg => decode_mjpeg_to_rgb8(&frame.data).ok(),
        PixelFormat::Yuyv => {
            let rgb =
                convert_yuyv_to_rgb8(&frame.data, frame.resolution.width, frame.resolution.height);
            (!rgb.is_empty()).then_some((frame.resolution.width, frame.resolution.height, rgb))
        }
        PixelFormat::Bgra8 => {
            convert_bgra8_to_rgb8(&frame.data, frame.resolution.width, frame.resolution.height)
                .ok()
                .map(|rgb| (frame.resolution.width, frame.resolution.height, rgb))
        }
        PixelFormat::Nv12 => {
            convert_nv12_to_rgb8(&frame.data, frame.resolution.width, frame.resolution.height)
                .ok()
                .map(|rgb| (frame.resolution.width, frame.resolution.height, rgb))
        }
        _ => None,
    }
}

fn ranked_stream_configurations(capabilities: &CameraCapabilities) -> Vec<StreamConfiguration> {
    capabilities
        .ranked_modes()
        .into_iter()
        .map(|(mode, frame_rate)| StreamConfiguration {
            pixel_format: mode.pixel_format.clone(),
            resolution: mode.resolution,
            frame_rate,
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--diagnose") {
        run_diagnose();
        return Ok(());
    }

    run_gui()
}

#[allow(clippy::too_many_lines)]
fn run_diagnose() {
    let mut backend = platform_camera::create_backend();
    println!("IrisScope Camera Diagnostic ({})", backend.kind());

    let devices = match backend.enumerate_devices() {
        Ok(devs) => devs,
        Err(err) => {
            eprintln!("Failed to enumerate cameras: {err}");
            std::process::exit(1);
        }
    };

    if devices.is_empty() {
        eprintln!("No video capture device detected.");
        std::process::exit(1);
    }

    for (index, device) in devices.iter().enumerate() {
        println!("\nDevice #{index}: {}", device.display_name);
        println!("  ID: {}", device.id);
        if let Some(usb) = &device.usb {
            println!(
                "  USB: VID {:04x}, PID {:04x}",
                usb.vendor_id, usb.product_id
            );
            if let Some(serial) = &usb.serial_number {
                println!("  Serial: {serial}");
            }
            if let Some(rev) = &usb.hardware_revision {
                println!("  Revision: {rev}");
            }
        }
    }

    let Some(target) = devices.iter().find(|device| is_de400(device)) else {
        eprintln!("Firefly DE400 not detected.");
        std::process::exit(1);
    };

    println!("\nOpening device: {} [{}]", target.display_name, target.id);
    let mut dev = match backend.open(&target.id) {
        Ok(dev) => dev,
        Err(err) => {
            eprintln!("Failed to open camera: {err}");
            std::process::exit(1);
        }
    };

    let caps = dev.capabilities();
    println!("\nModes available:");
    for mode in &caps.modes {
        let fps_list: Vec<String> = mode
            .frame_rates
            .iter()
            .map(|f| format!("{:.2} fps", f.frames_per_second()))
            .collect();
        println!(
            "  Format: {}, Resolution: {}, Rates: [{}]",
            mode.pixel_format,
            mode.resolution,
            fps_list.join(", ")
        );
    }

    println!("\nControls available:");
    for ctrl in &caps.controls {
        println!(
            "  Control: {} ({})",
            ctrl.name,
            if ctrl.read_only {
                "Read-only"
            } else {
                "Read-Write"
            }
        );
    }

    let candidates = ranked_stream_configurations(caps);

    if candidates.is_empty() {
        eprintln!("No usable mode found.");
        std::process::exit(1);
    }

    let mut active_configuration = None;
    println!("\nTrying camera modes in preferred order...");
    for candidate in candidates {
        print!(
            "  {} at {} with {:.2} fps... ",
            candidate.pixel_format,
            candidate.resolution,
            candidate.frame_rate.frames_per_second()
        );

        match dev.start_stream(&candidate) {
            Ok(()) => {
                println!("OK");
                active_configuration = Some(dev.active_configuration().unwrap_or(candidate));
                break;
            }
            Err(error) => {
                println!("failed ({error})");
            }
        }
    }

    let Some(config) = active_configuration else {
        eprintln!("No advertised camera mode could be started.");
        std::process::exit(1);
    };

    println!(
        "Stream started successfully with {} at {} / {:.2} fps. Capturing 3 test frames...",
        config.pixel_format,
        config.resolution,
        config.frame_rate.frames_per_second()
    );
    let start_time = Instant::now();
    let mut captured = 0;
    let mut decoded = 0;
    while decoded < 3 && start_time.elapsed() < Duration::from_secs(5) {
        match dev.next_event(Duration::from_millis(1500)) {
            Ok(CameraEvent::Frame(frame)) => {
                captured += 1;
                println!(
                    "  Frame #{}: {} bytes (seq {}, timestamp {:?})",
                    captured,
                    frame.data.len(),
                    frame.sequence_number,
                    frame.timestamp
                );
                if let Some((width, height, rgb)) = decode_camera_frame_to_rgb8(&frame) {
                    decoded += 1;
                    println!(
                        "  ✓ Successfully decoded {} to RGB8: {width}×{height} ({} bytes)",
                        frame.pixel_format,
                        rgb.len()
                    );
                } else {
                    eprintln!(
                        "  ✗ No RGB8 diagnostic decoder is available for {}",
                        frame.pixel_format
                    );
                }
            }
            Ok(other) => {
                println!("  Event: {other:?}");
            }
            Err(err) => {
                eprintln!("  Capture error: {err}");
                break;
            }
        }
    }

    let elapsed = start_time.elapsed();
    let _ = dev.stop_stream();
    println!(
        "Stopped stream. Captured {captured} frames, decoded {decoded} in {:.2}s",
        elapsed.as_secs_f64()
    );
    if decoded < 3 {
        std::process::exit(1);
    }
}

fn load_full_image(path: &std::path::Path) -> Option<slint::Image> {
    let bytes = std::fs::read(path).ok()?;
    let (width, height, rgb) = decode_image_to_rgb8(&bytes).ok()?;
    let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
    Some(slint::Image::from_rgb8(pixels))
}

fn apply_reference_images(win: &MainWindow, settings: &AppSettings) {
    if let Some(path) = settings.iridology_map_path.as_deref()
        && let Some(image) = load_full_image(path)
    {
        win.set_iridology_map_image(image);
        win.set_has_iridology_map(true);
    } else {
        win.set_has_iridology_map(false);
    }

    if let Some(path) = settings.iridology_symbols_path.as_deref()
        && let Some(image) = load_full_image(path)
    {
        win.set_iridology_symbols_image(image);
        win.set_has_iridology_symbols(true);
    } else {
        win.set_has_iridology_symbols(false);
    }
}

fn load_thumbnail(path: &std::path::Path) -> Option<DecodedFrame> {
    let bytes = std::fs::read(path).ok()?;
    let (width, height, rgb) = decode_image_to_rgb8(&bytes).ok()?;
    resize_rgb8_to_fit(&rgb, width, height, 240).ok()
}

fn load_video_thumbnail(path: &std::path::Path) -> Option<DecodedFrame> {
    let mut reader = AviMjpegReader::open(path).ok()?;
    let jpeg = reader.read_frame(0).ok()?;
    let jpeg = ensure_jpeg_has_dht(&jpeg);
    let (width, height, rgb) = decode_mjpeg_to_rgb8(&jpeg).ok()?;
    resize_rgb8_to_fit(&rgb, width, height, 240).ok()
}

#[derive(Clone)]
struct LibraryItemPayload {
    date_time: String,
    eye_label: String,
    file_path: String,
    id: String,
    is_current_session: bool,
    is_video: bool,
    thumbnail: Option<SharedPixelBuffer<Rgb8Pixel>>,
    title: String,
}

struct CachedThumbnail {
    length: u64,
    modified: Option<std::time::SystemTime>,
    image: SharedPixelBuffer<Rgb8Pixel>,
    last_used: u64,
}

#[derive(Default)]
struct ThumbnailCache {
    entries: HashMap<std::path::PathBuf, CachedThumbnail>,
    bytes: usize,
    clock: u64,
}

impl ThumbnailCache {
    fn invalidate(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn get_or_load(
        &mut self,
        path: &std::path::Path,
        load: impl FnOnce(&std::path::Path) -> Option<DecodedFrame>,
    ) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
        let metadata = std::fs::metadata(path).ok()?;
        let length = metadata.len();
        let modified = metadata.modified().ok();
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(path)
            && entry.length == length
            && entry.modified == modified
        {
            entry.last_used = self.clock;
            return Some(entry.image.clone());
        }
        self.remove(path);
        let (width, height, rgb) = load(path)?;
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
                    length,
                    modified,
                    image: image.clone(),
                    last_used: self.clock,
                },
            );
        }
        Some(image)
    }

    fn remove(&mut self, path: &std::path::Path) {
        if let Some(removed) = self.entries.remove(path) {
            self.bytes = self.bytes.saturating_sub(removed.image.as_bytes().len());
        }
    }

    fn retain_paths(&mut self, paths: &std::collections::HashSet<std::path::PathBuf>) {
        self.entries.retain(|path, _| paths.contains(path));
        self.bytes = self
            .entries
            .values()
            .map(|entry| entry.image.as_bytes().len())
            .sum();
    }
}

struct LibraryPagePayloads {
    items: Vec<LibraryItemPayload>,
    total: usize,
    page: usize,
}

fn load_library_payloads(
    dir: &std::path::Path,
    active_session: &CaptureSession,
    filter: i32,
    requested_page: usize,
    cache: &mut ThumbnailCache,
    is_current: impl Fn() -> bool,
) -> Option<LibraryPagePayloads> {
    let raw_entries = scan_library_directory(dir);
    if !is_current() {
        return None;
    }
    let presented = present_library_items(&raw_entries, active_session, LibraryFilter::All);
    let paths = presented
        .iter()
        .map(|item| item.file_path.clone())
        .collect();
    cache.retain_paths(&paths);
    let filtered = presented
        .into_iter()
        .filter(|item| match filter {
            1 => item.kind == CaptureKind::Photo,
            2 => item.kind == CaptureKind::Video,
            3 => item.is_current_session,
            _ => true,
        })
        .collect::<Vec<_>>();
    let total = filtered.len();
    let page = requested_page.min(total.saturating_sub(1) / LIBRARY_PAGE_SIZE);
    let start = page * LIBRARY_PAGE_SIZE;
    let mut payloads = Vec::with_capacity(total.saturating_sub(start).min(LIBRARY_PAGE_SIZE));
    for (idx, item) in filtered
        .into_iter()
        .skip(start)
        .take(LIBRARY_PAGE_SIZE)
        .enumerate()
    {
        if !is_current() {
            return None;
        }
        let thumbnail = match item.kind {
            CaptureKind::Photo => cache.get_or_load(&item.file_path, load_thumbnail),
            CaptureKind::Video => cache.get_or_load(&item.file_path, load_video_thumbnail),
        };

        payloads.push(LibraryItemPayload {
            date_time: item.date_time,
            eye_label: item.eye_label,
            file_path: item.file_path.to_string_lossy().to_string(),
            id: (start + idx).to_string(),
            is_current_session: item.is_current_session,
            is_video: matches!(item.kind, CaptureKind::Video),
            thumbnail,
            title: item.display_title,
        });
    }
    Some(LibraryPagePayloads {
        items: payloads,
        total,
        page,
    })
}

fn present_library_payloads(payloads: Vec<LibraryItemPayload>) -> Vec<LibraryItemData> {
    payloads
        .into_iter()
        .map(|item| {
            let has_thumbnail = item.thumbnail.is_some();
            let thumbnail = item
                .thumbnail
                .map_or_else(slint::Image::default, slint::Image::from_rgb8);
            LibraryItemData {
                date_time: item.date_time.into(),
                eye_label: item.eye_label.into(),
                file_path: item.file_path.into(),
                has_thumbnail,
                id: item.id.into(),
                is_current_session: item.is_current_session,
                is_video: item.is_video,
                thumbnail,
                title: item.title.into(),
            }
        })
        .collect()
}

fn clear_library_view(win: &MainWindow) {
    win.set_library_items(ModelRc::new(VecModel::from(Vec::new())));
    win.set_library_total_count(0);
    win.set_library_page_summary("".into());
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

    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{PixelFormat, Resolution},
        library::scan_library_directory,
        session::{CaptureSession, Eye},
        storage::CaptureTimestamp,
    };

    use super::{
        LIBRARY_PAGE_SIZE, LibraryRefreshMailbox, PhotoMailbox, PhotoRequest, ThumbnailCache,
        load_library_payloads, save_photo,
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

    fn request(directory: &std::path::Path) -> PhotoRequest {
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
            session: CaptureSession::new("Ada", "Lovelace", Eye::Right),
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
    fn photo_worker_input_is_bounded_and_drains_on_close() {
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
    fn photo_save_writes_png_and_index_metadata() {
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
    fn thumbnail_cache_reuses_unchanged_file_and_invalidates_changed_file() {
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
    fn explicit_refresh_reloads_a_thumbnail_with_unchanged_file_metadata() {
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
    fn library_refresh_keeps_only_latest_request() {
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
    fn resolved_page_is_used_by_later_refreshes_but_stale_results_are_ignored() {
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
    fn only_explicit_refresh_invalidates_cached_thumbnails() {
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
    fn library_page_limits_loaded_items_and_keeps_later_captures_accessible() {
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

        let last = load_library_payloads(&directory.0, &session, 0, 99, &mut cache, || true)
            .expect("last page");
        assert_eq!(last.items.len(), 5);
        assert_eq!(last.page, 1);
    }
}

#[derive(Clone)]
struct CameraControlRuntimeState {
    key: String,
    descriptor: CameraControlDescriptor,
    value: CameraControlValue,
}

fn camera_control_key(id: &CameraControlId) -> String {
    match id {
        CameraControlId::Standard(control) => format!("standard:{control:?}"),
        CameraControlId::PlatformSpecific(name) => format!("platform:{name}"),
        CameraControlId::UvcExtension {
            unit,
            selector,
            guid,
        } => format!(
            "uvc:{unit}:{selector}:{}",
            guid.as_deref().unwrap_or_default()
        ),
        _ => format!("unknown:{id:?}"),
    }
}

fn default_camera_control_value(kind: &CameraControlKind) -> Option<CameraControlValue> {
    match kind {
        CameraControlKind::Integer { default, .. } => Some(CameraControlValue::Integer(*default)),
        CameraControlKind::Boolean { default } => Some(CameraControlValue::Boolean(*default)),
        CameraControlKind::Menu { default, .. } => Some(CameraControlValue::Menu(*default)),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn camera_control_ui_data(state: &CameraControlRuntimeState) -> CameraControlUiData {
    let mut data = CameraControlUiData {
        key: state.key.clone().into(),
        name: state.descriptor.name.clone().into(),
        kind: 0,
        minimum: 0.0,
        maximum: 1.0,
        value: 0.0,
        boolean_value: false,
        menu_label: "".into(),
        read_only: state.descriptor.read_only,
    };

    match (&state.descriptor.kind, &state.value) {
        (
            CameraControlKind::Integer {
                minimum,
                maximum,
                default,
                ..
            },
            CameraControlValue::Integer(value),
        ) => {
            data.kind = 0;
            data.minimum = *minimum as f32;
            data.maximum = *maximum as f32;
            data.value = *value as f32;
            if !data.value.is_finite() {
                data.value = *default as f32;
            }
        }
        (CameraControlKind::Boolean { default }, CameraControlValue::Boolean(value)) => {
            data.kind = 1;
            data.boolean_value = *value;
            if state.descriptor.read_only {
                data.boolean_value = *default;
            }
        }
        (CameraControlKind::Menu { items, default }, CameraControlValue::Menu(value)) => {
            data.kind = 2;
            let active = items
                .iter()
                .find(|item| item.value == *value)
                .or_else(|| items.iter().find(|item| item.value == *default));
            data.menu_label = active
                .map_or_else(|| value.to_string(), |item| item.label.clone())
                .into();
            data.value = *value as f32;
        }
        (kind, _) => {
            if let Some(fallback) = default_camera_control_value(kind) {
                return camera_control_ui_data(&CameraControlRuntimeState {
                    key: state.key.clone(),
                    descriptor: state.descriptor.clone(),
                    value: fallback,
                });
            }
            data.read_only = true;
        }
    }

    data
}

fn set_camera_control_model(win: &MainWindow, states: &[CameraControlRuntimeState]) {
    let rows = states
        .iter()
        .map(camera_control_ui_data)
        .collect::<Vec<_>>();
    win.set_camera_controls(ModelRc::new(VecModel::from(rows)));
}

#[allow(clippy::cast_possible_truncation)]
fn snap_integer_control_value(
    descriptor: &CameraControlDescriptor,
    requested: f32,
) -> Option<CameraControlValue> {
    let CameraControlKind::Integer {
        minimum,
        maximum,
        step,
        ..
    } = descriptor.kind
    else {
        return None;
    };

    let step = step.max(1);
    let requested = requested.round() as i64;
    let clamped = requested.clamp(minimum, maximum);
    let snapped = minimum + ((clamped - minimum) / step) * step;
    Some(CameraControlValue::Integer(snapped))
}

fn stop_recording(
    mailbox: &RecordingMailbox,
    recording_start: &Mutex<Option<Instant>>,
    reason: RecordingStopReason,
) -> bool {
    let stopped = mailbox.stop(reason);
    if stopped && let Ok(mut start) = recording_start.lock() {
        *start = None;
    }
    stopped
}

fn measured_recording_frame_rate(
    first: Option<Duration>,
    last: Option<Duration>,
    written_frames: u64,
) -> Option<FrameRate> {
    if written_frames < 2 {
        return None;
    }
    let elapsed_us = last?.checked_sub(first?)?.as_micros();
    if elapsed_us == 0 {
        return None;
    }
    let intervals = u128::from(written_frames - 1);
    let millifps = intervals
        .saturating_mul(1_000_000_000)
        .saturating_add(elapsed_us / 2)
        / elapsed_us;
    FrameRate::new(u32::try_from(millifps).ok()?, 1_000)
}

struct ActiveRecording {
    generation: u64,
    writer: AviMjpegWriter,
    saved_path: std::path::PathBuf,
    directory: std::path::PathBuf,
    session: CaptureSession,
    first_timestamp: Option<Duration>,
    last_timestamp: Option<Duration>,
    written_frames: u64,
    error: Option<String>,
}

struct LibraryRefreshRequest {
    directory: std::path::PathBuf,
    session: CaptureSession,
    filter: i32,
    page: usize,
    revision: u64,
    cache_revision: u64,
}

#[derive(Default)]
struct LibraryRefreshMailbox {
    state: Mutex<LibraryRefreshState>,
    ready: Condvar,
}

#[derive(Default)]
struct LibraryRefreshState {
    pending: Option<LibraryRefreshRequest>,
    filter: i32,
    page: usize,
    revision: u64,
    cache_revision: u64,
    closed: bool,
}

impl LibraryRefreshMailbox {
    fn invalidate_thumbnails(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cache_revision = state.cache_revision.wrapping_add(1);
    }

    fn set_filter(&self, filter: i32) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.filter = filter;
        state.page = 0;
    }

    fn set_page(&self, page: usize) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .page = page;
    }

    fn request(&self, directory: std::path::PathBuf, session: CaptureSession) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        state.revision = state.revision.wrapping_add(1);
        state.pending = Some(LibraryRefreshRequest {
            directory,
            session,
            filter: state.filter,
            page: state.page,
            revision: state.revision,
            cache_revision: state.cache_revision,
        });
        self.ready.notify_one();
    }

    fn receive(&self) -> Option<LibraryRefreshRequest> {
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
        state.pending.take()
    }

    fn is_current(&self, revision: u64) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !state.closed && state.revision == revision
    }

    fn set_resolved_page(&self, revision: u64, page: usize) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.revision != revision {
            return false;
        }
        state.page = page;
        true
    }

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.pending = None;
        self.ready.notify_all();
    }
}

fn refresh_library_in_background(
    mailbox: &LibraryRefreshMailbox,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
) {
    let directory = settings_snapshot(settings).capture_directory;
    let session = active_session
        .lock()
        .map_or_else(|_| CaptureSession::default(), |guard| guard.clone());
    mailbox.request(directory, session);
}

fn run_library_worker(
    mailbox: &Arc<LibraryRefreshMailbox>,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
) {
    let mut cache = ThumbnailCache::default();
    let mut cached_directory = None;
    let mut cached_revision = 0;
    while let Some(request) = mailbox.receive() {
        if cached_directory.as_ref() != Some(&request.directory) {
            cache.invalidate();
            cached_directory = Some(request.directory.clone());
        }
        if cached_revision != request.cache_revision {
            cache.invalidate();
            cached_revision = request.cache_revision;
        }
        let Some(result) = load_library_payloads(
            &request.directory,
            &request.session,
            request.filter,
            request.page,
            &mut cache,
            || mailbox.is_current(request.revision),
        ) else {
            continue;
        };
        if !mailbox.is_current(request.revision) {
            continue;
        }
        let mailbox_for_ui = Arc::clone(mailbox);
        let settings_for_ui = Arc::clone(settings);
        let session_for_ui = Arc::clone(active_session);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if win.get_library_filter() != request.filter
                || !library_context_matches(
                    &settings_for_ui,
                    &session_for_ui,
                    &request.directory,
                    &request.session,
                )
                || !mailbox_for_ui.set_resolved_page(request.revision, result.page)
            {
                return;
            }
            let first = result.page * LIBRARY_PAGE_SIZE + usize::from(result.total > 0);
            let last = ((result.page + 1) * LIBRARY_PAGE_SIZE).min(result.total);
            let summary = format!("{first}–{last} sur {}", result.total);
            win.set_library_items(ModelRc::new(VecModel::from(present_library_payloads(
                result.items,
            ))));
            win.set_library_total_count(i32::try_from(result.total).unwrap_or(i32::MAX));
            win.set_library_page(i32::try_from(result.page).unwrap_or(i32::MAX));
            win.set_library_page_summary(summary.into());
            win.set_library_loading(false);
        });
    }
}

fn library_context_matches(
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    directory: &std::path::Path,
    session: &CaptureSession,
) -> bool {
    settings_snapshot(settings).capture_directory == directory
        && active_session
            .lock()
            .is_ok_and(|current| *current == *session)
}

fn recording_context_matches(
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    directory: &std::path::Path,
    recorded_session: &CaptureSession,
    form_session: Option<&CaptureSession>,
) -> bool {
    form_session.is_some_and(|current| current == recorded_session)
        && library_context_matches(settings, active_session, directory, recorded_session)
}

struct SavedPhoto {
    path: std::path::PathBuf,
    thumbnail: Option<SharedPixelBuffer<Rgb8Pixel>>,
}

fn save_photo(request: &PhotoRequest) -> Result<SavedPhoto, String> {
    let (extension, bytes) = match request.frame.pixel_format {
        PixelFormat::Mjpeg => ("jpg", ensure_jpeg_has_dht(&request.frame.data).into_owned()),
        PixelFormat::Yuyv | PixelFormat::Bgra8 | PixelFormat::Nv12 => {
            let (width, height, rgb) = decode_camera_frame_to_rgb8(&request.frame)
                .ok_or_else(|| "Erreur de conversion photo : image caméra invalide".to_owned())?;
            let png = encode_rgb8_png(&rgb, width, height)
                .map_err(|error| format!("Erreur de conversion photo : {error}"))?;
            ("png", png)
        }
        _ => return Err("Le format caméra actif ne peut pas être enregistré en photo.".to_owned()),
    };
    let policy = CaptureNamingPolicy::new(&request.filename_template);
    let file_name = policy.filename(&request.session, request.timestamp, extension);
    let path = save_new_capture(&request.directory, &file_name, &bytes)
        .map_err(|error| format!("Erreur d'enregistrement : {error}"))?;
    if let Err(error) = record_capture_metadata(
        &request.directory,
        &path,
        &request.session,
        CaptureKind::Photo,
        request.timestamp,
    ) {
        eprintln!("[IrisScope] Index bibliothèque non mis à jour : {error}");
    }
    let thumbnail = decode_image_to_rgb8(&bytes)
        .ok()
        .and_then(|(width, height, rgb)| resize_rgb8_to_fit(&rgb, width, height, 240).ok())
        .map(|(width, height, rgb)| {
            SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height)
        });
    Ok(SavedPhoto { path, thumbnail })
}

#[allow(clippy::too_many_arguments)]
fn run_photo_worker(
    mailbox: &PhotoMailbox,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    last_toast_time: &Arc<Mutex<Option<Instant>>>,
    library_mailbox: &LibraryRefreshMailbox,
    context_generation: &Arc<AtomicU64>,
) {
    while let Some(request) = mailbox.receive() {
        let saved = save_photo(&request);
        if saved.is_ok() {
            refresh_library_in_background(library_mailbox, settings, active_session);
        }
        let settings_for_ui = Arc::clone(settings);
        let session_for_ui = Arc::clone(active_session);
        let toast_for_ui = Arc::clone(last_toast_time);
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
                    let display_name = photo
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("photo");
                    if let Some(pixels) = photo.thumbnail {
                        win.set_last_capture_thumbnail(slint::Image::from_rgb8(pixels));
                        win.set_has_last_capture(true);
                        win.set_last_capture_file_name(display_name.into());
                        win.set_last_capture_path(photo.path.to_string_lossy().to_string().into());
                    }
                    win.set_last_capture_message(
                        format!("✓ Photo #{count} enregistrée : {display_name}").into(),
                    );
                }
                Err(error) => win.set_last_capture_message(error.into()),
            }
            win.set_show_last_capture(true);
            if let Ok(mut toast) = toast_for_ui.lock() {
                *toast = Some(Instant::now());
            }
        });
    }
}

#[allow(clippy::too_many_lines)]
fn run_recording_worker(
    mailbox: &Arc<RecordingMailbox>,
    library_mailbox: &LibraryRefreshMailbox,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    recording_start: &Arc<Mutex<Option<Instant>>>,
    last_toast_time: &Arc<Mutex<Option<Instant>>>,
) {
    let mut active: Option<ActiveRecording> = None;
    while let Some(command) = mailbox.receive() {
        match command {
            RecordingCommand::Start(generation, request) => {
                match AviMjpegWriter::create_unique(
                    &request.directory,
                    &request.file_name,
                    request.width,
                    request.height,
                    request.frame_rate,
                ) {
                    Ok((writer, saved_path)) => {
                        if let Err(error) = record_capture_metadata(
                            &request.directory,
                            &saved_path,
                            &request.session,
                            CaptureKind::Video,
                            request.timestamp,
                        ) {
                            eprintln!("[IrisScope] Index bibliothèque non mis à jour : {error}");
                        }
                        let path_for_ui = saved_path.clone();
                        let mailbox_for_ui = Arc::clone(mailbox);
                        let settings_for_ui = Arc::clone(settings);
                        let active_session_for_ui = Arc::clone(active_session);
                        let directory_for_ui = request.directory.clone();
                        let session_for_ui = request.session.clone();
                        let _ = weak.upgrade_in_event_loop(move |win| {
                            let form_session = capture_session_from_window(&win).ok();
                            if mailbox_for_ui.active_generation() == Some(generation)
                                && recording_context_matches(
                                    &settings_for_ui,
                                    &active_session_for_ui,
                                    &directory_for_ui,
                                    &session_for_ui,
                                    form_session.as_ref(),
                                )
                            {
                                win.set_last_capture_path(
                                    path_for_ui.to_string_lossy().to_string().into(),
                                );
                                if let Some(name) =
                                    path_for_ui.file_name().and_then(|value| value.to_str())
                                {
                                    win.set_last_capture_file_name(name.into());
                                }
                            }
                        });
                        active = Some(ActiveRecording {
                            generation,
                            writer,
                            saved_path,
                            directory: request.directory,
                            session: request.session,
                            first_timestamp: None,
                            last_timestamp: None,
                            written_frames: 0,
                            error: None,
                        });
                    }
                    Err(error) => {
                        mailbox.abort(generation);
                        let mailbox_for_ui = Arc::clone(mailbox);
                        let recording_start_for_ui = Arc::clone(recording_start);
                        let message = format!("Erreur vidéo : {error}");
                        let _ = weak.upgrade_in_event_loop(move |win| {
                            if mailbox_for_ui.active_generation().is_none() {
                                if let Ok(mut start) = recording_start_for_ui.lock() {
                                    *start = None;
                                }
                                win.set_is_recording(false);
                                win.set_recording_duration("00:00".into());
                                win.set_last_capture_message(message.into());
                                win.set_show_last_capture(true);
                            }
                        });
                    }
                }
            }
            RecordingCommand::Frame(generation, frame) => {
                let Some(recording) = active
                    .as_mut()
                    .filter(|recording| recording.generation == generation)
                else {
                    continue;
                };
                if recording.error.is_some() {
                    continue;
                }
                let jpeg = match frame.pixel_format {
                    PixelFormat::Mjpeg => Ok(ensure_jpeg_has_dht(&frame.data)),
                    _ => decode_camera_frame_to_rgb8(&frame)
                        .ok_or_else(|| "format caméra non décodable".to_owned())
                        .and_then(|(width, height, rgb)| {
                            encode_rgb8_jpeg(&rgb, width, height, 95)
                                .map(std::borrow::Cow::Owned)
                                .map_err(|error| error.to_string())
                        }),
                };
                let result = jpeg.map_err(|error| error.to_string()).and_then(|jpeg| {
                    recording
                        .writer
                        .write_frame(jpeg.as_ref())
                        .map_err(|error| error.to_string())
                });
                match result {
                    Ok(()) => {
                        recording.first_timestamp.get_or_insert(frame.timestamp);
                        recording.last_timestamp = Some(frame.timestamp);
                        recording.written_frames = recording.written_frames.saturating_add(1);
                    }
                    Err(error) => recording.error = Some(error),
                }
            }
            RecordingCommand::Stop(generation, reason, dropped_frames) => {
                if active
                    .as_ref()
                    .is_none_or(|recording| recording.generation != generation)
                {
                    continue;
                }
                let mut recording = active.take().expect("matching active recording");
                if let Some(rate) = measured_recording_frame_rate(
                    recording.first_timestamp,
                    recording.last_timestamp,
                    recording.written_frames,
                ) {
                    recording.writer.set_frame_rate(rate);
                }
                let finish_result = recording.writer.finish();
                let result = recording
                    .error
                    .or_else(|| finish_result.err().map(|error| error.to_string()));
                let message = if let Some(error) = result {
                    format!("Erreur de finalisation vidéo : {error}")
                } else if recording.written_frames == 0 {
                    "Vidéo sans image enregistrée.".to_owned()
                } else {
                    let prefix = match reason {
                        RecordingStopReason::User => "✓ Vidéo enregistrée",
                        RecordingStopReason::Disconnected => {
                            "Vidéo arrêtée et finalisée après déconnexion caméra."
                        }
                        RecordingStopReason::Interrupted => {
                            "Vidéo arrêtée et finalisée après interruption caméra."
                        }
                    };
                    if dropped_frames == 0 {
                        prefix.to_owned()
                    } else {
                        format!("{prefix} ({dropped_frames} images ignorées)")
                    }
                };
                if let Ok(mut toast) = last_toast_time.lock() {
                    *toast = Some(Instant::now());
                }
                let mailbox_for_ui = Arc::clone(mailbox);
                let path_for_ui = recording.saved_path;
                let settings_for_ui = Arc::clone(settings);
                let active_session_for_ui = Arc::clone(active_session);
                let directory_for_ui = recording.directory;
                let session_for_ui = recording.session;
                let _ = weak.upgrade_in_event_loop(move |win| {
                    let form_session = capture_session_from_window(&win).ok();
                    if mailbox_for_ui.active_generation().is_none()
                        && recording_context_matches(
                            &settings_for_ui,
                            &active_session_for_ui,
                            &directory_for_ui,
                            &session_for_ui,
                            form_session.as_ref(),
                        )
                    {
                        win.set_is_recording(false);
                        win.set_recording_duration("00:00".into());
                        win.set_last_capture_path(path_for_ui.to_string_lossy().to_string().into());
                        if let Some(name) = path_for_ui.file_name().and_then(|value| value.to_str())
                        {
                            win.set_last_capture_file_name(name.into());
                        }
                        win.set_last_capture_message(message.into());
                        win.set_show_last_capture(true);
                    }
                });
                refresh_library_in_background(library_mailbox, settings, active_session);
            }
        }
    }
}

#[cfg(test)]
mod recording_pipeline_tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{FrameRate, PixelFormat, Resolution},
        session::{CaptureSession, Eye},
        settings::AppSettings,
        storage::CaptureTimestamp,
    };

    use super::{
        DecodeMailbox, RecordingCommand, RecordingMailbox, RecordingRequest, RecordingStopReason,
        decode_camera_frame_to_rgb8, library_context_matches, measured_recording_frame_rate,
        recording_context_matches,
    };

    fn frame(sequence_number: u64) -> CapturedFrame {
        CapturedFrame {
            sequence_number,
            timestamp: Duration::from_millis(sequence_number * 160),
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(640, 480),
            data: Arc::from([0xff, 0xd8, 0xff, 0xd9]),
        }
    }

    fn request() -> RecordingRequest {
        RecordingRequest {
            directory: std::env::temp_dir(),
            file_name: "unused.avi".to_owned(),
            session: CaptureSession::default(),
            timestamp: CaptureTimestamp::now(),
            width: 640,
            height: 480,
            frame_rate: FrameRate::new(30, 1).expect("valid rate"),
        }
    }

    #[test]
    fn recording_queue_bounds_frames_and_orders_stop_after_accepted_frames() {
        let mailbox = RecordingMailbox::default();
        let generation = mailbox.start(request()).expect("recording starts");
        for sequence in 0..10 {
            mailbox.publish_frame(frame(sequence));
        }
        assert!(mailbox.stop(RecordingStopReason::User));
        assert!(
            matches!(mailbox.receive(), Some(RecordingCommand::Start(id, _)) if id == generation)
        );
        for sequence in 0..8 {
            assert!(matches!(
                mailbox.receive(),
                Some(RecordingCommand::Frame(id, frame))
                    if id == generation && frame.sequence_number == sequence
            ));
        }
        assert!(matches!(
            mailbox.receive(),
            Some(RecordingCommand::Stop(id, RecordingStopReason::User, 2)) if id == generation
        ));
        assert!(mailbox.active_generation().is_none());
    }

    #[test]
    fn measured_rate_uses_written_frame_timestamps() {
        assert_eq!(
            measured_recording_frame_rate(
                Some(Duration::ZERO),
                Some(Duration::from_millis(2_240)),
                15,
            ),
            FrameRate::new(25, 4),
        );
        assert_eq!(
            measured_recording_frame_rate(Some(Duration::ZERO), Some(Duration::ZERO), 2),
            None,
        );
    }

    #[test]
    fn old_decode_generation_is_rejected_after_reconnect() {
        let mailbox = DecodeMailbox::default();
        let generation = std::sync::atomic::AtomicU64::new(1);
        assert!(!mailbox.publish(1, frame(1)));
        let in_flight = mailbox.receive().expect("first frame");
        generation.store(2, std::sync::atomic::Ordering::Release);
        mailbox.clear();
        assert!(!in_flight.is_current(&generation));
        assert!(!mailbox.publish(2, frame(2)));
        assert!(
            mailbox
                .receive()
                .expect("new frame")
                .is_current(&generation)
        );
    }

    #[test]
    fn old_library_result_cannot_replace_new_session_or_directory() {
        let settings = Arc::new(Mutex::new(AppSettings::default()));
        let session = Arc::new(Mutex::new(CaptureSession::new("A", "B", Eye::Left)));
        let directory = settings.lock().expect("settings").capture_directory.clone();
        let original_session = session.lock().expect("session").clone();
        assert!(library_context_matches(
            &settings,
            &session,
            &directory,
            &original_session,
        ));

        *session.lock().expect("session") = CaptureSession::default();
        assert!(!library_context_matches(
            &settings,
            &session,
            &directory,
            &original_session,
        ));

        *session.lock().expect("session") = original_session.clone();
        settings.lock().expect("settings").capture_directory = directory.join("another");
        assert!(!library_context_matches(
            &settings,
            &session,
            &directory,
            &original_session,
        ));
    }

    #[test]
    fn old_recording_result_cannot_replace_new_patient_capture() {
        let settings = Arc::new(Mutex::new(AppSettings::default()));
        let recorded_session = CaptureSession::new("A", "B", Eye::Left);
        let active_session = Arc::new(Mutex::new(recorded_session.clone()));
        let directory = settings.lock().expect("settings").capture_directory.clone();
        assert!(recording_context_matches(
            &settings,
            &active_session,
            &directory,
            &recorded_session,
            Some(&recorded_session),
        ));

        let new_patient = CaptureSession::new("C", "D", Eye::Right);
        assert!(!recording_context_matches(
            &settings,
            &active_session,
            &directory,
            &recorded_session,
            Some(&new_patient),
        ));
        *active_session.lock().expect("session") = new_patient.clone();
        assert!(!recording_context_matches(
            &settings,
            &active_session,
            &directory,
            &recorded_session,
            Some(&new_patient),
        ));
        assert!(!recording_context_matches(
            &settings,
            &active_session,
            &directory,
            &recorded_session,
            None,
        ));
    }

    #[test]
    fn truncated_yuyv_frame_is_not_published_for_preview() {
        let mut frame = frame(1);
        frame.pixel_format = PixelFormat::Yuyv;
        frame.data = Arc::from([0_u8, 0_u8]);
        assert!(decode_camera_frame_to_rgb8(&frame).is_none());
    }
}

#[allow(clippy::too_many_lines)]
fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let main_window = MainWindow::new()?;
    let settings_path = settings_file_path();
    let mut loaded_settings = AppSettings::load_from_file(&settings_path);
    if !filename_template_preserves_identity(&loaded_settings.filename_template) {
        DEFAULT_FILENAME_TEMPLATE.clone_into(&mut loaded_settings.filename_template);
    }
    let _ = loaded_settings.save_to_file(&settings_path);
    main_window.set_settings_capture_directory(
        loaded_settings
            .capture_directory
            .to_string_lossy()
            .to_string()
            .into(),
    );
    main_window.set_settings_filename_template(loaded_settings.filename_template.clone().into());
    main_window.set_settings_button_mode(physical_button_mode_index(
        loaded_settings.physical_button_behavior,
    ));
    main_window.set_settings_iridology_map_path(
        loaded_settings
            .iridology_map_path
            .as_deref()
            .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
            .into(),
    );
    main_window.set_settings_iridology_symbols_path(
        loaded_settings
            .iridology_symbols_path
            .as_deref()
            .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
            .into(),
    );
    apply_reference_images(&main_window, &loaded_settings);
    let settings = Arc::new(Mutex::new(loaded_settings));
    let latest_frame = Arc::new(LatestFrame::new());
    let decode_mailbox = Arc::new(DecodeMailbox::default());
    let stream_generation = Arc::new(AtomicU64::new(0));
    let latest_decoded_frame: Arc<Mutex<Option<(u64, DecodedFrame)>>> = Arc::new(Mutex::new(None));
    let decoded_frame_update_pending = Arc::new(AtomicBool::new(false));
    let preview_active = Arc::new(AtomicBool::new(true));
    let decode_time_micros = Arc::new(AtomicU64::new(0));
    let dropped_decode_frames = Arc::new(AtomicU64::new(0));
    let active_stream_configuration: Arc<Mutex<Option<StreamConfiguration>>> =
        Arc::new(Mutex::new(None));
    let recording_mailbox = Arc::new(RecordingMailbox::default());
    let photo_mailbox = Arc::new(PhotoMailbox::default());
    let library_mailbox = Arc::new(LibraryRefreshMailbox::default());
    let photo_context_generation = Arc::new(AtomicU64::new(0));
    let active_session = Arc::new(Mutex::new(CaptureSession::default()));
    let recording_start: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let last_toast_time: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let frozen_frame: Arc<Mutex<Option<CapturedFrame>>> = Arc::new(Mutex::new(None));
    let viewer_generation = Arc::new(AtomicU64::new(0));
    let viewer_video_playing = Arc::new(AtomicBool::new(false));
    let viewer_video_frame_count = Arc::new(AtomicU64::new(0));
    let viewer_video_seek_request = Arc::new(Mutex::new(ViewerSeekState {
        epoch: 0,
        requested_frame: None,
    }));
    let camera_controls: Arc<Mutex<Vec<CameraControlRuntimeState>>> =
        Arc::new(Mutex::new(Vec::new()));

    let control_commands = Arc::new(ControlCommandMailbox::default());

    let library_worker = thread::spawn({
        let mailbox = Arc::clone(&library_mailbox);
        let weak = main_window.as_weak();
        let settings = Arc::clone(&settings);
        let active_session = Arc::clone(&active_session);
        move || run_library_worker(&mailbox, &weak, &settings, &active_session)
    });

    let photo_worker = thread::spawn({
        let mailbox = Arc::clone(&photo_mailbox);
        let weak = main_window.as_weak();
        let settings = Arc::clone(&settings);
        let active_session = Arc::clone(&active_session);
        let last_toast_time = Arc::clone(&last_toast_time);
        let library_mailbox = Arc::clone(&library_mailbox);
        let context_generation = Arc::clone(&photo_context_generation);
        move || {
            run_photo_worker(
                &mailbox,
                &weak,
                &settings,
                &active_session,
                &last_toast_time,
                &library_mailbox,
                &context_generation,
            );
        }
    });

    let recorder_worker = thread::spawn({
        let mailbox = Arc::clone(&recording_mailbox);
        let library_mailbox = Arc::clone(&library_mailbox);
        let weak = main_window.as_weak();
        let settings = Arc::clone(&settings);
        let active_session = Arc::clone(&active_session);
        let recording_start = Arc::clone(&recording_start);
        let last_toast_time = Arc::clone(&last_toast_time);
        move || {
            run_recording_worker(
                &mailbox,
                &library_mailbox,
                &weak,
                &settings,
                &active_session,
                &recording_start,
                &last_toast_time,
            );
        }
    });

    // Decode away from the camera thread. The mailbox contains at most one frame,
    // so a slow decoder always skips ahead instead of increasing display latency.
    let decode_mailbox_worker = Arc::clone(&decode_mailbox);
    let latest_decoded_frame_worker = Arc::clone(&latest_decoded_frame);
    let decoded_frame_update_pending_worker = Arc::clone(&decoded_frame_update_pending);
    let decode_time_micros_worker = Arc::clone(&decode_time_micros);
    let stream_generation_decoder = Arc::clone(&stream_generation);
    let preview_active_decoder = Arc::clone(&preview_active);
    let decode_weak = main_window.as_weak();
    let decoder_worker = thread::spawn(move || {
        while let Some(job) = decode_mailbox_worker.receive() {
            if !job.is_current(&stream_generation_decoder)
                || !preview_active_decoder.load(Ordering::Acquire)
            {
                continue;
            }
            let decode_start = Instant::now();
            let decoded_frame = decode_camera_frame_to_rgb8(&job.frame);
            let elapsed_micros =
                u64::try_from(decode_start.elapsed().as_micros()).unwrap_or(u64::MAX);
            decode_time_micros_worker.store(elapsed_micros, Ordering::Relaxed);

            let Some(decoded_frame) = decoded_frame else {
                continue;
            };
            if !job.is_current(&stream_generation_decoder)
                || !preview_active_decoder.load(Ordering::Acquire)
            {
                continue;
            }
            if let Ok(mut latest) = latest_decoded_frame_worker.lock() {
                *latest = Some((job.generation, decoded_frame));
            }

            if decoded_frame_update_pending_worker.swap(true, Ordering::AcqRel) {
                continue;
            }

            let latest = Arc::clone(&latest_decoded_frame_worker);
            let pending = Arc::clone(&decoded_frame_update_pending_worker);
            let generation = Arc::clone(&stream_generation_decoder);
            let update_result = decode_weak.upgrade_in_event_loop(move |win| {
                // Clear the gate before taking the slot. A concurrent publisher can
                // queue one follow-up update, while the queue remains strictly bounded.
                pending.store(false, Ordering::Release);
                let decoded = latest.lock().ok().and_then(|mut slot| slot.take());
                if win.get_is_frozen() || win.get_current_tab() != 0 {
                    return;
                }
                let Some((job_generation, (width, height, raw_rgb))) = decoded else {
                    return;
                };
                if job_generation != generation.load(Ordering::Acquire) {
                    return;
                }

                let pixel_buffer =
                    SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&raw_rgb, width, height);
                if job_generation != generation.load(Ordering::Acquire) {
                    return;
                }
                win.set_live_frame(slint::Image::from_rgb8(pixel_buffer));
            });
            if update_result.is_err() {
                decoded_frame_update_pending_worker.store(false, Ordering::Release);
            }
        }
    });

    // Spawn camera capture worker thread
    let main_weak = main_window.as_weak();
    let latest_frame_clone = Arc::clone(&latest_frame);
    let decode_mailbox_camera = Arc::clone(&decode_mailbox);
    let preview_active_camera = Arc::clone(&preview_active);
    let latest_decoded_frame_camera = Arc::clone(&latest_decoded_frame);
    let stream_generation_camera = Arc::clone(&stream_generation);
    let decode_time_micros_camera = Arc::clone(&decode_time_micros);
    let dropped_decode_frames_camera = Arc::clone(&dropped_decode_frames);
    let recording_mailbox_camera = Arc::clone(&recording_mailbox);
    let rec_start_clone = Arc::clone(&recording_start);
    let toast_clone = Arc::clone(&last_toast_time);
    let active_stream_configuration_worker = Arc::clone(&active_stream_configuration);
    let camera_controls_worker = Arc::clone(&camera_controls);
    let control_commands_worker = Arc::clone(&control_commands);
    let settings_worker = Arc::clone(&settings);
    let frozen_frame_worker = Arc::clone(&frozen_frame);

    thread::spawn(move || {
        let mut backend = platform_camera::create_backend();
        loop {
            let devices = match backend.enumerate_devices() {
                Ok(devices) => devices,
                Err(error) => {
                    let status = camera_error_status(error.kind());
                    let _ = main_weak.upgrade_in_event_loop(move |win| {
                        win.set_camera_connected(false);
                        win.set_is_streaming(false);
                        win.set_status_text(status.into());
                    });
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            if devices.is_empty() {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let Some(target) = devices.iter().find(|device| is_de400(device)).cloned() else {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            };

            let usb_info = target.usb.as_ref().map_or_else(
                || "N/A".to_string(),
                |u| format!("{:04x}:{:04x}", u.vendor_id, u.product_id),
            );

            let mut device = match backend.open(&target.id) {
                Ok(device) => device,
                Err(error) => {
                    let status = camera_error_status(error.kind());
                    let _ = main_weak.upgrade_in_event_loop(move |win| {
                        win.set_camera_connected(false);
                        win.set_is_streaming(false);
                        win.set_status_text(status.into());
                    });
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let discovered_controls = device
                .capabilities()
                .controls
                .iter()
                .cloned()
                .filter_map(|descriptor| {
                    let fallback = default_camera_control_value(&descriptor.kind)?;
                    let value = device.control_value(&descriptor.id).unwrap_or(fallback);
                    Some(CameraControlRuntimeState {
                        key: camera_control_key(&descriptor.id),
                        descriptor,
                        value,
                    })
                })
                .collect::<Vec<_>>();
            if let Ok(mut controls) = camera_controls_worker.lock() {
                controls.clone_from(&discovered_controls);
            }

            let candidates = ranked_stream_configurations(device.capabilities());

            let mut active_config = None;
            for candidate in candidates {
                if device.start_stream(&candidate).is_ok() {
                    active_config = Some(device.active_configuration().unwrap_or(candidate));
                    break;
                }
            }

            let Some(config) = active_config else {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("Flux DE400 indisponible".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            let generation = stream_generation_camera.fetch_add(1, Ordering::AcqRel) + 1;
            let _ = latest_frame_clone.take();
            decode_mailbox_camera.clear();
            if let Ok(mut latest) = latest_decoded_frame_camera.lock() {
                *latest = None;
            }
            decode_time_micros_camera.store(0, Ordering::Relaxed);
            dropped_decode_frames_camera.store(0, Ordering::Relaxed);
            if let Ok(mut frozen) = frozen_frame_worker.lock() {
                *frozen = None;
            }
            if let Ok(mut active) = active_stream_configuration_worker.lock() {
                *active = Some(config.clone());
            }

            let _ = main_weak.upgrade_in_event_loop({
                let dev_name = target.display_name.clone();
                let dev_id = target.id.to_string();
                let backend_name = backend.kind().to_string();
                let usb_text = usb_info.clone();
                let format_text = config.pixel_format.to_string();
                let res_text = config.resolution.to_string();
                let fps_text = format!("{:.2} fps", config.frame_rate.frames_per_second());
                let controls = discovered_controls.clone();
                let ui_generation = Arc::clone(&stream_generation_camera);
                let preview_active_for_ui = Arc::clone(&preview_active_camera);

                move |win| {
                    if ui_generation.load(Ordering::Acquire) != generation {
                        return;
                    }
                    win.set_camera_connected(true);
                    win.set_is_streaming(false);
                    win.set_is_frozen(false);
                    preview_active_for_ui.store(win.get_current_tab() == 0, Ordering::Release);
                    win.set_live_frame(slint::Image::default());
                    win.set_status_text("DE400 connecté — démarrage du flux...".into());

                    let mut diag = win.get_diagnostics();
                    diag.device_name = dev_name.into();
                    diag.device_id = dev_id.into();
                    diag.backend_name = backend_name.into();
                    diag.usb_info = usb_text.into();
                    diag.active_format = format_text.into();
                    diag.active_resolution = res_text.into();
                    diag.active_fps = fps_text.into();
                    win.set_diagnostics(diag);
                    set_camera_control_model(&win, &controls);
                }
            });

            // Streaming loop
            let mut frame_count: u64 = 0;
            let mut last_stat_time = Instant::now();
            let mut stat_frames = 0;
            let mut stream_ready_announced = false;

            loop {
                // Multiple slider changes to the same control collapse to the
                // newest value, while shutdown remains highest priority.
                let commands = control_commands_worker.take();
                if commands.stop {
                    stop_recording(
                        &recording_mailbox_camera,
                        &rec_start_clone,
                        RecordingStopReason::Interrupted,
                    );
                    let _ = device.stop_stream();
                    return;
                }
                if commands.reset {
                    let _ = device.reset_controls();
                }
                for (id, value) in commands.updates {
                    let _ = device.set_control_value(&id, &value);
                }

                match device.next_event(Duration::from_millis(500)) {
                    Ok(CameraEvent::Frame(frame)) => {
                        frame_count = frame_count.saturating_add(1);
                        stat_frames += 1;

                        latest_frame_clone.publish(frame.clone());
                        if preview_active_camera.load(Ordering::Acquire)
                            && decode_mailbox_camera.publish(generation, frame.clone())
                        {
                            dropped_decode_frames_camera.fetch_add(1, Ordering::Relaxed);
                        }
                        recording_mailbox_camera.publish_frame(frame);

                        if !stream_ready_announced {
                            stream_ready_announced = true;
                            let _ = main_weak.upgrade_in_event_loop(|win| {
                                win.set_is_streaming(true);
                                win.set_status_text("DE400 Connecté".into());
                            });
                        }

                        // Update FPS & Timer metrics every 1s
                        if last_stat_time.elapsed() >= Duration::from_secs(1) {
                            let fps =
                                f64::from(stat_frames) / last_stat_time.elapsed().as_secs_f64();
                            let decode_ms = Duration::from_micros(
                                decode_time_micros_camera.load(Ordering::Relaxed),
                            )
                            .as_secs_f64()
                                * 1_000.0;
                            let dropped_frames =
                                dropped_decode_frames_camera.load(Ordering::Relaxed);
                            last_stat_time = Instant::now();
                            stat_frames = 0;

                            let rec_dur_str =
                                rec_start_clone.lock().ok().and_then(|g| *g).map(|st| {
                                    let el = st.elapsed().as_secs();
                                    format!("{:02}:{:02}", el / 60, el % 60)
                                });

                            let should_hide_toast = toast_clone
                                .lock()
                                .ok()
                                .and_then(|g| *g)
                                .is_some_and(|st| st.elapsed() >= Duration::from_secs(4));

                            if should_hide_toast && let Ok(mut g) = toast_clone.lock() {
                                *g = None;
                            }

                            let _ = main_weak.upgrade_in_event_loop(move |win| {
                                let mut diag = win.get_diagnostics();
                                diag.measured_fps = format!("{fps:.2} fps").into();
                                diag.decode_time_ms = format!("{decode_ms:.1} ms").into();
                                diag.frame_count = i32::try_from(frame_count).unwrap_or(i32::MAX);
                                diag.dropped_frames =
                                    i32::try_from(dropped_frames).unwrap_or(i32::MAX);
                                win.set_diagnostics(diag);

                                if let Some(dur) = rec_dur_str {
                                    win.set_recording_duration(dur.into());
                                }
                                if should_hide_toast {
                                    win.set_show_last_capture(false);
                                }
                            });
                        }
                    }
                    Ok(CameraEvent::HardwareButtonPressed) => {
                        let behavior = settings_snapshot(&settings_worker).physical_button_behavior;
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            dispatch_hardware_button(&win, behavior);
                        });
                    }
                    Ok(CameraEvent::Disconnected) => {
                        let disconnected_generation =
                            stream_generation_camera.fetch_add(1, Ordering::AcqRel) + 1;
                        decode_mailbox_camera.clear();
                        if let Ok(mut latest) = latest_decoded_frame_camera.lock() {
                            *latest = None;
                        }
                        stop_recording(
                            &recording_mailbox_camera,
                            &rec_start_clone,
                            RecordingStopReason::Disconnected,
                        );
                        let _ = device.stop_stream();
                        let _ = latest_frame_clone.take();
                        if let Ok(mut frozen) = frozen_frame_worker.lock() {
                            *frozen = None;
                        }
                        if let Ok(mut active) = active_stream_configuration_worker.lock() {
                            *active = None;
                        }
                        if let Ok(mut controls) = camera_controls_worker.lock() {
                            controls.clear();
                        }
                        let ui_generation = Arc::clone(&stream_generation_camera);
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            if ui_generation.load(Ordering::Acquire) != disconnected_generation {
                                return;
                            }
                            win.set_camera_connected(false);
                            win.set_is_streaming(false);
                            win.set_is_recording(false);
                            win.set_is_frozen(false);
                            win.set_live_frame(slint::Image::default());
                            win.set_recording_duration("00:00".into());
                            win.set_status_text(
                                "DE400 déconnecté — reconnexion en cours...".into(),
                            );
                            win.set_camera_controls(ModelRc::new(VecModel::from(Vec::new())));
                        });
                        break;
                    }
                    Err(error) if error.kind() == CameraErrorKind::TimedOut => {}
                    Err(error) => {
                        let interrupted_generation =
                            stream_generation_camera.fetch_add(1, Ordering::AcqRel) + 1;
                        decode_mailbox_camera.clear();
                        if let Ok(mut latest) = latest_decoded_frame_camera.lock() {
                            *latest = None;
                        }
                        let error_kind = error.kind();
                        stop_recording(
                            &recording_mailbox_camera,
                            &rec_start_clone,
                            RecordingStopReason::Interrupted,
                        );
                        let _ = device.stop_stream();
                        let _ = latest_frame_clone.take();
                        if let Ok(mut frozen) = frozen_frame_worker.lock() {
                            *frozen = None;
                        }
                        if let Ok(mut active) = active_stream_configuration_worker.lock() {
                            *active = None;
                        }
                        if let Ok(mut controls) = camera_controls_worker.lock() {
                            controls.clear();
                        }
                        let status = if matches!(
                            error_kind,
                            CameraErrorKind::Disconnected | CameraErrorKind::DeviceNotFound
                        ) {
                            "DE400 déconnecté — reconnexion en cours...".to_owned()
                        } else {
                            format!(
                                "{} — reconnexion en cours...",
                                camera_error_status(error_kind)
                            )
                        };
                        let ui_generation = Arc::clone(&stream_generation_camera);
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            if ui_generation.load(Ordering::Acquire) != interrupted_generation {
                                return;
                            }
                            win.set_camera_connected(false);
                            win.set_is_streaming(false);
                            win.set_is_recording(false);
                            win.set_is_frozen(false);
                            win.set_live_frame(slint::Image::default());
                            win.set_recording_duration("00:00".into());
                            win.set_status_text(status.into());
                            win.set_camera_controls(ModelRc::new(VecModel::from(Vec::new())));
                        });
                        thread::sleep(Duration::from_millis(100));
                        break;
                    }
                    Ok(_) => {}
                }
            }
        }
    });

    // Library loading and photo encoding happen on dedicated workers.
    main_window.set_library_loading(true);
    refresh_library_in_background(&library_mailbox, &settings, &active_session);

    let library_mailbox_filter = Arc::clone(&library_mailbox);
    let settings_filter = Arc::clone(&settings);
    let session_filter = Arc::clone(&active_session);
    let weak_library_filter = main_window.as_weak();
    main_window.on_library_filter_changed(move |filter| {
        let Some(win) = weak_library_filter.upgrade() else {
            return;
        };
        library_mailbox_filter.set_filter(filter);
        clear_library_view(&win);
        refresh_library_in_background(&library_mailbox_filter, &settings_filter, &session_filter);
    });

    let library_mailbox_page = Arc::clone(&library_mailbox);
    let settings_page = Arc::clone(&settings);
    let session_page = Arc::clone(&active_session);
    let weak_library_page = main_window.as_weak();
    main_window.on_library_page_changed(move |page| {
        let Some(win) = weak_library_page.upgrade() else {
            return;
        };
        library_mailbox_page.set_page(usize::try_from(page).unwrap_or(0));
        clear_library_view(&win);
        refresh_library_in_background(&library_mailbox_page, &settings_page, &session_page);
    });

    let session_changed_session = Arc::clone(&active_session);
    let session_changed_settings = Arc::clone(&settings);
    let session_changed_mailbox = Arc::clone(&library_mailbox);
    let session_changed_generation = Arc::clone(&photo_context_generation);
    let weak_session_changed = main_window.as_weak();
    main_window.on_session_changed(move || {
        let Some(win) = weak_session_changed.upgrade() else {
            return;
        };
        let eye = match win.get_selected_eye() {
            1 => Eye::Left,
            2 => Eye::Right,
            _ => Eye::Unspecified,
        };
        let session = CaptureSession::new(
            win.get_patient_first_name().to_string(),
            win.get_patient_last_name().to_string(),
            eye,
        );
        let changed = if let Ok(mut current) = session_changed_session.lock() {
            let changed = *current != session;
            *current = session;
            changed
        } else {
            false
        };
        if changed {
            session_changed_generation.fetch_add(1, Ordering::AcqRel);
            session_changed_mailbox.set_page(0);
            clear_library_view(&win);
            win.set_has_last_capture(false);
            win.set_last_capture_thumbnail(slint::Image::default());
            win.set_last_capture_path("".into());
            win.set_last_capture_file_name("".into());
            win.set_session_photo_count(0);
            refresh_library_in_background(
                &session_changed_mailbox,
                &session_changed_settings,
                &session_changed_session,
            );
        }
    });

    let weak = main_window.as_weak();
    let latest_frame_cap = Arc::clone(&latest_frame);
    let frozen_frame_cap = Arc::clone(&frozen_frame);
    let settings_cap = Arc::clone(&settings);
    let session_cap = Arc::clone(&active_session);
    let photo_mailbox_cap = Arc::clone(&photo_mailbox);
    let library_mailbox_cap = Arc::clone(&library_mailbox);
    let context_generation_cap = Arc::clone(&photo_context_generation);

    main_window.on_trigger_capture(move || {
        let Some(win) = weak.upgrade() else { return };
        let frame = if win.get_is_frozen() {
            frozen_frame_cap
                .lock()
                .ok()
                .and_then(|guard| guard.clone())
                .or_else(|| latest_frame_cap.snapshot())
        } else {
            latest_frame_cap.snapshot()
        };
        let Some(frame) = frame else {
            eprintln!("[IrisScope] Capture demandée mais aucune frame caméra disponible");
            return;
        };
        let session = match capture_session_from_window(&win) {
            Ok(session) => session,
            Err(message) => {
                win.set_last_capture_message(message.into());
                win.set_show_last_capture(true);
                return;
            }
        };
        let capture_settings = settings_snapshot(&settings_cap);
        let request = PhotoRequest {
            frame,
            directory: capture_settings.capture_directory,
            filename_template: capture_settings.filename_template,
            session: session.clone(),
            timestamp: CaptureTimestamp::now(),
            context_generation: context_generation_cap.load(Ordering::Acquire),
        };
        if !photo_mailbox_cap.enqueue(request) {
            win.set_last_capture_message(
                "Traitement photo en cours : réessayez dans un instant.".into(),
            );
            win.set_show_last_capture(true);
            return;
        }
        let changed = if let Ok(mut current) = session_cap.lock() {
            let changed = *current != session;
            *current = session;
            changed
        } else {
            false
        };
        if changed {
            clear_library_view(&win);
            refresh_library_in_background(&library_mailbox_cap, &settings_cap, &session_cap);
        }
        win.set_shutter_flash_opacity(0.85);
        slint::Timer::single_shot(Duration::from_millis(50), {
            let weak_flash = win.as_weak();
            move || {
                if let Some(window) = weak_flash.upgrade() {
                    window.set_shutter_flash_opacity(0.0);
                }
            }
        });
        thread::spawn(|| {
            let sound_path = "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga";
            if std::path::Path::new(sound_path).exists() {
                let _ = std::process::Command::new("paplay")
                    .arg(sound_path)
                    .status()
                    .or_else(|_| {
                        std::process::Command::new("pw-play")
                            .arg(sound_path)
                            .status()
                    });
            }
        });
    });

    // Recording callback
    let weak_rec = main_window.as_weak();
    let recording_mailbox_rec = Arc::clone(&recording_mailbox);
    let settings_rec = Arc::clone(&settings);
    let session_rec = Arc::clone(&active_session);
    let rec_start_rec = Arc::clone(&recording_start);
    let toast_rec = Arc::clone(&last_toast_time);
    let active_stream_configuration_rec = Arc::clone(&active_stream_configuration);
    let latest_frame_rec = Arc::clone(&latest_frame);

    main_window.on_toggle_recording(move || {
        let Some(win) = weak_rec.upgrade() else {
            return;
        };
        let currently_recording = recording_mailbox_rec.active_generation().is_some();
        let recording_settings = settings_snapshot(&settings_rec);

        if currently_recording {
            // Stop recording
            stop_recording(
                &recording_mailbox_rec,
                &rec_start_rec,
                RecordingStopReason::User,
            );
            win.set_is_recording(false);
            win.set_recording_duration("00:00".into());
            win.set_last_capture_message("Finalisation de la vidéo en cours...".into());
            win.set_show_last_capture(true);
            if let Ok(mut g) = toast_rec.lock() {
                *g = Some(Instant::now());
            }
        } else {
            // Start recording
            let session = match capture_session_from_window(&win) {
                Ok(session) => session,
                Err(message) => {
                    win.set_last_capture_message(message.into());
                    win.set_show_last_capture(true);
                    return;
                }
            };
            if let Ok(mut current_session) = session_rec.lock() {
                *current_session = session.clone();
            }

            let Some(configuration) = active_stream_configuration_rec
                .lock()
                .ok()
                .and_then(|active| active.clone())
            else {
                win.set_last_capture_message("Erreur vidéo : aucun flux caméra actif".into());
                win.set_show_last_capture(true);
                return;
            };

            let Some(current_frame) = latest_frame_rec.snapshot() else {
                win.set_last_capture_message(
                    "Erreur vidéo : aucune frame caméra disponible".into(),
                );
                win.set_show_last_capture(true);
                return;
            };
            let timestamp = CaptureTimestamp::now();
            let policy = CaptureNamingPolicy::new(&recording_settings.filename_template);
            let file_name = policy.filename(&session, timestamp, "avi");

            let request = RecordingRequest {
                directory: recording_settings.capture_directory,
                file_name,
                session,
                timestamp,
                width: current_frame.resolution.width,
                height: current_frame.resolution.height,
                frame_rate: configuration.frame_rate,
            };
            if recording_mailbox_rec.start(request).is_some() {
                if let Ok(mut g) = rec_start_rec.lock() {
                    *g = Some(Instant::now());
                }
                win.set_is_recording(true);
                win.set_recording_duration("00:00".into());
            }
        }
    });

    // Session clearing
    let weak_session = main_window.as_weak();
    let session_clear = Arc::clone(&active_session);
    let settings_clear = Arc::clone(&settings);
    let library_clear = Arc::clone(&library_mailbox);
    let photo_generation_clear = Arc::clone(&photo_context_generation);
    main_window.on_clear_session(move || {
        let Some(win) = weak_session.upgrade() else {
            return;
        };
        win.set_patient_first_name("".into());
        win.set_patient_last_name("".into());
        win.set_selected_eye(0);
        win.set_session_photo_count(0);
        win.set_has_last_capture(false);
        win.set_last_capture_thumbnail(slint::Image::default());
        win.set_last_capture_path("".into());
        win.set_last_capture_file_name("".into());
        win.set_show_last_capture(false);
        photo_generation_clear.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut s) = session_clear.lock() {
            s.clear();
        }
        win.set_library_filter(0);
        library_clear.set_filter(0);
        clear_library_view(&win);
        refresh_library_in_background(&library_clear, &settings_clear, &session_clear);
    });

    // Refresh library callback
    let settings_refresh = Arc::clone(&settings);
    let session_refresh = Arc::clone(&active_session);
    let library_refresh = Arc::clone(&library_mailbox);
    main_window.on_refresh_library(move || {
        library_refresh.invalidate_thumbnails();
        refresh_library_in_background(&library_refresh, &settings_refresh, &session_refresh);
    });

    let viewer_display_mailbox = Arc::new(ViewerDisplayMailbox::default());
    let viewer_open_mailbox = Arc::new(ViewerOpenMailbox::default());
    let _viewer_worker = thread::spawn({
        let mailbox = Arc::clone(&viewer_open_mailbox);
        let runtime = ViewerRuntime {
            weak: main_window.as_weak(),
            generation: Arc::clone(&viewer_generation),
            playing: Arc::clone(&viewer_video_playing),
            frame_count: Arc::clone(&viewer_video_frame_count),
            seek: Arc::clone(&viewer_video_seek_request),
            display: Arc::clone(&viewer_display_mailbox),
        };
        move || runtime.run(&mailbox)
    });

    // File reads, image decoding, and AVI indexing are serialized off the UI thread.
    let weak_viewer = main_window.as_weak();
    let viewer_generation_open = Arc::clone(&viewer_generation);
    let viewer_playing_open = Arc::clone(&viewer_video_playing);
    let viewer_frame_count_open = Arc::clone(&viewer_video_frame_count);
    let viewer_seek_open = Arc::clone(&viewer_video_seek_request);
    let viewer_display_open = Arc::clone(&viewer_display_mailbox);
    let viewer_mailbox_open = Arc::clone(&viewer_open_mailbox);
    main_window.on_open_capture_file(move |file_path_str| {
        let Some(win) = weak_viewer.upgrade() else {
            return;
        };
        let path = std::path::PathBuf::from(file_path_str.as_str());
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_photo = matches!(extension.as_str(), "jpg" | "jpeg" | "png");
        if !is_photo && extension != "avi" {
            win.set_last_capture_message(
                "Ce format vidéo n'est pas encore lisible dans IrisScope.".into(),
            );
            win.set_show_last_capture(true);
            return;
        }

        let generation = viewer_generation_open
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        viewer_playing_open.store(false, Ordering::Release);
        viewer_frame_count_open.store(0, Ordering::Release);
        viewer_display_open.clear();
        {
            let mut seek = viewer_seek_open
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            seek.epoch = seek.epoch.wrapping_add(1);
            seek.requested_frame = None;
        }
        viewer_mailbox_open.request(ViewerOpenRequest {
            path,
            generation,
            is_photo,
        });
    });

    let viewer_generation_close = Arc::clone(&viewer_generation);
    let viewer_playing_close = Arc::clone(&viewer_video_playing);
    let viewer_frame_count_close = Arc::clone(&viewer_video_frame_count);
    let viewer_seek_close = Arc::clone(&viewer_video_seek_request);
    let viewer_display_close = Arc::clone(&viewer_display_mailbox);
    let weak_close_viewer = main_window.as_weak();
    main_window.on_close_viewer(move || {
        viewer_generation_close.fetch_add(1, Ordering::AcqRel);
        viewer_playing_close.store(false, Ordering::Release);
        viewer_frame_count_close.store(0, Ordering::Release);
        viewer_display_close.clear();
        {
            let mut seek = viewer_seek_close
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            seek.epoch = seek.epoch.wrapping_add(1);
            seek.requested_frame = None;
        }
        if let Some(win) = weak_close_viewer.upgrade() {
            win.set_viewer_image(slint::Image::default());
            win.set_viewer_is_video(false);
            win.set_viewer_video_playing(false);
            win.set_viewer_video_progress(0.0);
            win.set_viewer_video_position("00:00".into());
            win.set_viewer_video_duration("00:00".into());
        }
    });

    let viewer_playing_toggle = Arc::clone(&viewer_video_playing);
    let weak_toggle_viewer = main_window.as_weak();
    main_window.on_toggle_viewer_video(move || {
        let playing = !viewer_playing_toggle.load(Ordering::Relaxed);
        viewer_playing_toggle.store(playing, Ordering::Relaxed);
        if let Some(win) = weak_toggle_viewer.upgrade() {
            win.set_viewer_video_playing(playing);
        }
    });

    let viewer_frame_count_seek = Arc::clone(&viewer_video_frame_count);
    let viewer_seek_request = Arc::clone(&viewer_video_seek_request);
    main_window.on_seek_viewer_video(move |progress| {
        let frame_count = viewer_frame_count_seek.load(Ordering::Relaxed);
        if frame_count == 0 {
            return;
        }

        let frame_index = video_frame_for_progress(progress, frame_count);
        let mut seek = viewer_seek_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seek.epoch = seek.epoch.wrapping_add(1);
        seek.requested_frame = Some(frame_index);
    });

    // Keep the capture stream running for photos and recordings while avoiding
    // decode work when its preview is hidden.
    let weak_preview = main_window.as_weak();
    let preview_active_tab = Arc::clone(&preview_active);
    let decode_mailbox_tab = Arc::clone(&decode_mailbox);
    let latest_decoded_frame_tab = Arc::clone(&latest_decoded_frame);
    let decode_time_micros_tab = Arc::clone(&decode_time_micros);
    main_window.on_preview_tab_changed(move |visible| {
        let frozen = weak_preview
            .upgrade()
            .is_some_and(|win| win.get_is_frozen());
        let active = visible && !frozen;
        preview_active_tab.store(active, Ordering::Release);
        if !active {
            decode_mailbox_tab.clear();
            if let Ok(mut frame) = latest_decoded_frame_tab.lock() {
                *frame = None;
            }
            decode_time_micros_tab.store(0, Ordering::Relaxed);
        }
    });

    // Freeze frame
    let weak_freeze = main_window.as_weak();
    let latest_frame_freeze = Arc::clone(&latest_frame);
    let frozen_frame_freeze = Arc::clone(&frozen_frame);
    let preview_active_freeze = Arc::clone(&preview_active);
    let decode_mailbox_freeze = Arc::clone(&decode_mailbox);
    let latest_decoded_frame_freeze = Arc::clone(&latest_decoded_frame);
    let decode_time_micros_freeze = Arc::clone(&decode_time_micros);
    main_window.on_toggle_freeze(move || {
        let Some(win) = weak_freeze.upgrade() else {
            return;
        };
        let was_frozen = win.get_is_frozen();
        let new_frozen = !was_frozen;
        win.set_is_frozen(new_frozen);
        preview_active_freeze.store(!new_frozen && win.get_current_tab() == 0, Ordering::Release);
        if new_frozen {
            decode_mailbox_freeze.clear();
            if let Ok(mut frame) = latest_decoded_frame_freeze.lock() {
                *frame = None;
            }
            decode_time_micros_freeze.store(0, Ordering::Relaxed);
            if let Ok(mut g) = frozen_frame_freeze.lock() {
                *g = latest_frame_freeze.snapshot();
            }
        } else if let Ok(mut g) = frozen_frame_freeze.lock() {
            *g = None;
        }
    });

    // Camera controls discovered dynamically from the active backend.
    let control_commands_value = Arc::clone(&control_commands);
    let controls_value = Arc::clone(&camera_controls);
    let weak_value = main_window.as_weak();
    main_window.on_set_camera_control_value(move |key, requested| {
        let Some(win) = weak_value.upgrade() else {
            return;
        };
        let Ok(mut controls) = controls_value.lock() else {
            return;
        };
        let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
            return;
        };
        if state.descriptor.read_only {
            return;
        }
        let Some(value) = snap_integer_control_value(&state.descriptor, requested) else {
            return;
        };
        state.value = value.clone();
        control_commands_value.set_control(state.descriptor.id.clone(), value);
        set_camera_control_model(&win, &controls);
    });

    let control_commands_bool = Arc::clone(&control_commands);
    let controls_bool = Arc::clone(&camera_controls);
    let weak_bool = main_window.as_weak();
    main_window.on_set_camera_control_bool(move |key, requested| {
        let Some(win) = weak_bool.upgrade() else {
            return;
        };
        let Ok(mut controls) = controls_bool.lock() else {
            return;
        };
        let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
            return;
        };
        if state.descriptor.read_only
            || !matches!(state.descriptor.kind, CameraControlKind::Boolean { .. })
        {
            return;
        }
        let value = CameraControlValue::Boolean(requested);
        state.value = value.clone();
        control_commands_bool.set_control(state.descriptor.id.clone(), value);
        set_camera_control_model(&win, &controls);
    });

    let control_commands_menu = Arc::clone(&control_commands);
    let controls_menu = Arc::clone(&camera_controls);
    let weak_menu = main_window.as_weak();
    main_window.on_cycle_camera_control_menu(move |key| {
        let Some(win) = weak_menu.upgrade() else {
            return;
        };
        let Ok(mut controls) = controls_menu.lock() else {
            return;
        };
        let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
            return;
        };
        if state.descriptor.read_only {
            return;
        }

        let CameraControlKind::Menu { items, .. } = &state.descriptor.kind else {
            return;
        };
        if items.is_empty() {
            return;
        }

        let current = match state.value {
            CameraControlValue::Menu(value) => value,
            _ => items[0].value,
        };
        let current_index = items
            .iter()
            .position(|item| item.value == current)
            .unwrap_or_default();
        let next = items[(current_index + 1) % items.len()].value;
        let value = CameraControlValue::Menu(next);
        state.value = value.clone();
        control_commands_menu.set_control(state.descriptor.id.clone(), value);
        set_camera_control_model(&win, &controls);
    });

    let control_commands_reset = Arc::clone(&control_commands);
    let controls_reset = Arc::clone(&camera_controls);
    let weak_reset = main_window.as_weak();
    main_window.on_reset_camera_controls(move || {
        let Some(win) = weak_reset.upgrade() else {
            return;
        };
        if let Ok(mut controls) = controls_reset.lock() {
            for state in controls.iter_mut() {
                if let Some(value) = default_camera_control_value(&state.descriptor.kind) {
                    state.value = value;
                }
            }
            set_camera_control_model(&win, &controls);
        }
        control_commands_reset.reset_controls();
    });

    // Persistent application settings
    let settings_directory = Arc::clone(&settings);
    let settings_path_directory = settings_path.clone();
    let session_directory = Arc::clone(&active_session);
    let library_directory = Arc::clone(&library_mailbox);
    let photo_generation_directory = Arc::clone(&photo_context_generation);
    let weak_directory = main_window.as_weak();
    main_window.on_update_capture_directory(move |value| {
        let Some(win) = weak_directory.upgrade() else {
            return;
        };
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let directory = std::path::PathBuf::from(value);
        let changed = settings_snapshot(&settings_directory).capture_directory != directory;
        if let Ok(mut guard) = settings_directory.lock() {
            guard.capture_directory.clone_from(&directory);
        }
        persist_settings(&settings_directory, &settings_path_directory);
        win.set_settings_capture_directory(directory.to_string_lossy().to_string().into());
        if changed {
            photo_generation_directory.fetch_add(1, Ordering::AcqRel);
            library_directory.set_page(0);
            clear_library_view(&win);
            win.set_has_last_capture(false);
            win.set_last_capture_thumbnail(slint::Image::default());
            win.set_last_capture_path("".into());
            win.set_last_capture_file_name("".into());
        }
        refresh_library_in_background(&library_directory, &settings_directory, &session_directory);
    });

    let settings_template = Arc::clone(&settings);
    let settings_path_template = settings_path.clone();
    let weak_template = main_window.as_weak();
    main_window.on_update_filename_template(move |value| {
        let Some(win) = weak_template.upgrade() else {
            return;
        };
        let value = value.trim();
        if !filename_template_preserves_identity(value) {
            let current = settings_snapshot(&settings_template).filename_template;
            win.set_settings_filename_template(current.into());
            win.set_last_capture_message(
                "Le modèle doit contenir {prenom}, {nom} et {oeil}.".into(),
            );
            win.set_show_last_capture(true);
            return;
        }
        if let Ok(mut guard) = settings_template.lock() {
            value.clone_into(&mut guard.filename_template);
        }
        persist_settings(&settings_template, &settings_path_template);
        win.set_settings_filename_template(value.into());
    });

    let settings_button = Arc::clone(&settings);
    let settings_path_button = settings_path.clone();
    let weak_button = main_window.as_weak();
    main_window.on_cycle_physical_button_mode(move || {
        let Some(win) = weak_button.upgrade() else {
            return;
        };
        let next = if let Ok(mut guard) = settings_button.lock() {
            guard.physical_button_behavior = match guard.physical_button_behavior {
                PhysicalButtonBehavior::FollowMode => PhysicalButtonBehavior::AlwaysPhoto,
                PhysicalButtonBehavior::AlwaysPhoto => PhysicalButtonBehavior::AlwaysVideo,
                PhysicalButtonBehavior::AlwaysVideo => PhysicalButtonBehavior::FollowMode,
            };
            guard.physical_button_behavior
        } else {
            PhysicalButtonBehavior::FollowMode
        };
        persist_settings(&settings_button, &settings_path_button);
        win.set_settings_button_mode(physical_button_mode_index(next));
    });

    let settings_map = Arc::clone(&settings);
    let settings_path_map = settings_path.clone();
    let weak_map = main_window.as_weak();
    main_window.on_update_iridology_map_path(move |value| {
        let Some(win) = weak_map.upgrade() else {
            return;
        };
        let value = value.trim();
        let path = (!value.is_empty()).then(|| std::path::PathBuf::from(value));
        if let Ok(mut guard) = settings_map.lock() {
            guard.iridology_map_path.clone_from(&path);
        }
        persist_settings(&settings_map, &settings_path_map);
        win.set_settings_iridology_map_path(
            path.as_deref()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
                .into(),
        );
        let snapshot = settings_snapshot(&settings_map);
        apply_reference_images(&win, &snapshot);
    });

    let settings_symbols = Arc::clone(&settings);
    let settings_path_symbols = settings_path.clone();
    let weak_symbols = main_window.as_weak();
    main_window.on_update_iridology_symbols_path(move |value| {
        let Some(win) = weak_symbols.upgrade() else {
            return;
        };
        let value = value.trim();
        let path = (!value.is_empty()).then(|| std::path::PathBuf::from(value));
        if let Ok(mut guard) = settings_symbols.lock() {
            guard.iridology_symbols_path.clone_from(&path);
        }
        persist_settings(&settings_symbols, &settings_path_symbols);
        win.set_settings_iridology_symbols_path(
            path.as_deref()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
                .into(),
        );
        let snapshot = settings_snapshot(&settings_symbols);
        apply_reference_images(&win, &snapshot);
    });

    // Capture directory opener
    let settings_open = Arc::clone(&settings);
    main_window.on_open_capture_directory(move || {
        let dir = settings_snapshot(&settings_open).capture_directory;
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&dir).spawn();
    });

    main_window.run()?;
    viewer_generation.fetch_add(1, Ordering::AcqRel);
    viewer_open_mailbox.close();
    control_commands.stop();
    recording_mailbox.close();
    decode_mailbox.close();
    photo_mailbox.close();
    let _ = decoder_worker.join();
    let _ = recorder_worker.join();
    let _ = photo_worker.join();
    library_mailbox.close();
    let _ = library_worker.join();
    Ok(())
}
