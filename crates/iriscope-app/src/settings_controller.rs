use crate::app_helpers::{
    physical_button_mode_index, settings_error, settings_file_path, settings_snapshot,
    video_quality_from_index, video_quality_index,
};
use crate::camera_queue::CameraSettingsSaveMailbox;
use crate::config::NOTICE_ERROR;
use crate::library_ui::clear_library_view;
use crate::library_worker::refresh_library_in_background;
use crate::patient_ui::clear_patient_candidates;
use crate::playback::show_capture_notice;
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::settings::{AppSettings, AppTheme, PhysicalButtonBehavior};
use iriscope_core::storage::{
    DEFAULT_FILENAME_TEMPLATE, filename_template_preserves_identity,
    filename_template_uses_supported_tokens,
};
use slint::ComponentHandle;
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn queue_settings_save(win: &MainWindow, mailbox: &CameraSettingsSaveMailbox, message: &str) {
    mailbox.mark_dirty();
    win.set_settings_feedback_is_error(false);
    win.set_settings_feedback(message.into());
}

pub(super) fn load_settings(
    main_window: &MainWindow,
) -> Result<(AppSettings, std::path::PathBuf), Box<dyn std::error::Error>> {
    let settings_path = settings_file_path();
    let (mut loaded_settings, recovered_settings) =
        match AppSettings::try_load_from_file(&settings_path) {
            Ok(settings) => (settings, None),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                let suffix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos();
                let backup = settings_path.with_extension(format!("invalid-{suffix}.json"));
                std::fs::rename(&settings_path, &backup)?;
                (AppSettings::default(), Some(backup))
            }
            Err(error) => return Err(error.into()),
        };
    if !filename_template_preserves_identity(&loaded_settings.filename_template)
        || !filename_template_uses_supported_tokens(&loaded_settings.filename_template)
    {
        DEFAULT_FILENAME_TEMPLATE.clone_into(&mut loaded_settings.filename_template);
    }
    loaded_settings.interface.normalize();
    let initial_save = loaded_settings.save_to_file(&settings_path);
    match (recovered_settings, initial_save) {
        (Some(backup), Ok(())) => settings_error(
            main_window,
            &format!(
                "Paramètres invalides : original conservé dans {}. Les valeurs par défaut sont chargées.",
                backup.display()
            ),
        ),
        (Some(backup), Err(error)) => settings_error(
            main_window,
            &format!(
                "Paramètres invalides : original conservé dans {}. Valeurs par défaut chargées, mais sauvegarde impossible : {error}",
                backup.display()
            ),
        ),
        (None, Err(error)) => settings_error(
            main_window,
            &format!("Impossible d'enregistrer les paramètres : {error}"),
        ),
        (None, Ok(())) => {}
    }
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
    main_window.set_settings_video_quality(video_quality_index(loaded_settings.video_quality));
    main_window.set_settings_theme(match loaded_settings.theme {
        AppTheme::System => 0,
        AppTheme::Light => 1,
        AppTheme::Dark => 2,
    });
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
    Ok((loaded_settings, settings_path))
}

