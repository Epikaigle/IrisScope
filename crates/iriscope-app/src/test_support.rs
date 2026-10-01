//! Small public harness for integration tests of the real Slint callbacks.
//!
//! It installs the same controllers as the application, but deliberately does
//! not start camera, disk, or playback workers.
use crate::gui::install_controllers;
use crate::recording_worker::RecordingRequest;
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::camera::CapturedFrame;
use iriscope_core::capabilities::{FrameRate, PixelFormat, Resolution};
use iriscope_core::session::{CaptureSession, Eye};
use iriscope_core::settings::AppSettings;
use iriscope_core::storage::CaptureTimestamp;
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// A window wired to production controllers, with no native workers running.
pub struct ControllerHarness {
    window: MainWindow,
    runtime: AppRuntime,
}

/// Data copied from a photo request queued by the capture callback.
#[derive(Debug, PartialEq, Eq)]
pub struct QueuedCapture {
    pub sequence_number: u64,
    pub directory: PathBuf,
    pub filename_template: String,
    pub first_name: String,
    pub last_name: String,
    pub patient_id: Option<u64>,
    pub eye: Eye,
    pub context_generation: u64,
}

/// Observable state after a settings callback.
#[derive(Debug, PartialEq, Eq)]
pub struct SettingsObservation {
    pub filename_template: String,
    pub displayed_filename_template: String,
    pub save_dirty: bool,
    pub save_generation: u64,
}

/// Observable state of the playback controller and the Slint window.
#[derive(Debug, PartialEq, Eq)]
pub struct ViewerObservation {
    pub generation: u64,
    pub playing: bool,
    pub frame_count: u64,
    pub seek_epoch: u64,
    pub requested_frame: Option<u64>,
    pub displayed_playing: bool,
}

impl ControllerHarness {
    /// Installs the production bindings without opening a camera or starting workers.
    ///
    /// # Errors
    ///
    /// Returns a Slint platform error when the test display cannot create a window.
    pub fn new(capture_directory: PathBuf) -> Result<Self, slint::PlatformError> {
        let window = MainWindow::new()?;
        let settings_path = capture_directory.join("test-settings.json");
        let settings = AppSettings {
            capture_directory,
            ..AppSettings::default()
        };
        let runtime = AppRuntime::new(settings, settings_path);
        install_controllers(&window, &runtime);
        Ok(Self { window, runtime })
    }

    /// Sets the patient fields exactly as the Slint form would expose them.
    pub fn set_patient(&self, first_name: &str, last_name: &str, patient_id: &str, eye: Eye) {
        self.window.set_patient_first_name(first_name.into());
        self.window.set_patient_last_name(last_name.into());
        self.window.set_patient_id(patient_id.into());
        self.window.set_selected_eye(match eye {
            Eye::Unspecified => 0,
            Eye::Left => 1,
            Eye::Right => 2,
        });
    }

    /// Publishes a tiny synthetic camera frame into the production latest-frame slot.
    pub fn publish_frame(&self, sequence_number: u64) {
        self.runtime.latest_frame.publish(CapturedFrame {
            sequence_number,
            timestamp: Duration::from_millis(sequence_number),
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(640, 480),
            data: Arc::from([0xff, 0xd8, 0xff, 0xd9]),
        });
    }

    /// Invokes the actual capture binding installed on the Slint global.
    pub fn trigger_capture(&self) {
        self.window.global::<AppState>().invoke_trigger_capture();
    }

    /// Reads the request without consuming it or waiting for a worker.
    #[must_use]
    pub fn queued_capture(&self) -> Option<QueuedCapture> {
        let state = self
            .runtime
            .photo_mailbox
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending.front().map(|request| QueuedCapture {
            sequence_number: request.frame.sequence_number,
            directory: request.directory.clone(),
            filename_template: request.filename_template.clone(),
            first_name: request.session.first_name().to_owned(),
            last_name: request.session.last_name().to_owned(),
            patient_id: request.session.patient_id(),
            eye: request.session.eye(),
            context_generation: request.context_generation,
        })
    }

    /// Invokes the production filename-template settings binding.
    pub fn update_filename_template(&self, value: &str) {
        self.window
            .global::<AppState>()
            .invoke_update_filename_template(value.into());
    }

