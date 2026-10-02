use crate::app_helpers::settings_snapshot;
use crate::config::{NOTICE_ERROR, NOTICE_SUCCESS};
use crate::library_ui::clear_library_view;
use crate::library_worker::refresh_library_in_background;
use crate::patient_ui::{PatientSearchRequest, clear_patient_candidates};
use crate::playback::show_capture_notice;
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::library::{PatientRecord, create_patient, get_patient, search_patients};
use iriscope_core::session::{CaptureSession, Eye};
use slint::ComponentHandle;
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn patient_error(win: &MainWindow, message: &str) {
    show_capture_notice(win, message, NOTICE_ERROR);
}

fn select_patient_in_window(win: &MainWindow, record: PatientRecord) {
    win.set_patient_first_name(record.first_name.into());
    win.set_patient_last_name(record.last_name.into());
    win.set_patient_id(record.id.to_string().into());
    win.set_patient_dossier_number(record.dossier_number.into());
    win.set_patient_search_pending(false);
    clear_patient_candidates(win);
    win.global::<AppState>().invoke_session_changed();
}

#[allow(clippy::too_many_lines)]
pub(super) fn install(main_window: &MainWindow, runtime: &AppRuntime) {
    let settings = Arc::clone(&runtime.settings);
    let library_mailbox = Arc::clone(&runtime.library_mailbox);
    let photo_context_generation = Arc::clone(&runtime.photo_context_generation);
    let active_session = Arc::clone(&runtime.active_session);
    let patient_search_generation = Arc::clone(&runtime.patient_search_generation);
    let patient_search_mailbox = Arc::clone(&runtime.patient_search_mailbox);
    let patient_action_pending = Arc::clone(&runtime.patient_action_pending);
    let patient_search_mailbox_edited = Arc::clone(&patient_search_mailbox);
    let patient_search_generation_edited = Arc::clone(&patient_search_generation);
    let patient_search_settings = Arc::clone(&settings);
    let weak_patient_edited = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_patient_name_edited(move || {
            let Some(win) = weak_patient_edited.upgrade() else {
                return;
            };
            let generation = patient_search_generation_edited
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1);
            clear_patient_candidates(&win);
            let first_name = win.get_patient_first_name().trim().to_owned();
            let last_name = win.get_patient_last_name().trim().to_owned();
            if first_name.is_empty() || last_name.is_empty() {
                win.set_patient_search_pending(false);
                return;
            }
            win.set_patient_search_pending(true);
            patient_search_mailbox_edited.request(PatientSearchRequest {
                directory: settings_snapshot(&patient_search_settings).capture_directory,
                first_name,
                last_name,
                generation,
            });
        });

    let weak_patient_select = main_window.as_weak();
    let patient_select_settings = Arc::clone(&settings);
    let patient_select_pending = Arc::clone(&patient_action_pending);
    let closing_select = Arc::clone(&runtime.closing);
    let background_1 = Arc::clone(&runtime.background_jobs);
    main_window
        .global::<AppState>()
        .on_select_patient(move |raw_id| {
            let Some(win) = weak_patient_select.upgrade() else {
                return;
            };
            let Ok(id) = raw_id.parse::<u64>() else {
                patient_error(&win, "Numéro de dossier invalide.");
                return;
            };
            if closing_select.load(Ordering::Acquire)
                || patient_select_pending.swap(true, Ordering::AcqRel)
            {
                return;
            }
            win.set_patient_action_pending(true);
            let directory = settings_snapshot(&patient_select_settings).capture_directory;
            let first_name = win.get_patient_first_name().trim().to_owned();
            let last_name = win.get_patient_last_name().trim().to_owned();
            let settings = Arc::clone(&patient_select_settings);
            let pending = Arc::clone(&patient_select_pending);
            let weak = weak_patient_select.clone();
            if !background_1.submit(move || {
                let result = search_patients(&directory, &first_name, &last_name)
                    .map(|records| records.into_iter().find(|record| record.id == id));
                let pending_for_ui = Arc::clone(&pending);
                if weak
                    .upgrade_in_event_loop(move |win| {
                        pending_for_ui.store(false, Ordering::Release);
                        win.set_patient_action_pending(false);
                        if settings_snapshot(&settings).capture_directory != directory
                            || win.get_patient_first_name().trim() != first_name
                            || win.get_patient_last_name().trim() != last_name
                        {
                            return;
                        }
                        match result {
                            Ok(Some(record)) => {
                                select_patient_in_window(&win, record);
                            }
                            Ok(None) => {
                                patient_error(&win, "Ce dossier ne correspond plus au nom saisi.");
                                win.global::<AppState>().invoke_patient_name_edited();
                            }
                            Err(error) => patient_error(
                                &win,
                                &format!("Impossible d'ouvrir ce dossier patient : {error}"),
                            ),
                        }
                    })
                    .is_err()
                {
                    pending.store(false, Ordering::Release);
                }
            }) {
                patient_select_pending.store(false, Ordering::Release);
                win.set_patient_action_pending(false);
                crate::playback::show_capture_notice(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                    crate::config::NOTICE_ERROR,
                );
            }
        });

    let weak_patient_create = main_window.as_weak();
    let patient_create_settings = Arc::clone(&settings);
    let patient_create_pending = Arc::clone(&patient_action_pending);
    let closing_create = Arc::clone(&runtime.closing);
    let background_2 = Arc::clone(&runtime.background_jobs);
    main_window.global::<AppState>().on_create_patient(move || {
        let Some(win) = weak_patient_create.upgrade() else {
            return;
        };
        let first_name = win.get_patient_first_name().trim().to_owned();
        let last_name = win.get_patient_last_name().trim().to_owned();
        if first_name.is_empty() || last_name.is_empty() {
            patient_error(&win, "Renseignez le prénom et le nom du patient.");
            return;
        }
        if closing_create.load(Ordering::Acquire)
            || patient_create_pending.swap(true, Ordering::AcqRel)
        {
            return;
        }
        if win.get_patient_search_pending() {
            patient_create_pending.store(false, Ordering::Release);
            return;
        }
        win.set_patient_action_pending(true);
        let directory = settings_snapshot(&patient_create_settings).capture_directory;
        let settings = Arc::clone(&patient_create_settings);
        let pending = Arc::clone(&patient_create_pending);
        let weak = weak_patient_create.clone();
        if !background_2.submit(move || {
            let result = create_patient(&directory, &first_name, &last_name);
            let pending_for_ui = Arc::clone(&pending);
            if weak
                .upgrade_in_event_loop(move |win| {
                    pending_for_ui.store(false, Ordering::Release);
                    win.set_patient_action_pending(false);
                    if settings_snapshot(&settings).capture_directory != directory
                        || win.get_patient_first_name().trim() != first_name
                        || win.get_patient_last_name().trim() != last_name
                    {
                        return;
                    }
                    match result {
                        Ok(record) => {
                            let dossier_number = record.dossier_number.clone();
                            select_patient_in_window(&win, record);
                            show_capture_notice(
                                &win,
                                format!("Dossier {dossier_number} créé."),
                                NOTICE_SUCCESS,
                            );
                        }
                        Err(error) => patient_error(
                            &win,
                            &format!("Impossible de créer le dossier patient : {error}"),
                        ),
                    }
                })
                .is_err()
            {
                pending.store(false, Ordering::Release);
            }
        }) {
            patient_create_pending.store(false, Ordering::Release);
            win.set_patient_action_pending(false);
            crate::playback::show_capture_notice(
                &win,
                "Traitement disque en cours : réessayez dans un instant.",
                crate::config::NOTICE_ERROR,
            );
        }
    });

    let weak_patient_lookup = main_window.as_weak();
    let patient_lookup_settings = Arc::clone(&settings);
    let patient_lookup_pending = Arc::clone(&patient_action_pending);
    let closing_lookup = Arc::clone(&runtime.closing);
    let background_3 = Arc::clone(&runtime.background_jobs);
    main_window
        .global::<AppState>()
        .on_lookup_patient_by_dossier(move |raw_number| {
            let Some(win) = weak_patient_lookup.upgrade() else {
                return;
            };
            let raw_number = raw_number.trim();
            let digits = raw_number
                .strip_prefix("D-")
                .or_else(|| raw_number.strip_prefix("d-"))
                .unwrap_or(raw_number);
            let Ok(id) = digits.parse::<u64>() else {
                patient_error(&win, "Numéro de dossier invalide (exemple : D-000123).");
                return;
            };
            if closing_lookup.load(Ordering::Acquire)
                || patient_lookup_pending.swap(true, Ordering::AcqRel)
            {
                return;
            }
            win.set_patient_action_pending(true);
            let directory = settings_snapshot(&patient_lookup_settings).capture_directory;
            let settings = Arc::clone(&patient_lookup_settings);
            let pending = Arc::clone(&patient_lookup_pending);
            let weak = weak_patient_lookup.clone();
            if !background_3.submit(move || {
                let result = get_patient(&directory, id);
                let pending_for_ui = Arc::clone(&pending);
                if weak
                    .upgrade_in_event_loop(move |win| {
                        pending_for_ui.store(false, Ordering::Release);
                        win.set_patient_action_pending(false);
                        if settings_snapshot(&settings).capture_directory != directory {
                            return;
                        }
                        match result {
                            Ok(Some(record)) => select_patient_in_window(&win, record),
                            Ok(None) => patient_error(&win, "Aucun dossier ne porte ce numéro."),
                            Err(error) => patient_error(
                                &win,
                                &format!("Ouverture du dossier impossible : {error}"),
                            ),
                        }
                    })
                    .is_err()
                {
                    pending.store(false, Ordering::Release);
                }
            }) {
                patient_lookup_pending.store(false, Ordering::Release);
                win.set_patient_action_pending(false);
                crate::playback::show_capture_notice(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                    crate::config::NOTICE_ERROR,
                );
            }
        });

    let session_changed_session = Arc::clone(&active_session);
    let session_changed_settings = Arc::clone(&settings);
    let session_changed_mailbox = Arc::clone(&library_mailbox);
    let session_changed_generation = Arc::clone(&photo_context_generation);
    let weak_session_changed = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_session_changed(move || {
            let Some(win) = weak_session_changed.upgrade() else {
                return;
            };
            let eye = match win.get_selected_eye() {
                1 => Eye::Left,
                2 => Eye::Right,
                _ => Eye::Unspecified,
            };
            let mut session = CaptureSession::new(
                win.get_patient_first_name().to_string(),
                win.get_patient_last_name().to_string(),
                eye,
            );
            session.set_patient_id(win.get_patient_id().parse::<u64>().ok());
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
                win.set_has_last_capture_thumbnail(false);
                win.set_last_capture_thumbnail(slint::Image::default());
                win.set_last_capture_path("".into());
                win.set_last_capture_file_version("".into());
                win.set_last_capture_file_name("".into());
                win.set_session_photo_count(0);
                refresh_library_in_background(
                    &session_changed_mailbox,
                    &session_changed_settings,
                    &session_changed_session,
                );
            }
        });

    // Session clearing
    let weak_session = main_window.as_weak();
    let session_clear = Arc::clone(&active_session);
    let settings_clear = Arc::clone(&settings);
    let library_clear = Arc::clone(&library_mailbox);
    let photo_generation_clear = Arc::clone(&photo_context_generation);
    main_window.global::<AppState>().on_clear_session(move || {
        let Some(win) = weak_session.upgrade() else {
            return;
        };
        win.set_patient_first_name("".into());
        win.set_patient_last_name("".into());
        win.set_patient_id("".into());
        win.set_patient_dossier_number("".into());
        clear_patient_candidates(&win);
        win.set_patient_search_pending(false);
        win.set_patient_action_pending(false);
        win.set_selected_eye(0);
        win.set_session_photo_count(0);
        win.set_has_last_capture(false);
        win.set_has_last_capture_thumbnail(false);
        win.set_last_capture_thumbnail(slint::Image::default());
        win.set_last_capture_path("".into());
        win.set_last_capture_file_version("".into());
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
}