#[allow(clippy::too_many_lines)]
pub(super) fn install(main_window: &MainWindow, runtime: &AppRuntime) {
    crate::file_dialogs::install(main_window, runtime);
    let settings = Arc::clone(&runtime.settings);
    let stream_restart_requested = Arc::clone(&runtime.stream_restart_requested);
    let camera_settings_save = Arc::clone(&runtime.camera_settings_save);
    let recovery_pending = Arc::clone(&runtime.recovery_pending);
    bind_capture_directory(main_window, runtime);

    let settings_template = Arc::clone(&settings);
    let save_template = Arc::clone(&camera_settings_save);
    let weak_template = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_update_filename_template(move |value| {
            let Some(win) = weak_template.upgrade() else {
                return;
            };
            let value = value.trim();
            if !filename_template_preserves_identity(value) {
                settings_error(&win, "Le modèle doit contenir {prenom}, {nom} et {oeil}.");
                return;
            }
            if !filename_template_uses_supported_tokens(value) {
                settings_error(
                    &win,
                    "Jetons autorisés : {prenom}, {nom}, {oeil}, {date}, {heure}.",
                );
                return;
            }
            if let Ok(mut guard) = settings_template.lock() {
                value.clone_into(&mut guard.filename_template);
            }
            win.set_settings_filename_template(value.into());
            queue_settings_save(&win, &save_template, "Enregistrement du modèle de nommage…");
        });

    let settings_button = Arc::clone(&settings);
    let save_button = Arc::clone(&camera_settings_save);
    let weak_button = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_cycle_physical_button_mode(move || {
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
            win.set_settings_button_mode(physical_button_mode_index(next));
            queue_settings_save(&win, &save_button, "Enregistrement de l'action du bouton…");
        });

    let settings_theme = Arc::clone(&settings);
    let save_theme = Arc::clone(&camera_settings_save);
    let weak_theme = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_select_theme(move |index| {
            let Some(win) = weak_theme.upgrade() else {
                return;
            };
            let next = match index {
                0 => AppTheme::System,
                1 => AppTheme::Light,
                2 => AppTheme::Dark,
                _ => return,
            };
            let changed = if let Ok(mut guard) = settings_theme.lock() {
                let changed = guard.theme != next;
                guard.theme = next;
                changed
            } else {
                return;
            };
            win.set_settings_theme(match next {
                AppTheme::System => 0,
                AppTheme::Light => 1,
                AppTheme::Dark => 2,
            });
            if changed || win.get_settings_feedback_is_error() {
                queue_settings_save(&win, &save_theme, "Enregistrement du thème…");
            }
        });

    let settings_quality = Arc::clone(&settings);
    let save_quality = Arc::clone(&camera_settings_save);
    let weak_quality = main_window.as_weak();
    let stream_restart_quality = Arc::clone(&stream_restart_requested);
    let recovery_pending_quality = Arc::clone(&recovery_pending);
    main_window
        .global::<AppState>()
        .on_update_video_quality(move |index| {
            let Some(win) = weak_quality.upgrade() else {
                return;
            };
            if win.get_is_recording()
                || win.get_recording_finalizing()
                || recovery_pending_quality.load(Ordering::Acquire)
            {
                settings_error(&win, "Terminez la vidéo avant de changer de qualité.");
                return;
            }
            let Some(quality) = video_quality_from_index(index) else {
                return;
            };
            let changed = if let Ok(mut settings) = settings_quality.lock() {
                let changed = settings.video_quality != quality;
                settings.video_quality = quality;
                changed
            } else {
                return;
            };
            win.set_settings_video_quality(index);
            if changed {
                stream_restart_quality.store(true, Ordering::Release);
            }
            if changed || win.get_settings_feedback_is_error() {
                queue_settings_save(
                    &win,
                    &save_quality,
                    if changed {
                        "Enregistrement de la qualité. Mise à jour du flux…"
                    } else {
                        "Enregistrement de la qualité…"
                    },
                );
            }
        });

    bind_reference_path(main_window, runtime, true);
    bind_reference_path(main_window, runtime, false);

    bind_directory_opener(main_window, runtime);
}

fn bind_reference_path(main_window: &MainWindow, runtime: &AppRuntime, is_map: bool) {
    let settings = Arc::clone(&runtime.settings);
    let save = Arc::clone(&runtime.camera_settings_save);
    let generation = if is_map {
        Arc::clone(&runtime.map_generation)
    } else {
        Arc::clone(&runtime.symbols_generation)
    };
    let mailbox = Arc::clone(&runtime.reference_mailbox);
    let weak = main_window.as_weak();
    let callback = move |value: slint::SharedString| {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let revision = generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let value = value.trim();
        if value.is_empty() {
            if let Ok(mut settings) = settings.lock() {
                if is_map {
                    settings.iridology_map_path = None;
                } else {
                    settings.iridology_symbols_path = None;
                }
            }
            if is_map {
                win.set_settings_iridology_map_path("".into());
                win.set_has_iridology_map(false);
                win.set_iridology_map_image(slint::Image::default());
            } else {
                win.set_settings_iridology_symbols_path("".into());
                win.set_has_iridology_symbols(false);
                win.set_iridology_symbols_image(slint::Image::default());
            }
            queue_settings_save(&win, &save, "Enregistrement de la référence…");
        } else {
            win.set_settings_feedback_is_error(false);
            win.set_settings_feedback("Chargement de la référence…".into());
            mailbox.request(crate::reference_worker::ReferenceRequest {
                path: value.into(),
                generation: revision,
                current_generation: Arc::clone(&generation),
                is_map,
                persist: true,
            });
        }
    };
    if is_map {
        main_window
            .global::<AppState>()
            .on_update_iridology_map_path(callback);
    } else {
        main_window
            .global::<AppState>()
            .on_update_iridology_symbols_path(callback);
    }
}

