use crate::app_helpers::{capture_session_from_window, decode_camera_frame_to_rgb8};
#[cfg(test)]
use crate::camera_queue::DecodeMailbox;
use crate::config::{MAX_QUEUED_RECORDING_FRAMES, NOTICE_ERROR, NOTICE_SUCCESS};
use crate::library_ui::load_video_thumbnail;
#[cfg(test)]
use crate::library_worker::library_context_matches;
use crate::library_worker::{
    LibraryRefreshMailbox, recording_context_matches, refresh_library_in_background,
};
#[cfg(test)]
use crate::photo_worker::{PhotoRequest, save_photo};
use crate::playback::show_capture_notice;
use crate::ui::MainWindow;
use iriscope_core::camera::CapturedFrame;
use iriscope_core::capabilities::{FrameRate, PixelFormat};
#[cfg(test)]
use iriscope_core::library::try_scan_library_directory;
use iriscope_core::library::{CaptureCommit, CaptureKind, publish_indexed_capture};
use iriscope_core::session::CaptureSession;
use iriscope_core::settings::AppSettings;
use iriscope_core::storage::CaptureTimestamp;
#[cfg(test)]
use iriscope_core::video::AviMjpegReader;
use iriscope_core::video::{AviMjpegWriter, recover_partial_avi_staged};
use iriscope_imaging::{decode_mjpeg_to_rgb8, encode_rgb8_jpeg, ensure_jpeg_has_dht};
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(super) struct RecordingRequest {
    pub(super) directory: std::path::PathBuf,
    pub(super) file_name: String,
    pub(super) session: CaptureSession,
    pub(super) timestamp: CaptureTimestamp,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) frame_rate: FrameRate,
}

#[derive(Clone, Copy)]
pub(super) enum RecordingStopReason {
    User,
    Disconnected,
    Interrupted,
}

pub(super) enum RecordingCommand {
    Start(u64, RecordingRequest),
    Frame(u64, CapturedFrame),
    Stop(u64, RecordingStopReason, u64, Option<Duration>),
}

pub(super) struct RecordingMailbox {
    byte_budget: usize,
    pub(super) state: Mutex<RecordingMailboxState>,
    pub(super) ready: Condvar,
}

#[derive(Default)]
pub(super) struct RecordingMailboxState {
    pub(super) commands: VecDeque<RecordingCommand>,
    pub(super) active_generation: Option<u64>,
    pub(super) finalizing_generation: Option<u64>,
    pub(super) next_generation: u64,
    pub(super) queued_frames: usize,
    queued_bytes: usize,
    pub(super) dropped_frames: u64,
    pub(super) last_frame_timestamp: Option<Duration>,
    pub(super) closed: bool,
}

impl Default for RecordingMailbox {
    fn default() -> Self {
        Self::with_byte_budget(crate::config::MAX_RECORDING_QUEUE_BYTES)
    }
}

