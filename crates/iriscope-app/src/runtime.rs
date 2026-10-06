use crate::app_helpers::settings_snapshot;
use crate::camera_controller;
use crate::camera_queue::{
    CameraSettingsSaveMailbox, ControlCommandMailbox, DecodeMailbox,
    run_camera_settings_save_worker,
};
use crate::controls_ui::CameraControlRuntimeState;
use crate::library_worker::{LibraryRefreshMailbox, run_library_worker};
use crate::patient_ui::{PatientSearchMailbox, run_patient_search_worker};
use crate::photo_worker::{PhotoMailbox, run_photo_worker};
use crate::playback::{ViewerDisplayMailbox, ViewerOpenMailbox, ViewerRuntime, ViewerSeekState};
use crate::recording_worker::{RecordingMailbox, run_recording_worker};
use crate::ui::MainWindow;
use iriscope_core::camera::{CapturedFrame, StreamConfiguration};
use iriscope_core::capture::LatestFrame;
use iriscope_core::session::CaptureSession;
use iriscope_core::settings::AppSettings;
use slint::ComponentHandle;
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type DecodedPreview = (u64, SharedPixelBuffer<Rgb8Pixel>);
type PreviewFrameSlot = Arc<Mutex<Option<DecodedPreview>>>;

#[derive(Default)]
pub(super) struct WorkerOwner {
    workers: Vec<(&'static str, thread::JoinHandle<()>)>,
}
impl WorkerOwner {
    fn add(&mut self, name: &'static str, worker: thread::JoinHandle<()>) {
        self.workers.push((name, worker));
    }
    fn finish(&mut self, budget: Duration) {
        let deadline = Instant::now() + budget;
        while self.workers.iter().any(|(_, worker)| !worker.is_finished())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(20));
        }
        for (name, worker) in self.workers.drain(..) {
            if worker.is_finished() {
                if worker.join().is_err() {
                    eprintln!("[IrisScope] Le worker {name} s'est arrêté anormalement.");
                }
            } else {
                eprintln!("[IrisScope] Budget d’arrêt dépassé pour le worker {name}.");
            }
        }
    }
}