// The asynchronous result rechecks its directory generation and recording state before commit.
#[allow(clippy::too_many_lines)]
fn bind_capture_directory(main_window: &MainWindow, runtime: &AppRuntime) {
    let settings_directory = Arc::clone(&runtime.settings);
    let save_directory = Arc::clone(&runtime.camera_settings_save);
    let session_directory = Arc::clone(&runtime.active_session);
    let library_directory = Arc::clone(&runtime.library_mailbox);
    let photo_generation_directory = Arc::clone(&runtime.photo_context_generation);
    let weak_directory = main_window.as_weak();
    let recovery_pending_directory = Arc::clone(&runtime.recovery_pending);
    let directory_generation = Arc::clone(&runtime.directory_generation);
    let closing = Arc::clone(&runtime.closing);
    let background = Arc::clone(&runtime.background_jobs);
    main_window
        .global::<AppState>()
        .on_update_capture_directory(move |value| {
            let Some(win) = weak_directory.upgrade() else {
                return;
            };
            if closing.load(Ordering::Acquire) || win.global::<AppState>().get_storage_busy() {
                return;
            }
            if win.get_is_recording()
                || win.get_recording_finalizing()
                || recovery_pending_directory.load(Ordering::Acquire)
            {
                settings_error(&win, "Terminez la vidéo avant de changer de dossier.");
                return;
            }
            let value = value.trim();
            if value.is_empty() {
                settings_error(&win, "Indiquez un dossier pour les captures.");
                return;
            }
            let requested_directory = std::path::PathBuf::from(value);
            let revision = directory_generation
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1);
            let weak = weak_directory.clone();
            let settings_directory = Arc::clone(&settings_directory);
            let save_directory = Arc::clone(&save_directory);
            let session_directory = Arc::clone(&session_directory);
            let library_directory = Arc::clone(&library_directory);
            let photo_generation_directory = Arc::clone(&photo_generation_directory);
            let recovery_pending = Arc::clone(&recovery_pending_directory);
            let current_generation = Arc::clone(&directory_generation);
            let closing = Arc::clone(&closing);
            let background_for_count = Arc::clone(&background);
            win.set_settings_feedback_is_error(false);
            win.set_settings_feedback("Vérification du dossier…".into());
            if !background.submit(move || {
                if closing.load(Ordering::Acquire)
                    || current_generation.load(Ordering::Acquire) != revision
                {
                    return;
                }
                let result = std::fs::create_dir_all(&requested_directory)
                    .and_then(|()| std::fs::canonicalize(&requested_directory));
                let _ = weak.upgrade_in_event_loop(move |win| {
                    if closing.load(Ordering::Acquire)
                        || current_generation.load(Ordering::Acquire) != revision
                    {
                        return;
                    }
                    let directory = match result {
                        Ok(directory) => directory,
                        Err(error) => {
                            settings_error(&win, &format!("Dossier inaccessible : {error}"));
                            return;
                        }
                    };
                    if win.get_is_recording()
                        || win.get_recording_finalizing()
                        || recovery_pending.load(Ordering::Acquire)
                    {
                        settings_error(&win, "Terminez la vidéo avant de changer de dossier.");
                        return;
                    }
                    let changed =
                        settings_snapshot(&settings_directory).capture_directory != directory;
                    if let Ok(mut guard) = settings_directory.lock() {
                        guard.capture_directory.clone_from(&directory);
                    }
                    win.set_settings_capture_directory(
                        directory.to_string_lossy().to_string().into(),
                    );
                    crate::storage_controller::refresh_backup_status(
                        &win,
                        &settings_snapshot(&settings_directory),
                    );
                    queue_settings_save(
                        &win,
                        &save_directory,
                        "Enregistrement du dossier des captures…",
                    );
                    if changed {
                        photo_generation_directory.fetch_add(1, Ordering::AcqRel);
                        win.global::<AppState>().invoke_clear_comparison_image();
                        win.global::<AppState>().set_viewer_path("".into());
                        win.set_patient_id("".into());
                        win.set_patient_dossier_number("".into());
                        clear_patient_candidates(&win);
                        win.global::<AppState>().invoke_patient_name_edited();
                        win.global::<AppState>().invoke_session_changed();
                        library_directory.set_page(0);
                        clear_library_view(&win);
                        win.set_has_last_capture(false);
                        win.set_has_last_capture_thumbnail(false);
                        win.set_last_capture_thumbnail(slint::Image::default());
                        win.set_last_capture_path("".into());
                        win.set_last_capture_file_version("".into());
                        win.set_last_capture_file_name("".into());
                    }
                    refresh_library_in_background(
                        &library_directory,
                        &settings_directory,
                        &session_directory,
                    );

                    crate::library_controller::refresh_pending_recording_count(
                        win.as_weak(),
                        Arc::clone(&settings_directory),
                        &background_for_count,
                    );
                });
            }) {
                settings_error(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                );
            }
        });
}

fn bind_directory_opener(main_window: &MainWindow, runtime: &AppRuntime) {
    let settings = Arc::clone(&runtime.settings);
    let background = Arc::clone(&runtime.background_jobs);
    let weak = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_open_capture_directory(move || {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let directory = settings_snapshot(&settings).capture_directory;
            let weak = weak.clone();
            if !background.submit(move || {
                let result = std::fs::create_dir_all(&directory).and_then(|()| {
                    #[cfg(target_os = "linux")]
                    let command = "xdg-open";
                    #[cfg(target_os = "windows")]
                    let command = "explorer";
                    #[cfg(target_os = "macos")]
                    let command = "open";
                    std::process::Command::new(command)
                        .arg(&directory)
                        .spawn()
                        .map(|_| ())
                });
                if let Err(error) = result {
                    let _ = weak.upgrade_in_event_loop(move |win| {
                        show_capture_notice(
                            &win,
                            format!("Impossible d'ouvrir le dossier des captures : {error}"),
                            NOTICE_ERROR,
                        );
                    });
                }
            }) {
                settings_error(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                );
            }
        });
}