impl RecordingMailbox {
    pub(super) fn with_byte_budget(byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(RecordingMailboxState::default()),
            ready: Condvar::new(),
            byte_budget,
        }
    }

    pub(super) fn is_finalizing(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finalizing_generation
            .is_some()
    }

    pub(super) fn finish_finalization(&self, generation: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.finalizing_generation == Some(generation) {
            state.finalizing_generation = None;
        }
    }

    pub(super) fn active_generation(&self) -> Option<u64> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active_generation
    }

    pub(super) fn start(&self, request: RecordingRequest) -> Option<u64> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed
            || state.active_generation.is_some()
            || state.finalizing_generation.is_some()
        {
            return None;
        }
        state.next_generation = state.next_generation.saturating_add(1);
        let generation = state.next_generation;
        state.active_generation = Some(generation);
        state.dropped_frames = 0;
        state.last_frame_timestamp = None;
        state
            .commands
            .push_back(RecordingCommand::Start(generation, request));
        self.ready.notify_one();
        Some(generation)
    }

    pub(super) fn publish_frame(&self, frame: CapturedFrame) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(generation) = state.active_generation else {
            return;
        };
        state.last_frame_timestamp = Some(
            state
                .last_frame_timestamp
                .map_or(frame.timestamp, |last| last.max(frame.timestamp)),
        );
        if state.queued_frames >= MAX_QUEUED_RECORDING_FRAMES
            || frame.data.len() > self.byte_budget.saturating_sub(state.queued_bytes)
        {
            state.dropped_frames = state.dropped_frames.saturating_add(1);
            return;
        }
        state.queued_bytes += frame.data.len();
        state
            .commands
            .push_back(RecordingCommand::Frame(generation, frame));
        state.queued_frames += 1;
        self.ready.notify_one();
    }

    pub(super) fn stop(&self, reason: RecordingStopReason) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(generation) = state.active_generation.take() else {
            return false;
        };
        state.finalizing_generation = Some(generation);
        let dropped = state.dropped_frames;
        let last_timestamp = state.last_frame_timestamp;
        state.commands.push_back(RecordingCommand::Stop(
            generation,
            reason,
            dropped,
            last_timestamp,
        ));
        self.ready.notify_one();
        true
    }

    pub(super) fn abort(&self, generation: u64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active_generation == Some(generation) {
            state.active_generation = None;
            state.finalizing_generation = None;
            return true;
        }
        if state.finalizing_generation == Some(generation) {
            state.finalizing_generation = None;
        }
        false
    }

    pub(super) fn receive(&self) -> Option<RecordingCommand> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(command) = state.commands.pop_front() {
                if let RecordingCommand::Frame(_, frame) = &command {
                    state.queued_frames -= 1;
                    state.queued_bytes = state.queued_bytes.saturating_sub(frame.data.len());
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

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(generation) = state.active_generation.take() {
            state.finalizing_generation = Some(generation);
            let dropped = state.dropped_frames;
            let last_timestamp = state.last_frame_timestamp;
            state.commands.push_back(RecordingCommand::Stop(
                generation,
                RecordingStopReason::Interrupted,
                dropped,
                last_timestamp,
            ));
        }
        state.closed = true;
        self.ready.notify_all();
    }
}

pub(super) fn pending_recording_paths(
    directory: &std::path::Path,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut pending = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with(".iriscope-recording") || name.starts_with(".iriscope-recovery"))
            && name.ends_with(".part")
        {
            pending.push(entry.path());
        }
    }
    pending.sort();
    Ok(pending)
}

pub(super) fn recover_indexed_recording(
    source: &std::path::Path,
    directory: &std::path::Path,
    file_name: &str,
) -> std::io::Result<Option<CaptureCommit>> {
    recover_partial_avi_staged(source, directory)?
        .map(|pending| {
            publish_indexed_capture(
                &pending,
                directory,
                file_name,
                &CaptureSession::default(),
                CaptureKind::Video,
                CaptureTimestamp::now(),
            )
        })
        .transpose()
}