    /// Reads the setting and the save mailbox without running its disk worker.
    #[must_use]
    pub fn settings_observation(&self) -> SettingsObservation {
        let filename_template = self
            .runtime
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .filename_template
            .clone();
        let state = self
            .runtime
            .camera_settings_save
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        SettingsObservation {
            filename_template,
            displayed_filename_template: self.window.get_settings_filename_template().to_string(),
            save_dirty: state.dirty,
            save_generation: state.generation,
        }
    }

    /// Seeds a playing viewer so its close callback has state to clear.
    pub fn seed_playing_viewer(&self) {
        self.runtime.viewer_generation.store(4, Ordering::Release);
        self.runtime
            .viewer_video_playing
            .store(true, Ordering::Release);
        self.runtime
            .viewer_video_frame_count
            .store(8, Ordering::Release);
        let mut seek = self
            .runtime
            .viewer_video_seek_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seek.epoch = 2;
        seek.requested_frame = Some(3);
        self.window.set_viewer_video_playing(true);
    }

    /// Invokes the production viewer close binding.
    pub fn close_viewer(&self) {
        self.window.global::<AppState>().invoke_close_viewer();
    }

    /// Reads the viewer state after the callback.
    #[must_use]
    pub fn viewer_observation(&self) -> ViewerObservation {
        let seek = self
            .runtime
            .viewer_video_seek_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ViewerObservation {
            generation: self.runtime.viewer_generation.load(Ordering::Acquire),
            playing: self.runtime.viewer_video_playing.load(Ordering::Acquire),
            frame_count: self
                .runtime
                .viewer_video_frame_count
                .load(Ordering::Acquire),
            seek_epoch: seek.epoch,
            requested_frame: seek.requested_frame,
            displayed_playing: self.window.get_viewer_video_playing(),
        }
    }

    /// Shows the test window so a real close request can be observed.
    ///
    /// # Errors
    ///
    /// Returns a Slint platform error if the test display cannot show the window.
    pub fn show_window(&self) -> Result<(), slint::PlatformError> {
        self.window.show()
    }

    /// Dispatches Slint's real window-close event through the installed close guard.
    ///
    /// # Errors
    ///
    /// Returns a Slint platform error if the event or window hide fails.
    pub fn request_window_close(&self) -> Result<(), slint::PlatformError> {
        self.window
            .window()
            .try_dispatch_event(slint::platform::WindowEvent::CloseRequested)
    }

    /// Whether Slint still considers the window visible.
    #[must_use]
    pub fn window_visible(&self) -> bool {
        self.window.window().is_visible()
    }

    /// Whether the production close guard marked the runtime as closing.
    #[must_use]
    pub fn is_closing(&self) -> bool {
        self.runtime.closing.load(Ordering::Acquire)
    }

    /// Number of photo requests still queued or being processed.
    #[must_use]
    pub fn photo_work_count(&self) -> usize {
        self.runtime.photo_mailbox.work_count()
    }

    /// Enqueues a recording start without launching the native recording worker.
    #[must_use]
    pub fn seed_recording(&self) -> u64 {
        let directory = self
            .runtime
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .capture_directory
            .clone();
        self.runtime
            .recording_mailbox
            .start(RecordingRequest {
                directory,
                file_name: "unused.avi".to_owned(),
                session: CaptureSession::new("Alice", "Martin", Eye::Left),
                timestamp: CaptureTimestamp::now(),
                width: 640,
                height: 480,
                frame_rate: FrameRate::new(30, 1).expect("valid test frame rate"),
            })
            .expect("recording mailbox accepts one test recording")
    }

    /// Whether the production mailbox still has a recording to finalize.
    #[must_use]
    pub fn recording_is_finalizing(&self) -> bool {
        self.runtime.recording_mailbox.is_finalizing()
    }

    /// Simulates completion of a recording after the close callback requested its stop.
    pub fn complete_recording_finalization(&self, generation: u64) {
        self.runtime
            .recording_mailbox
            .finish_finalization(generation);
    }

    /// Simulates completion of one queued photo without writing a test image.
    ///
    /// Call only after closing the mailbox, so receiving an empty queue cannot wait.
    #[must_use]
    pub fn complete_queued_photo_without_saving(&self) -> bool {
        if self.runtime.photo_mailbox.receive().is_none() {
            return false;
        }
        self.runtime.photo_mailbox.complete();
        true
    }
}

impl Drop for ControllerHarness {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}
