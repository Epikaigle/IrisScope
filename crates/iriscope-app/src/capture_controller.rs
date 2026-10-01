use crate::app_helpers::{capture_session_from_window, settings_snapshot};
use crate::config::{NOTICE_ERROR, NOTICE_INFO, NOTICE_SUCCESS};
use crate::library_controller::refresh_pending_recording_count;
use crate::library_ui::clear_library_view;
use crate::library_worker::refresh_library_in_background;
use crate::photo_worker::PhotoRequest;
use crate::playback::show_capture_notice;
use crate::recording_worker::{
    RecordingRequest, RecordingStopReason, pending_recording_paths, recover_indexed_recording,
    stop_recording,
};
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::storage::{CaptureNamingPolicy, CaptureTimestamp};
use slint::ComponentHandle;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[allow(clippy::too_many_lines)]
pub(super) fn install(main_window: &MainWindow, runtime: &AppRuntime) {
    let settings = Arc::clone(&runtime.settings);
    let latest_frame = Arc::clone(&runtime.latest_frame);
    let active_stream_configuration = Arc::clone(&runtime.active_stream_configuration);
    let recording_mailbox = Arc::clone(&runtime.recording_mailbox);
    let photo_mailbox = Arc::clone(&runtime.photo_mailbox);
    let library_mailbox = Arc::clone(&runtime.library_mailbox);
    let photo_context_generation = Arc::clone(&runtime.photo_context_generation);
    let active_session = Arc::clone(&runtime.active_session);
    let recording_start = Arc::clone(&runtime.recording_start);
    let frozen_frame = Arc::clone(&runtime.frozen_frame);
    let recovery_pending = Arc::clone(&runtime.recovery_pending);
    let weak = main_window.as_weak();
    let latest_frame_cap = Arc::clone(&latest_frame);
    let frozen_frame_cap = Arc::clone(&frozen_frame);
    let settings_cap = Arc::clone(&settings);
    let session_cap = Arc::clone(&active_session);
    let photo_mailbox_cap = Arc::clone(&photo_mailbox);
    let library_mailbox_cap = Arc::clone(&library_mailbox);
    let context_generation_cap = Arc::clone(&photo_context_generation);

    let closing_photo = Arc::clone(&runtime.closing);
    let background_1 = Arc::clone(&runtime.background_jobs);
    main_window
        .global::<AppState>()
        .on_trigger_capture(move || {
            let Some(win) = weak.upgrade() else { return };
            if closing_photo.load(Ordering::Acquire) {
                return;
            }
            if win.get_recording_finalizing() {
                show_capture_notice(&win, "Finalisation de la vidéo en cours...", NOTICE_INFO);
                return;
            }
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
                show_capture_notice(&win, "Aucune image caméra disponible.", NOTICE_ERROR);
                return;
            };
            let session = match capture_session_from_window(&win) {
                Ok(session) => session,
                Err(message) => {
                    show_capture_notice(&win, message, NOTICE_ERROR);
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
                show_capture_notice(
                    &win,
                    "Traitement photo en cours : réessayez dans un instant.",
                    NOTICE_INFO,
                );
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
            if !background_1.submit(|| {
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
            }) {
                crate::playback::show_capture_notice(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                    crate::config::NOTICE_ERROR,
                );
            }
        });

    let closing_recording = Arc::clone(&runtime.closing);
    // Recording callback
    let weak_rec = main_window.as_weak();
    let recording_mailbox_rec = Arc::clone(&recording_mailbox);
    let settings_rec = Arc::clone(&settings);
    let session_rec = Arc::clone(&active_session);
    let rec_start_rec = Arc::clone(&recording_start);
    let active_stream_configuration_rec = Arc::clone(&active_stream_configuration);
    let latest_frame_rec = Arc::clone(&latest_frame);
    let recovery_pending_rec = Arc::clone(&recovery_pending);

    main_window
        .global::<AppState>()
        .on_toggle_recording(move || {
            let Some(win) = weak_rec.upgrade() else {
                return;
            };
            if closing_recording.load(Ordering::Acquire) && !win.get_is_recording() {
                return;
            }
            if win.get_recording_finalizing() {
                show_capture_notice(&win, "Finalisation de la vidéo en cours...", NOTICE_INFO);
                return;
            }
            let currently_recording = recording_mailbox_rec.active_generation().is_some();
            let recording_settings = settings_snapshot(&settings_rec);

            if currently_recording {
                // Stop recording
                let stopped = stop_recording(
                    &recording_mailbox_rec,
                    &rec_start_rec,
                    RecordingStopReason::User,
                );
                win.set_is_recording(false);
                win.set_recording_finalizing(stopped);
                win.set_recording_duration("00:00".into());
                show_capture_notice(
                    &win,
                    if stopped {
                        "Finalisation de la vidéo en cours..."
                    } else {
                        "La vidéo est déjà arrêtée."
                    },
                    NOTICE_INFO,
                );
            } else {
                // Start recording
                if recovery_pending_rec.load(Ordering::Acquire) {
                    show_capture_notice(
                        &win,
                        "Récupération des vidéos en cours : patientez avant d'enregistrer.",
                        NOTICE_INFO,
                    );
                    return;
                }
                let session = match capture_session_from_window(&win) {
                    Ok(session) => session,
                    Err(message) => {
                        show_capture_notice(&win, message, NOTICE_ERROR);
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
                    show_capture_notice(
                        &win,
                        "Erreur vidéo : aucun flux caméra actif",
                        NOTICE_ERROR,
                    );
                    return;
                };

                let Some(current_frame) = latest_frame_rec.snapshot() else {
                    show_capture_notice(
                        &win,
                        "Erreur vidéo : aucune frame caméra disponible",
                        NOTICE_ERROR,
                    );
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
                    win.set_recording_finalizing(false);
                    win.set_recording_duration("00:00".into());
                } else {
                    show_capture_notice(&win, "La vidéo n'a pas pu démarrer.", NOTICE_ERROR);
                }
            }
        });

    let weak_recover = main_window.as_weak();
    let settings_recover = Arc::clone(&settings);
    let library_recover = Arc::clone(&library_mailbox);
    let session_recover = Arc::clone(&active_session);
    let recovery_pending_callback = Arc::clone(&recovery_pending);
    let recording_mailbox_recover = Arc::clone(&recording_mailbox);
    let background_2 = Arc::clone(&runtime.background_jobs);
    let closing_recover = Arc::clone(&runtime.closing);
    main_window
        .global::<AppState>()
        .on_recover_pending_recordings(move || {
            let Some(win) = weak_recover.upgrade() else { return; };
            if win.get_is_recording() || win.get_recording_finalizing()
                || recording_mailbox_recover.active_generation().is_some()
                || recording_mailbox_recover.is_finalizing()
            {
                show_capture_notice(&win, "Terminez la vidéo avant la récupération.", NOTICE_ERROR);
                return;
            }
            if closing_recover.load(Ordering::Acquire) || recovery_pending_callback.swap(true, Ordering::AcqRel) { return; }
            let directory = settings_snapshot(&settings_recover).capture_directory;
            let weak = weak_recover.clone();
            let settings = Arc::clone(&settings_recover);
            let library = Arc::clone(&library_recover);
            let session = Arc::clone(&session_recover);
            let pending = Arc::clone(&recovery_pending_callback);
            let recording_mailbox = Arc::clone(&recording_mailbox_recover);
            win.set_pending_recordings_count(0);
            let background_for_count = Arc::clone(&background_2);
            if !background_2.submit(move || {
                let result = if recording_mailbox.active_generation().is_some()
                    || recording_mailbox.is_finalizing()
                {
                    Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "Un enregistrement est actif."))
                } else {
                    pending_recording_paths(&directory).map(|paths| {
                    let mut recovered = 0usize;
                    let mut failed = 0usize;
                    let mut associations_pending = 0usize;
                    for (index, source) in paths.iter().enumerate() {
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |duration| duration.as_secs());
                        let name = format!("Iris_Recuperee_{stamp}_{index}.avi");
                        match recover_indexed_recording(source, &directory, &name) {
                            Ok(Some(committed)) => {
                                if committed.metadata_warning.is_some() {
                                    associations_pending += 1;
                                }
                                if let Err(error) = std::fs::remove_file(source) {
                                    eprintln!("[IrisScope] Nettoyage temporaire impossible : {error}");
                                }
                                recovered += 1;
                            }
                            Ok(None) | Err(_) => failed += 1,
                        }
                    }
                    (recovered, failed, associations_pending)
                    })
                };
                let weak_for_refresh = weak.clone();
                let pending_for_ui = Arc::clone(&pending);
                if weak.upgrade_in_event_loop(move |win| {
                    pending_for_ui.store(false, Ordering::Release);
                    if settings_snapshot(&settings).capture_directory != directory { return; }
                    match result {
                        Ok((recovered, failed, associations_pending)) => {
                            let mut message = format!("{recovered} vidéo(s) récupérée(s), {failed} non récupérable(s). Vérifiez leur dossier patient avant rattachement.");
                            if associations_pending > 0 {
                                let _ = write!(message, " {associations_pending} inscription(s) en bibliothèque en attente de reprise.");
                            }
                            show_capture_notice(&win, message, if failed == 0 && associations_pending == 0 { NOTICE_SUCCESS } else { NOTICE_ERROR });
                            library.invalidate_thumbnails();
                            refresh_library_in_background(&library, &settings, &session);
                        }
                        Err(error) => show_capture_notice(&win, format!("Récupération impossible : {error}"), NOTICE_ERROR),
                    }
                    refresh_pending_recording_count(weak_for_refresh, Arc::clone(&settings), &background_for_count);
                }).is_err() {
                    pending.store(false, Ordering::Release);
                }
            }) { recovery_pending_callback.store(false, Ordering::Release); crate::playback::show_capture_notice(&win, "Traitement disque en cours : réessayez dans un instant.", crate::config::NOTICE_ERROR); }
        });
}