pub(super) struct AppRuntime {
    pub(super) directory_generation: Arc<AtomicU64>,
    pub(super) closing: Arc<AtomicBool>,
    pub(super) background_jobs: Arc<crate::background::BackgroundJobs>,
    pub(super) storage_operation: Arc<crate::operation_progress::OperationControl>,
    pub(super) export_operation: Arc<crate::operation_progress::OperationControl>,
    pub(super) reference_mailbox: Arc<crate::reference_worker::ReferenceMailbox>,
    pub(super) settings: Arc<Mutex<AppSettings>>,
    pub(super) settings_path: std::path::PathBuf,
    pub(super) map_generation: Arc<AtomicU64>,
    pub(super) symbols_generation: Arc<AtomicU64>,
    pub(super) latest_frame: Arc<LatestFrame>,
    pub(super) decode_mailbox: Arc<DecodeMailbox>,
    pub(super) stream_generation: Arc<AtomicU64>,
    pub(super) stream_restart_requested: Arc<AtomicBool>,
    pub(super) latest_decoded_frame: PreviewFrameSlot,
    pub(super) decoded_frame_update_pending: Arc<AtomicBool>,
    pub(super) preview_active: Arc<AtomicBool>,
    pub(super) decode_time_micros: Arc<AtomicU64>,
    pub(super) dropped_decode_frames: Arc<AtomicU64>,
    pub(super) active_stream_configuration: Arc<Mutex<Option<StreamConfiguration>>>,
    pub(super) recording_mailbox: Arc<RecordingMailbox>,
    pub(super) photo_mailbox: Arc<PhotoMailbox>,
    pub(super) library_mailbox: Arc<LibraryRefreshMailbox>,
    pub(super) photo_context_generation: Arc<AtomicU64>,
    pub(super) active_session: Arc<Mutex<CaptureSession>>,
    pub(super) patient_search_generation: Arc<AtomicU64>,
    pub(super) patient_search_mailbox: Arc<PatientSearchMailbox>,
    pub(super) recording_start: Arc<Mutex<Option<Instant>>>,
    pub(super) frozen_frame: Arc<Mutex<Option<CapturedFrame>>>,
    pub(super) viewer_generation: Arc<AtomicU64>,
    pub(super) viewer_video_playing: Arc<AtomicBool>,
    pub(super) viewer_video_frame_count: Arc<AtomicU64>,
    pub(super) viewer_video_seek_request: Arc<Mutex<ViewerSeekState>>,
    pub(super) camera_controls: Arc<Mutex<Vec<CameraControlRuntimeState>>>,
    pub(super) control_commands: Arc<ControlCommandMailbox>,
    pub(super) camera_settings_save: Arc<CameraSettingsSaveMailbox>,
    pub(super) recovery_pending: Arc<AtomicBool>,
    pub(super) patient_action_pending: Arc<AtomicBool>,
    pub(super) viewer_display_mailbox: Arc<ViewerDisplayMailbox>,
    pub(super) viewer_open_mailbox: Arc<ViewerOpenMailbox>,
    workers: WorkerOwner,
    recording_duration_timer: slint::Timer,
    close_poll_timer: slint::Timer,
    storage_timer: slint::Timer,
}
impl AppRuntime {
    pub(super) fn new(settings_value: AppSettings, settings_path: std::path::PathBuf) -> Self {
        let settings = Arc::new(Mutex::new(settings_value));
        let map_generation = Arc::new(AtomicU64::new(0));
        let symbols_generation = Arc::new(AtomicU64::new(0));
        let latest_frame = Arc::new(LatestFrame::new());
        let decode_mailbox = Arc::new(DecodeMailbox::default());
        let stream_generation = Arc::new(AtomicU64::new(0));
        let stream_restart_requested = Arc::new(AtomicBool::new(false));
        let latest_decoded_frame: PreviewFrameSlot = Arc::new(Mutex::new(None));
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
        let patient_search_generation = Arc::new(AtomicU64::new(0));
        let patient_search_mailbox = Arc::new(PatientSearchMailbox::default());
        let recording_start: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
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
        let camera_settings_save = Arc::new(CameraSettingsSaveMailbox::default());
        let recovery_pending = Arc::new(AtomicBool::new(false));
        let patient_action_pending = Arc::new(AtomicBool::new(false));
        let viewer_display_mailbox = Arc::new(ViewerDisplayMailbox::default());
        let viewer_open_mailbox = Arc::new(ViewerOpenMailbox::default());
        Self {
            directory_generation: Arc::new(AtomicU64::new(0)),
            closing: Arc::new(AtomicBool::new(false)),
            background_jobs: Arc::new(crate::background::BackgroundJobs::default()),
            storage_operation: Arc::new(crate::operation_progress::OperationControl::default()),
            export_operation: Arc::new(crate::operation_progress::OperationControl::default()),
            reference_mailbox: Arc::new(crate::reference_worker::ReferenceMailbox::default()),
            settings,
            settings_path,
            map_generation,
            symbols_generation,
            latest_frame,
            decode_mailbox,
            stream_generation,
            stream_restart_requested,
            latest_decoded_frame,
            decoded_frame_update_pending,
            preview_active,
            decode_time_micros,
            dropped_decode_frames,
            active_stream_configuration,
            recording_mailbox,
            photo_mailbox,
            library_mailbox,
            photo_context_generation,
            active_session,
            patient_search_generation,
            patient_search_mailbox,
            recording_start,
            frozen_frame,
            viewer_generation,
            viewer_video_playing,
            viewer_video_frame_count,
            viewer_video_seek_request,
            camera_controls,
            control_commands,
            camera_settings_save,
            recovery_pending,
            patient_action_pending,
            viewer_display_mailbox,
            viewer_open_mailbox,
            workers: WorkerOwner::default(),
            recording_duration_timer: slint::Timer::default(),
            close_poll_timer: slint::Timer::default(),
            storage_timer: slint::Timer::default(),
        }
    }
    pub(super) fn start_workers(&mut self, main_window: &MainWindow) {
        self.start_auxiliary_workers(main_window);
        self.start_storage_timer(main_window);
        let settings = Arc::clone(&self.settings);
        let settings_path = self.settings_path.clone();
        let recording_mailbox = Arc::clone(&self.recording_mailbox);
        let photo_mailbox = Arc::clone(&self.photo_mailbox);
        let library_mailbox = Arc::clone(&self.library_mailbox);
        let photo_context_generation = Arc::clone(&self.photo_context_generation);
        let active_session = Arc::clone(&self.active_session);
        let patient_search_generation = Arc::clone(&self.patient_search_generation);
        let patient_search_mailbox = Arc::clone(&self.patient_search_mailbox);
        let recording_start = Arc::clone(&self.recording_start);
        let viewer_generation = Arc::clone(&self.viewer_generation);
        let viewer_video_playing = Arc::clone(&self.viewer_video_playing);
        let viewer_video_frame_count = Arc::clone(&self.viewer_video_frame_count);
        let viewer_video_seek_request = Arc::clone(&self.viewer_video_seek_request);
        let camera_settings_save = Arc::clone(&self.camera_settings_save);
        let viewer_display_mailbox = Arc::clone(&self.viewer_display_mailbox);
        let viewer_open_mailbox = Arc::clone(&self.viewer_open_mailbox);
        let patient_search_worker = thread::spawn({
            let mailbox = Arc::clone(&patient_search_mailbox);
            let generation = Arc::clone(&patient_search_generation);
            let settings = Arc::clone(&settings);
            let weak = main_window.as_weak();
            move || run_patient_search_worker(&mailbox, &generation, &settings, &weak)
        });
        let camera_settings_save_worker = thread::spawn({
            let mailbox = Arc::clone(&camera_settings_save);
            let settings = Arc::clone(&settings);
            let path = settings_path.clone();
            let window = main_window.as_weak();
            move || run_camera_settings_save_worker(&mailbox, &settings, &path, Some(&window))
        });

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
            let library_mailbox = Arc::clone(&library_mailbox);
            let context_generation = Arc::clone(&photo_context_generation);
            move || {
                run_photo_worker(
                    &mailbox,
                    &weak,
                    &settings,
                    &active_session,
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
            move || {
                run_recording_worker(
                    &mailbox,
                    &library_mailbox,
                    &weak,
                    &settings,
                    &active_session,
                    &recording_start,
                );
            }
        });

        let viewer_worker = thread::spawn({
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

        self.workers
            .add("recherche patients", patient_search_worker);
        self.workers
            .add("sauvegarde paramètres", camera_settings_save_worker);
        self.workers.add("bibliothèque", library_worker);
        self.workers.add("photos", photo_worker);
        self.workers.add("enregistrement", recorder_worker);
        self.workers.add("visionneuse", viewer_worker);
        for (name, handle) in camera_controller::start_workers(main_window, self) {
            self.workers.add(name, handle);
        }
        self.start_recording_timer(main_window);
    }

    fn start_auxiliary_workers(&mut self, main_window: &MainWindow) {
        for _ in 0..2 {
            let jobs = Arc::clone(&self.background_jobs);
            self.workers
                .add("tâches disque", thread::spawn(move || jobs.run()));
        }
        let mailbox = Arc::clone(&self.reference_mailbox);
        let settings = Arc::clone(&self.settings);
        let save = Arc::clone(&self.camera_settings_save);
        let weak = main_window.as_weak();
        self.workers.add(
            "références",
            thread::spawn(move || {
                crate::reference_worker::run_reference_worker(&mailbox, &weak, &settings, &save);
            }),
        );
    }

    fn start_storage_timer(&self, window: &MainWindow) {
        let weak = window.as_weak();
        let settings = Arc::clone(&self.settings);
        let jobs = Arc::clone(&self.background_jobs);
        let pending = Arc::new(AtomicBool::new(false));
        self.storage_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(5),
            move || {
                if pending.swap(true, Ordering::AcqRel) {
                    return;
                }
                let weak = weak.clone();
                let directory = settings_snapshot(&settings).capture_directory;
                let settings = Arc::clone(&settings);
                let completed = Arc::clone(&pending);
                if !jobs.submit(move || {
                    let result = iriscope_core::disk_space::available_space(&directory);
                    completed.store(false, Ordering::Release);
                    let _ = weak.upgrade_in_event_loop(move |win| {
                        if settings_snapshot(&settings).capture_directory != directory {
                            return;
                        }
                        let state = win.global::<crate::ui::AppState>();
                        crate::storage_controller::refresh_backup_status(
                            &win,
                            &settings_snapshot(&settings),
                        );
                        match result {
                            Ok(bytes) => {
                                state.set_storage_low(
                                    bytes < iriscope_core::disk_space::LOW_SPACE_THRESHOLD,
                                );
                                state.set_storage_critical(
                                    bytes < iriscope_core::disk_space::MIN_CAPTURE_RESERVE,
                                );
                                state.set_storage_status(
                                    format!(
                                        "Espace disponible : {}.{} Gio{}",
                                        bytes / 1_073_741_824,
                                        (bytes % 1_073_741_824) * 10 / 1_073_741_824,
                                        if state.get_storage_low() {
                                            " · espace faible"
                                        } else {
                                            ""
                                        }
                                    )
                                    .into(),
                                );
                                if state.get_storage_critical() && win.get_is_recording() {
                                    state.invoke_toggle_recording();
                                }
                            }
                            Err(error) => {
                                state.set_storage_status(
                                    format!("Espace disque non vérifié : {error}").into(),
                                );
                                state.set_storage_low(false);
                                state.set_storage_critical(false);
                            }
                        }
                    });
                }) {
                    pending.store(false, Ordering::Release);
                }
            },
        );
    }
    fn start_recording_timer(&self, main_window: &MainWindow) {
        self.recording_duration_timer
            .start(slint::TimerMode::Repeated, Duration::from_secs(1), {
                let weak = main_window.as_weak();
                let recording_start = Arc::clone(&self.recording_start);
                move || {
                    let Some(win) = weak.upgrade() else {
                        return;
                    };
                    if !win.get_is_recording() {
                        return;
                    }
                    if let Some(start) = recording_start.lock().ok().and_then(|guard| *guard) {
                        let elapsed = start.elapsed().as_secs();
                        win.set_recording_duration(
                            format!("{:02}:{:02}", elapsed / 60, elapsed % 60).into(),
                        );
                    }
                }
            });
    }

    pub(super) fn start_references(&mut self, _main_window: &MainWindow) {
        let settings = settings_snapshot(&self.settings);
        for (path, generation, is_map) in [
            (
                settings.iridology_map_path,
                Arc::clone(&self.map_generation),
                true,
            ),
            (
                settings.iridology_symbols_path,
                Arc::clone(&self.symbols_generation),
                false,
            ),
        ] {
            if let Some(path) = path {
                self.reference_mailbox
                    .request(crate::reference_worker::ReferenceRequest {
                        path,
                        generation: 0,
                        current_generation: generation,
                        is_map,
                        persist: false,
                    });
            }
        }
    }
    pub(crate) fn install_close_guard(&self, main_window: &MainWindow) {
        let closing = Arc::clone(&self.closing);
        let photos = Arc::clone(&self.photo_mailbox);
        let recording = Arc::clone(&self.recording_mailbox);
        let patient_action = Arc::clone(&self.patient_action_pending);
        let recovery = Arc::clone(&self.recovery_pending);
        let storage_operation = Arc::clone(&self.storage_operation);
        let export_operation = Arc::clone(&self.export_operation);
        let weak = main_window.as_weak();
        let started = Arc::new(Mutex::new(None::<Instant>));
        let started_request = Arc::clone(&started);
        let controls = Arc::clone(&self.control_commands);
        let preview_active = Arc::clone(&self.preview_active);
        let decode = Arc::clone(&self.decode_mailbox);
        main_window.window().on_close_requested(move || {
            storage_operation.cancel();
            export_operation.cancel();
            if weak.upgrade().is_some_and(|win| save_pending_notes(&win)) {
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            closing.store(true, Ordering::Release);
            controls.stop();
            preview_active.store(false, Ordering::Release);
            decode.close();
            photos.close();
            recording.stop(crate::recording_worker::RecordingStopReason::Interrupted);
            if photos.work_count() == 0
                && !recording.is_finalizing()
                && !patient_action.load(Ordering::Acquire)
                && !recovery.load(Ordering::Acquire)
                && !storage_operation.is_running()
                && !export_operation.is_running()
            {
                return slint::CloseRequestResponse::HideWindow;
            }
            if let Some(win) = weak.upgrade() {
                win.set_is_streaming(false);
                win.set_settings_feedback("Finalisation des sauvegardes avant fermeture…".into());
            }
            started_request
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(Instant::now);
            slint::CloseRequestResponse::KeepWindowShown
        });
        let closing = Arc::clone(&self.closing);
        let photos = Arc::clone(&self.photo_mailbox);
        let recording = Arc::clone(&self.recording_mailbox);
        let patient_action = Arc::clone(&self.patient_action_pending);
        let recovery = Arc::clone(&self.recovery_pending);
        let storage_operation = Arc::clone(&self.storage_operation);
        let export_operation = Arc::clone(&self.export_operation);
        let weak = main_window.as_weak();
        let warned = Arc::new(AtomicBool::new(false));
        self.close_poll_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                if !closing.load(Ordering::Acquire) {
                    return;
                }
                if photos.work_count() == 0
                    && !recording.is_finalizing()
                    && !patient_action.load(Ordering::Acquire)
                    && !recovery.load(Ordering::Acquire)
                    && !storage_operation.is_running()
                    && !export_operation.is_running()
                {
                    let _ = slint::quit_event_loop();
                    return;
                }
                if started
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_some_and(|time| time.elapsed() >= Duration::from_secs(5))
                    && !warned.swap(true, Ordering::AcqRel)
                    && let Some(win) = weak.upgrade()
                {
                    crate::playback::show_capture_notice(
                        &win,
                        "Sauvegarde encore en cours. La fermeture attend la fin des opérations.",
                        crate::config::NOTICE_INFO,
                    );
                }
            },
        );
    }

