use crate::app_helpers::settings_snapshot;
use crate::config::{NOTICE_ERROR, NOTICE_SUCCESS};
use crate::library_ui::clear_library_view;
use crate::library_worker::refresh_library_in_background;
use crate::playback::show_capture_notice;
use crate::recording_worker::pending_recording_paths;
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::library::{CaptureFileVersion, assign_capture_to_patient_if_unchanged};
use iriscope_core::settings::AppSettings;
use slint::ComponentHandle;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub(super) fn refresh_pending_recording_count(
    weak: slint::Weak<MainWindow>,
    settings: Arc<Mutex<AppSettings>>,
    background: &crate::background::BackgroundJobs,
) {
    let directory = settings_snapshot(&settings).capture_directory;
    background.submit(move || {
        let count = match pending_recording_paths(&directory) {
            Ok(paths) => i32::try_from(paths.len()).unwrap_or(i32::MAX),
            Err(error) => {
                eprintln!("[IrisScope] Recherche de vidéos interrompues impossible : {error}");
                return;
            }
        };
        let _ = weak.upgrade_in_event_loop(move |win| {
            if settings_snapshot(&settings).capture_directory == directory {
                win.set_pending_recordings_count(count);
            }
        });
    });
}

#[allow(clippy::too_many_lines)]
pub(super) fn install(main_window: &MainWindow, runtime: &AppRuntime) {
    let settings = Arc::clone(&runtime.settings);
    let library_mailbox = Arc::clone(&runtime.library_mailbox);
    let active_session = Arc::clone(&runtime.active_session);
    let library_mailbox_filter = Arc::clone(&library_mailbox);
    let settings_filter = Arc::clone(&settings);
    let session_filter = Arc::clone(&active_session);
    let weak_library_filter = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_library_filter_changed(move |filter| {
            let Some(win) = weak_library_filter.upgrade() else {
                return;
            };
            library_mailbox_filter.set_filter(filter);
            clear_library_view(&win);
            refresh_library_in_background(
                &library_mailbox_filter,
                &settings_filter,
                &session_filter,
            );
        });

    let library_mailbox_page = Arc::clone(&library_mailbox);
    let settings_page = Arc::clone(&settings);
    let session_page = Arc::clone(&active_session);
    let weak_library_page = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_library_page_changed(move |page| {
            let Some(win) = weak_library_page.upgrade() else {
                return;
            };
            library_mailbox_page.set_page(usize::try_from(page).unwrap_or(0));
            clear_library_view(&win);
            refresh_library_in_background(&library_mailbox_page, &settings_page, &session_page);
        });

    // Refresh library callback
    let settings_refresh = Arc::clone(&settings);
    let session_refresh = Arc::clone(&active_session);
    let library_refresh = Arc::clone(&library_mailbox);
    main_window
        .global::<AppState>()
        .on_refresh_library(move || {
            library_refresh.invalidate_thumbnails();
            refresh_library_in_background(&library_refresh, &settings_refresh, &session_refresh);
        });

    let weak_assign = main_window.as_weak();
    let settings_assign = Arc::clone(&settings);
    let library_assign = Arc::clone(&library_mailbox);
    let session_assign = Arc::clone(&active_session);
    let background_1 = Arc::clone(&runtime.background_jobs);
    let closing_assign = Arc::clone(&runtime.closing);
    let pending_assign = Arc::clone(&runtime.patient_action_pending);
    main_window
        .global::<AppState>()
        .on_assign_selected_capture_to_patient(move |raw_path, raw_version| {
            let Some(win) = weak_assign.upgrade() else {
                return;
            };
            if win.get_selected_library_path() != raw_path
                || win.get_selected_library_file_version() != raw_version
                || !win.get_selected_library_dossier_number().is_empty()
                || win.get_is_recording()
                || win.get_recording_finalizing()
            {
                return;
            }
            let Ok(patient_id) = win.get_patient_id().parse::<u64>() else {
                show_capture_notice(&win, "Sélectionnez un dossier patient.", NOTICE_ERROR);
                return;
            };
            let Ok(expected_version) = CaptureFileVersion::from_token(raw_version.as_str()) else {
                show_capture_notice(
                    &win,
                    "Actualisez la bibliothèque avant de rattacher cette capture.",
                    NOTICE_ERROR,
                );
                win.global::<AppState>().invoke_refresh_library();
                return;
            };
            let path = std::path::PathBuf::from(raw_path.as_str());
            let directory = settings_snapshot(&settings_assign).capture_directory;
            let weak = weak_assign.clone();
            let settings = Arc::clone(&settings_assign);
            let library = Arc::clone(&library_assign);
            let session = Arc::clone(&session_assign);
            if closing_assign.load(Ordering::Acquire) || pending_assign.swap(true, Ordering::AcqRel)
            {
                return;
            }
            win.set_patient_action_pending(true);
            let pending = Arc::clone(&pending_assign);
            if !background_1.submit(move || {
                let result = assign_capture_to_patient_if_unchanged(
                    &directory,
                    &path,
                    patient_id,
                    &expected_version,
                );
                let pending_for_ui = Arc::clone(&pending);
                if weak
                    .upgrade_in_event_loop(move |win| {
                        pending_for_ui.store(false, Ordering::Release);
                        win.set_patient_action_pending(false);
                        if settings_snapshot(&settings).capture_directory != directory {
                            return;
                        }
                        match result {
                            Ok(()) => {
                                win.set_selected_library_path("".into());
                                win.set_selected_library_file_version("".into());
                                win.set_selected_library_dossier_number("".into());
                                show_capture_notice(
                                    &win,
                                    "Capture rattachée au dossier patient.",
                                    NOTICE_SUCCESS,
                                );
                                library.invalidate_thumbnails();
                                refresh_library_in_background(&library, &settings, &session);
                            }
                            Err(error) => {
                                win.set_selected_library_path("".into());
                                win.set_selected_library_file_version("".into());
                                win.set_selected_library_dossier_number("".into());
                                show_capture_notice(
                                    &win,
                                    format!("Rattachement impossible : {error}"),
                                    NOTICE_ERROR,
                                );
                                library.invalidate_thumbnails();
                                refresh_library_in_background(&library, &settings, &session);
                            }
                        }
                    })
                    .is_err()
                {
                    pending.store(false, Ordering::Release);
                }
            }) {
                pending_assign.store(false, Ordering::Release);
                win.set_patient_action_pending(false);
                crate::playback::show_capture_notice(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                    crate::config::NOTICE_ERROR,
                );
            }
        });
}