pub(super) fn stop_recording(
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

const MAX_RECORDING_DURATION: Duration = Duration::from_secs(60 * 60);
const MAX_TIMING_BURST_FRAMES: u64 = 300;
const MAX_TIMING_BURST_BYTES: u64 = 64 * 1024 * 1024;

/// AVI stores a constant frame rate. Keep that rate and hold the latest image
/// until the next camera timestamp, so a missing frame does not accelerate time.
struct RecordingTimeline {
    frame_rate: FrameRate,
    first: Option<Duration>,
    last: Option<Duration>,
    next_slot: u64,
    pending: bool,
}

struct TimingPlan {
    repeated_frames: u64,
    write_current: bool,
}

impl RecordingTimeline {
    fn new(frame_rate: FrameRate) -> Self {
        Self {
            frame_rate,
            first: None,
            last: None,
            next_slot: 0,
            pending: false,
        }
    }

    fn observe(
        &mut self,
        timestamp: Duration,
        previous_jpeg_size: usize,
    ) -> Result<Option<TimingPlan>, String> {
        let Some(first) = self.first else {
            self.first = Some(timestamp);
            self.last = Some(timestamp);
            self.next_slot = 1;
            return Ok(Some(TimingPlan {
                repeated_frames: 0,
                write_current: true,
            }));
        };
        // Ignore duplicates and reordered timestamps without changing the
        // origin or replacing the latest image with an older one.
        if self.last.is_some_and(|last| timestamp <= last) {
            return Ok(None);
        }
        let elapsed = timestamp
            .checked_sub(first)
            .ok_or_else(|| "Horodatage vidéo antérieur au début de l'enregistrement.".to_owned())?;
        if elapsed > MAX_RECORDING_DURATION {
            return Err("Durée maximale d'enregistrement atteinte (1 heure).".to_owned());
        }
        let scaled = elapsed.as_nanos() * u128::from(self.frame_rate.numerator());
        let denominator = 1_000_000_000_u128 * u128::from(self.frame_rate.denominator());
        // Emit only grid positions strictly before the incoming timestamp.
        // The incoming image is retained for the next grid position.
        let before_current = u64::try_from(scaled.div_ceil(denominator))
            .map_err(|_| "Horodatage vidéo trop grand.".to_owned())?;
        let repeated_frames = before_current.saturating_sub(self.next_slot);
        if repeated_frames > MAX_TIMING_BURST_FRAMES
            || u128::from(repeated_frames) * previous_jpeg_size as u128
                > u128::from(MAX_TIMING_BURST_BYTES)
        {
            return Err(
                "Interruption caméra trop longue pour conserver la durée vidéo. Enregistrement arrêté."
                    .to_owned(),
            );
        }
        self.next_slot = self.next_slot.max(before_current);
        self.last = Some(timestamp);
        self.pending = true;
        Ok(Some(TimingPlan {
            repeated_frames,
            write_current: false,
        }))
    }
}

pub(super) struct ActiveRecording {
    pub(super) generation: u64,
    pub(super) writer: AviMjpegWriter,
    pub(super) pending_path: std::path::PathBuf,
    pub(super) file_name: String,
    pub(super) directory: std::path::PathBuf,
    pub(super) session: CaptureSession,
    pub(super) timestamp: CaptureTimestamp,
    timeline: RecordingTimeline,
    last_jpeg: Vec<u8>,
    pub(super) written_frames: u64,
    pub(super) invalid_frames: u64,
    pub(super) error: Option<String>,
}

impl ActiveRecording {
    fn write_timed_frame(&mut self, jpeg: &[u8], timestamp: Duration) -> Result<(), String> {
        let Some(plan) = self.timeline.observe(timestamp, self.last_jpeg.len())? else {
            return Ok(());
        };
        for _ in 0..plan.repeated_frames {
            self.writer
                .write_frame(&self.last_jpeg)
                .map_err(|error| error.to_string())?;
            self.written_frames += 1;
        }
        if plan.write_current {
            self.writer
                .write_frame(jpeg)
                .map_err(|error| error.to_string())?;
            self.written_frames += 1;
        }
        // Reuse one buffer instead of retaining the recording or allocating one
        // image for every repeated output frame.
        self.last_jpeg.clear();
        self.last_jpeg.extend_from_slice(jpeg);
        Ok(())
    }

    fn flush_last_frame(&mut self) -> Result<(), String> {
        if self.timeline.pending {
            self.writer
                .write_frame(&self.last_jpeg)
                .map_err(|error| error.to_string())?;
            self.written_frames += 1;
            self.timeline.pending = false;
        }
        Ok(())
    }

    fn extend_to_last_received_frame(&mut self, timestamp: Duration) -> Result<(), String> {
        if self.written_frames == 0 {
            return Ok(());
        }
        if let Some(plan) = self.timeline.observe(timestamp, self.last_jpeg.len())? {
            for _ in 0..plan.repeated_frames {
                self.writer
                    .write_frame(&self.last_jpeg)
                    .map_err(|error| error.to_string())?;
                self.written_frames += 1;
            }
        }
        Ok(())
    }
}

fn publish_finished_video(
    pending_path: &std::path::Path,
    directory: &std::path::Path,
    file_name: &str,
    session: &CaptureSession,
    timestamp: CaptureTimestamp,
) -> std::io::Result<CaptureCommit> {
    publish_indexed_capture(
        pending_path,
        directory,
        file_name,
        session,
        CaptureKind::Video,
        timestamp,
    )
}

#[allow(clippy::too_many_lines)]
pub(super) fn run_recording_worker(
    mailbox: &Arc<RecordingMailbox>,
    library_mailbox: &LibraryRefreshMailbox,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    recording_start: &Arc<Mutex<Option<Instant>>>,
) {
    let mut active: Option<ActiveRecording> = None;
    while let Some(command) = mailbox.receive() {
        match command {
            RecordingCommand::Start(generation, request) => {
                match iriscope_core::disk_space::ensure_available_space(&request.directory, 0)
                    .and_then(|()| {
                        AviMjpegWriter::create_unique(
                            &request.directory,
                            ".iriscope-recording.part",
                            request.width,
                            request.height,
                            request.frame_rate,
                        )
                    }) {
                    Ok((writer, pending_path)) => {
                        active = Some(ActiveRecording {
                            generation,
                            writer,
                            pending_path,
                            file_name: request.file_name,
                            directory: request.directory,
                            session: request.session,
                            timestamp: request.timestamp,
                            timeline: RecordingTimeline::new(request.frame_rate),
                            last_jpeg: Vec::new(),
                            written_frames: 0,
                            invalid_frames: 0,
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
                                win.set_recording_finalizing(false);
                                win.set_recording_duration("00:00".into());
                                show_capture_notice(&win, message, NOTICE_ERROR);
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
                    PixelFormat::Mjpeg => {
                        let jpeg = ensure_jpeg_has_dht(&frame.data);
                        if decode_mjpeg_to_rgb8(&jpeg).is_err() {
                            recording.invalid_frames = recording.invalid_frames.saturating_add(1);
                            continue;
                        }
                        Ok(jpeg)
                    }
                    _ => decode_camera_frame_to_rgb8(&frame)
                        .ok_or_else(|| "format caméra non décodable".to_owned())
                        .and_then(|(width, height, rgb)| {
                            encode_rgb8_jpeg(&rgb, width, height, 95)
                                .map(std::borrow::Cow::Owned)
                                .map_err(|error| error.to_string())
                        }),
                };
                let result = jpeg
                    .map_err(|error| error.to_string())
                    .and_then(|jpeg| recording.write_timed_frame(jpeg.as_ref(), frame.timestamp));
                match result {
                    Ok(()) => {}
                    Err(error) => {
                        recording.error = Some(error.clone());
                        stop_recording(mailbox, recording_start, RecordingStopReason::Interrupted);
                        let mailbox_for_ui = Arc::clone(mailbox);
                        let _ = weak.upgrade_in_event_loop(move |win| {
                            if mailbox_for_ui.active_generation().is_none() {
                                win.set_is_recording(false);
                                win.set_recording_finalizing(mailbox_for_ui.is_finalizing());
                                win.set_recording_duration("00:00".into());
                                show_capture_notice(
                                    &win,
                                    format!("Enregistrement arrêté : {error}"),
                                    NOTICE_ERROR,
                                );
                            }
                        });
                    }
                }
            }
            RecordingCommand::Stop(generation, reason, dropped_frames, last_received_timestamp) => {
                if active
                    .as_ref()
                    .is_none_or(|recording| recording.generation != generation)
                {
                    mailbox.finish_finalization(generation);
                    continue;
                }
                let mut recording = active.take().expect("matching active recording");
                if recording.error.is_none() {
                    let timing_result = last_received_timestamp
                        .map_or(Ok(()), |timestamp| {
                            recording.extend_to_last_received_frame(timestamp)
                        })
                        .and_then(|()| recording.flush_last_frame());
                    if let Err(error) = timing_result {
                        recording.error = Some(error);
                    }
                }
                let finish_result = recording.writer.finish();
                // Close the file before creating its final name. This is also
                // required for publication on Windows.
                drop(recording.writer);
                let write_error = recording
                    .error
                    .or_else(|| finish_result.err().map(|error| error.to_string()));
                let has_write_error = write_error.is_some();
                let capture_result = if let Some(error) = write_error {
                    Err(format!(
                        "Erreur vidéo : {error}. Fichier temporaire récupérable : {}",
                        recording.pending_path.display()
                    ))
                } else if recording.written_frames == 0 {
                    Err("Vidéo sans image enregistrée.".to_owned())
                } else {
                    publish_finished_video(
                        &recording.pending_path,
                        &recording.directory,
                        &recording.file_name,
                        &recording.session,
                        recording.timestamp,
                    )
                    .map_err(|error| {
                        format!(
                            "Impossible de publier la vidéo : {error}. Fichier récupérable : {}",
                            recording.pending_path.display()
                        )
                    })
                };
                if recording.written_frames == 0
                    && !has_write_error
                    && let Err(error) = std::fs::remove_file(&recording.pending_path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    eprintln!("[IrisScope] Nettoyage vidéo temporaire impossible : {error}");
                }
                let saved_path = capture_result
                    .as_ref()
                    .ok()
                    .map(|committed| committed.file_path.clone());
                let saved_version = capture_result
                    .as_ref()
                    .ok()
                    .map(|committed| committed.file_version.clone());
                let metadata_warning = capture_result
                    .as_ref()
                    .ok()
                    .and_then(|committed| committed.metadata_warning.as_ref())
                    .map(|error| {
                        format!(
                            "Vidéo enregistrée ; association au dossier en attente de reprise : {error}"
                        )
                    });
                let success = saved_path.is_some();
                let message = if let Err(error) = capture_result {
                    error
                } else if let Some(warning) = metadata_warning.as_ref() {
                    warning.clone()
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
                    let omitted_frames = dropped_frames.saturating_add(recording.invalid_frames);
                    if omitted_frames == 0 {
                        prefix.to_owned()
                    } else {
                        format!("{prefix} ({omitted_frames} images ignorées)")
                    }
                };
                let mailbox_for_ui = Arc::clone(mailbox);
                let thumbnail = saved_path.as_ref().and_then(|path| {
                    let unchanged = || {
                        iriscope_core::library::capture_file_version(path)
                            .ok()
                            .as_ref()
                            == saved_version.as_ref()
                    };
                    if !unchanged() {
                        return None;
                    }
                    let (width, height, rgb) = load_video_thumbnail(path)?;
                    unchanged().then(|| {
                        SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height)
                    })
                });
                let settings_for_ui = Arc::clone(settings);
                let active_session_for_ui = Arc::clone(active_session);
                let directory_for_ui = recording.directory;
                let session_for_ui = recording.session;
                let mailbox_if_ui_closed = Arc::clone(mailbox);
                let notice_tone = if success && metadata_warning.is_none() {
                    NOTICE_SUCCESS
                } else {
                    NOTICE_ERROR
                };
                if weak
                    .upgrade_in_event_loop(move |win| {
                        mailbox_for_ui.finish_finalization(generation);
                        let form_session = capture_session_from_window(&win).ok();
                        if mailbox_for_ui.active_generation().is_none() {
                            win.set_recording_finalizing(false);
                        }
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
                            if let Some(path_for_ui) = saved_path {
                                win.set_last_capture_file_version(
                                    saved_version
                                        .as_ref()
                                        .map_or_else(
                                            String::new,
                                            iriscope_core::library::CaptureFileVersion::token,
                                        )
                                        .into(),
                                );
                                win.set_last_capture_path(
                                    path_for_ui.to_string_lossy().to_string().into(),
                                );
                                if let Some(name) =
                                    path_for_ui.file_name().and_then(|value| value.to_str())
                                {
                                    win.set_last_capture_file_name(name.into());
                                }
                                win.set_has_last_capture_thumbnail(thumbnail.is_some());
                                if let Some(pixels) = thumbnail {
                                    win.set_last_capture_thumbnail(slint::Image::from_rgb8(pixels));
                                }
                                win.set_has_last_capture(true);
                            }
                            show_capture_notice(&win, message, notice_tone);
                        }
                    })
                    .is_err()
                {
                    mailbox_if_ui_closed.finish_finalization(generation);
                }
                if metadata_warning.is_some() {
                    library_mailbox.invalidate_thumbnails();
                }
                refresh_library_in_background(library_mailbox, settings, active_session);
            }
        }
    }
}

#[cfg(test)]
mod recording_pipeline_tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
        time::{Duration, SystemTime},
    };

    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{FrameRate, PixelFormat, Resolution},
        session::{CaptureSession, Eye},
        settings::AppSettings,
        storage::CaptureTimestamp,
    };

    use super::{
        ActiveRecording, AviMjpegReader, AviMjpegWriter, DecodeMailbox, PhotoRequest,
        RecordingCommand, RecordingMailbox, RecordingRequest, RecordingStopReason,
        RecordingTimeline, decode_camera_frame_to_rgb8, encode_rgb8_jpeg, library_context_matches,
        pending_recording_paths, publish_finished_video, recording_context_matches,
        recover_indexed_recording, save_photo, try_scan_library_directory,
    };

    pub(super) fn frame(sequence_number: u64) -> CapturedFrame {
        CapturedFrame {
            sequence_number,
            timestamp: Duration::from_millis(sequence_number * 160),
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(640, 480),
            data: Arc::from([0xff, 0xd8, 0xff, 0xd9]),
        }
    }

    pub(super) fn request() -> RecordingRequest {
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
    pub(super) fn detects_interrupted_recording_and_recovery_files() {
        let directory = std::env::temp_dir().join(format!(
            "iriscope_pending_recordings_{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir(&directory).expect("create directory");
        for name in [
            ".iriscope-recording.part",
            ".iriscope-recovery_2.part",
            "unrelated.part",
        ] {
            fs::write(directory.join(name), b"test").expect("write placeholder");
        }
        let pending = pending_recording_paths(&directory).expect("scan pending files");
        assert_eq!(pending.len(), 2);
        assert!(
            pending
                .iter()
                .any(|path| path.ends_with(".iriscope-recording.part"))
        );
        assert!(
            pending
                .iter()
                .any(|path| path.ends_with(".iriscope-recovery_2.part"))
        );
        fs::remove_dir_all(directory).expect("remove directory");
    }

    #[test]
    pub(super) fn rejects_invalid_native_jpeg_before_creating_a_photo() {
        let directory = std::env::temp_dir().join(format!(
            "iriscope_bad_photo_{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let request = PhotoRequest {
            frame: frame(0),
            directory: directory.clone(),
            filename_template: "Iris_{date}_{time}_{eye}".to_owned(),
            session: CaptureSession::default(),
            timestamp: CaptureTimestamp::now(),
            context_generation: 0,
        };
        assert!(save_photo(&request).is_err());
        assert!(!directory.exists());
    }

    #[test]
    pub(super) fn publishes_complete_video_without_replacing_a_colliding_name() {
        let directory = std::env::temp_dir().join(format!(
            "iriscope_publish_video_{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir(&directory).expect("create directory");
        fs::write(directory.join("capture.avi"), b"older capture").expect("old capture");
        let pending = directory.join(".iriscope-recording.part");
        fs::write(&pending, b"completed AVI").expect("pending capture");
        let published = publish_finished_video(
            &pending,
            &directory,
            "capture.avi",
            &CaptureSession::default(),
            CaptureTimestamp::now(),
        )
        .expect("publish new capture")
        .file_path;
        assert_eq!(published.file_name().unwrap(), "capture_2.avi");
        assert_eq!(
            fs::read(directory.join("capture.avi")).unwrap(),
            b"older capture"
        );
        assert_eq!(fs::read(&published).unwrap(), b"completed AVI");
        assert!(!pending.exists());
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn recording_queue_saturates_by_bytes_and_releases_consumed_budget() {
        let mailbox = RecordingMailbox::with_byte_budget(5);
        let generation = mailbox.start(request()).unwrap();
        assert!(matches!(
            mailbox.receive(),
            Some(RecordingCommand::Start(_, _))
        ));
        let mut first = frame(1);
        first.data = vec![0_u8; 3].into();
        let mut second = frame(2);
        second.data = vec![0_u8; 3].into();
        mailbox.publish_frame(first);
        mailbox.publish_frame(second.clone());
        assert_eq!(mailbox.state.lock().unwrap().queued_frames, 1);
        assert_eq!(mailbox.state.lock().unwrap().dropped_frames, 1);
        assert!(
            matches!(mailbox.receive(), Some(RecordingCommand::Frame(id, _)) if id == generation)
        );
        mailbox.publish_frame(second);
        assert_eq!(mailbox.state.lock().unwrap().queued_frames, 1);
        mailbox.close();
    }

    #[test]
    pub(super) fn recording_queue_bounds_frames_and_orders_stop_after_accepted_frames() {
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
            Some(RecordingCommand::Stop(id, RecordingStopReason::User, 2, Some(timestamp)))
                if id == generation && timestamp == Duration::from_millis(1_440)
        ));
        assert!(mailbox.active_generation().is_none());
        assert!(mailbox.is_finalizing());
        assert!(mailbox.start(request()).is_none());
        mailbox.finish_finalization(generation);
        assert!(!mailbox.is_finalizing());
        assert!(mailbox.start(request()).is_some());
    }

    #[test]
    pub(super) fn irregular_timestamps_hold_the_previous_image_at_nominal_rate() {
        let rate = FrameRate::new(10, 1).expect("frame rate");
        let (mut recording, directory) = test_recording(rate);
        let first = encode_rgb8_jpeg(&[255, 0, 0], 1, 1, 95).expect("first JPEG");
        let second = encode_rgb8_jpeg(&[0, 255, 0], 1, 1, 95).expect("second JPEG");
        let third = encode_rgb8_jpeg(&[0, 0, 255], 1, 1, 95).expect("third JPEG");
        for (timestamp, jpeg) in [(0, &first), (180, &second), (610, &third)] {
            recording
                .write_timed_frame(jpeg, Duration::from_millis(timestamp))
                .expect("write timed image");
        }
        recording.flush_last_frame().expect("flush final image");
        recording.writer.finish().expect("finish AVI");
        let path = recording.pending_path.clone();
        drop(recording);
        let mut reader = AviMjpegReader::open(path).expect("read AVI");
        assert_eq!(reader.frame_rate(), rate);
        let expected = [
            &first, &first, &second, &second, &second, &second, &second, &third,
        ];
        assert_eq!(reader.frame_count(), expected.len());
        for (index, jpeg) in expected.into_iter().enumerate() {
            assert_eq!(reader.read_frame(index).expect("frame"), *jpeg);
        }
        // The last image appears at 700 ms, less than one nominal interval
        // after its 610 ms capture, even though many intervening frames are lost.
        drop(reader);
        fs::remove_dir_all(directory).expect("remove timing test directory");
    }

    #[test]
    pub(super) fn dropped_tail_frames_keep_their_elapsed_time_in_the_avi() {
        let rate = FrameRate::new(10, 1).expect("frame rate");
        let (mut recording, directory) = test_recording(rate);
        let jpeg = encode_rgb8_jpeg(&[255, 0, 0], 1, 1, 95).expect("JPEG");
        recording
            .write_timed_frame(&jpeg, Duration::ZERO)
            .expect("first frame");
        recording
            .write_timed_frame(&jpeg, Duration::from_millis(100))
            .expect("second frame");
        // The bounded mailbox dropped all subsequent frames through 900 ms.
        recording
            .extend_to_last_received_frame(Duration::from_millis(900))
            .expect("preserve dropped tail duration");
        recording.flush_last_frame().expect("flush tail");
        recording.writer.finish().expect("finish AVI");
        assert_eq!(recording.written_frames, 10);
        let path = recording.pending_path.clone();
        drop(recording);
        let reader = AviMjpegReader::open(path).expect("read AVI");
        assert_eq!(reader.frame_rate(), rate);
        assert_eq!(reader.frame_count(), 10);
        drop(reader);
        fs::remove_dir_all(directory).expect("remove timing test directory");
    }

    #[test]
    pub(super) fn recovered_recording_is_indexed_without_guessing_a_patient() {
        let rate = FrameRate::new(8, 1).expect("rate");
        let (mut recording, directory) = test_recording(rate);
        let jpeg = encode_rgb8_jpeg(&[255, 0, 0], 1, 1, 95).expect("JPEG");
        recording
            .write_timed_frame(&jpeg, Duration::ZERO)
            .expect("first frame");
        recording.writer.finish().expect("finish source");
        let source = recording.pending_path.clone();
        drop(recording);
        let committed = recover_indexed_recording(&source, &directory, "recovered.avi")
            .expect("recover indexed recording")
            .expect("complete frame");
        assert!(source.exists());
        assert!(committed.metadata_warning.is_none());
        let recovered_entry = try_scan_library_directory(&directory)
            .expect("scan indexed library")
            .into_iter()
            .find(|entry| entry.file_path == committed.file_path)
            .expect("recovered capture is in library");
        assert_eq!(
            recovered_entry.kind,
            iriscope_core::library::CaptureKind::Video
        );
        assert_eq!(recovered_entry.patient_id, None);
        assert_eq!(recovered_entry.first_name, None);
        assert_eq!(recovered_entry.last_name, None);
        fs::remove_dir_all(directory).expect("remove recovery test directory");
    }

    fn test_recording(rate: FrameRate) -> (ActiveRecording, std::path::PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "iriscope_timed_recording_{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let (writer, pending_path) =
            AviMjpegWriter::create_unique(&directory, "timing.avi", 1, 1, rate)
                .expect("create AVI");
        (
            ActiveRecording {
                generation: 1,
                writer,
                pending_path,
                file_name: "timing.avi".to_owned(),
                directory: directory.clone(),
                session: CaptureSession::default(),
                timestamp: CaptureTimestamp::now(),
                timeline: RecordingTimeline::new(rate),
                last_jpeg: Vec::new(),
                written_frames: 0,
                invalid_frames: 0,
                error: None,
            },
            directory,
        )
    }

    #[test]
    pub(super) fn resampling_rejects_timestamp_jumps_without_a_large_burst() {
        let mut timeline = RecordingTimeline::new(FrameRate::new(30, 1).expect("rate"));
        timeline
            .observe(Duration::ZERO, 0)
            .expect("first timestamp");
        assert!(timeline.observe(Duration::from_secs(60), 1_000).is_err());
        assert_eq!(timeline.next_slot, 1);
        assert!(timeline.observe(Duration::from_secs(3_601), 1_000).is_err());
        assert_eq!(timeline.next_slot, 1);
        // The byte budget also bounds bursts for unusually large JPEG frames.
        assert!(
            timeline
                .observe(Duration::from_secs(1), 4 * 1024 * 1024)
                .is_err()
        );
        assert_eq!(timeline.next_slot, 1);
    }

    #[test]
    pub(super) fn resampling_ignores_reordered_timestamps_and_coalesces_fast_frames() {
        let rate = FrameRate::new(10, 1).expect("rate");
        let mut timeline = RecordingTimeline::new(rate);
        assert!(
            timeline
                .observe(Duration::from_secs(1), 0)
                .unwrap()
                .unwrap()
                .write_current
        );
        assert_eq!(
            timeline
                .observe(Duration::from_millis(1_025), 10)
                .unwrap()
                .unwrap()
                .repeated_frames,
            0,
        );
        assert!(
            timeline
                .observe(Duration::from_millis(1_010), 10)
                .unwrap()
                .is_none()
        );
        assert!(
            timeline
                .observe(Duration::from_millis(1_025), 10)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            timeline
                .observe(Duration::from_millis(1_201), 10)
                .unwrap()
                .unwrap()
                .repeated_frames,
            2,
        );
    }

    #[test]
    pub(super) fn fractional_nominal_rate_uses_exact_integer_timestamps() {
        let rate = FrameRate::new(30_000, 1_001).expect("fractional rate");
        let mut timeline = RecordingTimeline::new(rate);
        timeline
            .observe(Duration::ZERO, 0)
            .expect("first timestamp");
        let plan = timeline
            .observe(Duration::from_millis(1_001), 100)
            .expect("exact 30-frame interval")
            .expect("new timestamp");
        assert_eq!(plan.repeated_frames, 29);
        assert_eq!(timeline.next_slot, 30);
    }

    #[test]
    pub(super) fn old_decode_generation_is_rejected_after_reconnect() {
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
    pub(super) fn old_library_result_cannot_replace_new_session_or_directory() {
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
    pub(super) fn old_recording_result_cannot_replace_new_patient_capture() {
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
    pub(super) fn truncated_yuyv_frame_is_not_published_for_preview() {
        let mut frame = frame(1);
        frame.pixel_format = PixelFormat::Yuyv;
        frame.data = Arc::from([0_u8, 0_u8]);
        assert!(decode_camera_frame_to_rgb8(&frame).is_none());
    }
}