    pub(super) fn shutdown(&mut self) {
        self.closing.store(true, Ordering::Release);
        self.storage_operation.cancel();
        self.export_operation.cancel();
        self.background_jobs.close();
        self.reference_mailbox.close();
        self.recording_duration_timer.stop();
        self.close_poll_timer.stop();
        self.storage_timer.stop();
        self.viewer_generation.fetch_add(1, Ordering::AcqRel);
        self.map_generation.fetch_add(1, Ordering::AcqRel);
        self.symbols_generation.fetch_add(1, Ordering::AcqRel);
        self.patient_search_generation
            .fetch_add(1, Ordering::AcqRel);
        self.viewer_open_mailbox.close();
        self.control_commands.stop();
        self.recording_mailbox.close();
        self.decode_mailbox.close();
        self.photo_mailbox.close();
        self.library_mailbox.close();
        self.patient_search_mailbox.close();
        self.camera_settings_save.close();
        self.workers.finish(Duration::from_secs(5));
    }
}

fn save_pending_notes(window: &MainWindow) -> bool {
    let state = window.global::<crate::ui::AppState>();
    if state.get_consultation_dirty() || state.get_consultation_saving() {
        state.set_consultation_close_app_pending(true);
        if !state.get_consultation_saving() {
            state.invoke_save_consultation_notes();
        }
        return true;
    }
    if state.get_viewer_review_dirty() || state.get_viewer_review_busy() {
        state.set_viewer_close_app_pending(true);
        if !state.get_viewer_review_busy() {
            state.invoke_save_viewer_review();
        }
        return true;
    }
    false
}
