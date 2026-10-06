use crate::config::NOTICE_ERROR;
use crate::playback::{ViewerOpenRequest, show_capture_notice, video_frame_for_progress};
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::library::CaptureFileVersion;
use slint::{ComponentHandle, Model};
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn reset_viewer_presentation(win: &MainWindow) {
    crate::photo_tools::reset(win);
    win.set_viewer_zoom(100);
    win.set_viewer_pan_x(0.0);
    win.set_viewer_pan_y(0.0);
    win.set_viewer_image(slint::Image::default());
    win.set_viewer_loading(false);
    win.set_viewer_is_video(false);
    win.set_viewer_video_playing(false);
    win.set_viewer_video_progress(0.0);
    win.set_viewer_video_position("00:00".into());
    win.set_viewer_video_duration("00:00".into());
}

#[allow(clippy::too_many_lines)]
pub(super) fn install(main_window: &MainWindow, runtime: &AppRuntime) {
    let viewer_generation = Arc::clone(&runtime.viewer_generation);
    let viewer_video_playing = Arc::clone(&runtime.viewer_video_playing);
    let viewer_video_frame_count = Arc::clone(&runtime.viewer_video_frame_count);
    let viewer_video_seek_request = Arc::clone(&runtime.viewer_video_seek_request);
    let viewer_display_mailbox = Arc::clone(&runtime.viewer_display_mailbox);
    let viewer_open_mailbox = Arc::clone(&runtime.viewer_open_mailbox);
    // File reads, image decoding, and AVI indexing are serialized off the UI thread.
    let weak_viewer = main_window.as_weak();
    let viewer_generation_open = Arc::clone(&viewer_generation);
    let viewer_playing_open = Arc::clone(&viewer_video_playing);
    let viewer_frame_count_open = Arc::clone(&viewer_video_frame_count);
    let viewer_seek_open = Arc::clone(&viewer_video_seek_request);
    let viewer_display_open = Arc::clone(&viewer_display_mailbox);
    let viewer_mailbox_open = Arc::clone(&viewer_open_mailbox);
    main_window
        .global::<AppState>()
        .on_open_capture_file(move |file_path_str, raw_version| {
            let Some(win) = weak_viewer.upgrade() else {
                return;
            };
            if crate::photo_tools::defer_leave(&win, Some((&file_path_str, &raw_version))) {
                return;
            }
            let path = std::path::PathBuf::from(file_path_str.as_str());
            let Ok(expected_version) = CaptureFileVersion::from_token(raw_version.as_str()) else {
                show_capture_notice(
                    &win,
                    "Actualisez la bibliothèque avant d'ouvrir cette capture.",
                    NOTICE_ERROR,
                );
                return;
            };
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_photo = matches!(extension.as_str(), "jpg" | "jpeg" | "png");
            if !is_photo && !expected_version.supports_fast_validation() {
                show_capture_notice(
                    &win,
                    super::playback::SLOW_VIDEO_VALIDATION_NOTICE,
                    NOTICE_ERROR,
                );
                return;
            }
            if !is_photo && extension != "avi" {
                show_capture_notice(
                    &win,
                    "Ce format vidéo n'est pas encore lisible dans IrisScope.",
                    NOTICE_ERROR,
                );
                return;
            }

            let state = win.global::<AppState>();
            if !win.get_viewer_open() {
                state.set_viewer_return_fullscreen(state.get_iris_fullscreen());
                state.set_viewer_fullscreen(state.get_iris_fullscreen());
                state.set_iris_fullscreen(false);
            }
            state.set_viewer_file_version(raw_version.clone());
            state.set_viewer_path(file_path_str.clone());
            let row = win
                .get_library_items()
                .iter()
                .find(|row| row.file_path == file_path_str);
            state.set_viewer_dossier(
                row.as_ref()
                    .map_or_else(Default::default, |r| r.dossier_number.clone()),
            );
            state.set_viewer_eye_label(
                row.as_ref()
                    .map_or_else(Default::default, |r| r.eye_label.clone()),
            );
            let caption = win
                .get_library_items()
                .iter()
                .find(|row| row.file_path == file_path_str)
                .map_or_else(
                    || "Capture".to_owned(),
                    |row| {
                        format!(
                            "{} · {}{}",
                            row.eye_label,
                            row.date_time,
                            if row.dossier_number.is_empty() {
                                " · Sans dossier".to_owned()
                            } else {
                                format!(" · {}", row.dossier_number)
                            }
                        )
                    },
                );
            state.set_viewer_caption(caption.into());
            state.set_viewer_navigation_direction(0);
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
                expected_version,
            });
            reset_viewer_presentation(&win);
            state.set_comparison_enabled(
                is_photo
                    && state.get_comparison_available()
                    && state.get_comparison_path() != file_path_str,
            );
            win.set_viewer_is_video(!is_photo);
            win.set_viewer_loading(true);
            win.set_viewer_open(true);
        });

    let weak_keep = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_keep_comparison_image(move || {
            let Some(win) = weak_keep.upgrade() else {
                return;
            };
            let state = win.global::<AppState>();
            if !win.get_viewer_open() || win.get_viewer_loading() || win.get_viewer_is_video() {
                return;
            }
            state.set_comparison_image(if state.get_viewer_original().size().width > 0 {
                state.get_viewer_original()
            } else {
                win.get_viewer_image()
            });
            state.set_comparison_caption(state.get_viewer_caption());
            state.set_comparison_path(state.get_viewer_path());
            state.set_comparison_available(true);
            state.set_comparison_enabled(false);
            state.set_viewer_feedback("Photo gardée. Ouvrez une autre photo pour comparer.".into());
        });
    let weak_clear = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_clear_comparison_image(move || {
            let Some(win) = weak_clear.upgrade() else {
                return;
            };
            let state = win.global::<AppState>();
            state.set_comparison_enabled(false);
            state.set_comparison_available(false);
            state.set_comparison_image(slint::Image::default());
            state.set_comparison_caption("".into());
            state.set_comparison_path("".into());
        });
    let weak_navigate = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_navigate_viewer(move |direction| {
            let Some(win) = weak_navigate.upgrade() else {
                return;
            };
            navigate(&win, direction, false);
        });

    let weak_external = main_window.as_weak();
    let background_1 = Arc::clone(&runtime.background_jobs);
    main_window.global::<AppState>().on_open_capture_externally(
        move |file_path_str, raw_version| {
            let Some(win) = weak_external.upgrade() else {
                return;
            };
            let path = std::path::PathBuf::from(file_path_str.as_str());
            let Ok(expected_version) = CaptureFileVersion::from_token(raw_version.as_str()) else {
                show_capture_notice(
                    &win,
                    "Actualisez la bibliothèque avant d'ouvrir cette capture.",
                    NOTICE_ERROR,
                );
                return;
            };
            let weak = weak_external.clone();
            let cancellation = Arc::clone(&background_1);
            if !background_1.submit(move || {
                let result = (|| {
                    let file = super::playback::open_verified_capture_cancellable(
                        &path,
                        &expected_version,
                        &|| cancellation.is_closed(),
                    )?;
                    #[cfg(target_os = "linux")]
                    let command = "xdg-open";
                    #[cfg(target_os = "windows")]
                    let command = "explorer";
                    #[cfg(target_os = "macos")]
                    let command = "open";
                    std::process::Command::new(command).arg(&path).spawn()?;
                    super::playback::verify_open_capture_cancellable(
                        &file,
                        &path,
                        &expected_version,
                        &|| cancellation.is_closed(),
                    )
                })();
                let _ = weak.upgrade_in_event_loop(move |win| {
                    if let Err(error) = result {
                        show_capture_notice(
                            &win,
                            format!(
                                "Impossible d'ouvrir la capture avec l'application du PC : {error}"
                            ),
                            NOTICE_ERROR,
                        );
                        win.global::<AppState>().invoke_refresh_library();
                    }
                });
            }) {
                crate::playback::show_capture_notice(
                    &win,
                    "Traitement disque en cours : réessayez dans un instant.",
                    crate::config::NOTICE_ERROR,
                );
            }
        },
    );

    let viewer_generation_close = Arc::clone(&viewer_generation);
    let viewer_playing_close = Arc::clone(&viewer_video_playing);
    let viewer_frame_count_close = Arc::clone(&viewer_video_frame_count);
    let viewer_seek_close = Arc::clone(&viewer_video_seek_request);
    let viewer_display_close = Arc::clone(&viewer_display_mailbox);
    let weak_close_viewer = main_window.as_weak();
    main_window.global::<AppState>().on_close_viewer(move || {
        if let Some(win) = weak_close_viewer.upgrade() {
            if crate::photo_tools::defer_leave(&win, None) {
                return;
            }
            let state = win.global::<AppState>();
            state.set_viewer_fullscreen(false);
            state.set_iris_fullscreen(state.get_viewer_return_fullscreen());
        }
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
            win.set_viewer_open(false);
            win.global::<AppState>().set_viewer_navigation_direction(0);
            reset_viewer_presentation(&win);
        }
    });

    let viewer_playing_toggle = Arc::clone(&viewer_video_playing);
    let weak_toggle_viewer = main_window.as_weak();
    main_window
        .global::<AppState>()
        .on_toggle_viewer_video(move || {
            let playing = !viewer_playing_toggle.load(Ordering::Relaxed);
            viewer_playing_toggle.store(playing, Ordering::Relaxed);
            if let Some(win) = weak_toggle_viewer.upgrade() {
                win.set_viewer_video_playing(playing);
            }
        });

    let viewer_frame_count_seek = Arc::clone(&viewer_video_frame_count);
    let viewer_seek_request = Arc::clone(&viewer_video_seek_request);
    main_window
        .global::<AppState>()
        .on_seek_viewer_video(move |progress| {
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
}

/// Continues navigation after a page change while retaining the applied library filters.
pub(super) fn continue_navigation(win: &MainWindow) {
    let direction = win.global::<AppState>().get_viewer_navigation_direction();
    if direction != 0 {
        navigate(win, direction, true);
    }
}
fn navigate(win: &MainWindow, direction: i32, new_page: bool) {
    let state = win.global::<AppState>();
    if !win.get_viewer_open()
        || (!new_page && (win.get_viewer_loading() || win.get_library_loading()))
        || ![-1, 1].contains(&direction)
    {
        return;
    }
    if !win.get_library_error().is_empty() {
        state.set_viewer_navigation_direction(0);
        return;
    }
    let rows: Vec<_> = win.get_library_items().iter().collect();
    let current = rows
        .iter()
        .position(|row| row.file_path == state.get_viewer_path());
    let next = if direction > 0 {
        rows.iter().enumerate().find(|(index, row)| {
            row.can_open_in_app && (new_page || current.is_some_and(|current| *index > current))
        })
    } else {
        rows.iter().enumerate().rev().find(|(index, row)| {
            row.can_open_in_app && (new_page || current.is_some_and(|current| *index < current))
        })
    };
    if let Some((_, row)) = next {
        state.invoke_open_capture_file(row.file_path.clone(), row.file_version.clone());
        return;
    }
    let page = win.get_library_page() + direction;
    let last = (win.get_library_total_count() - 1).max(0)
        / i32::try_from(crate::config::LIBRARY_PAGE_SIZE).unwrap_or(i32::MAX);
    if page >= 0 && page <= last {
        state.set_viewer_navigation_direction(direction);
        state.invoke_library_page_changed(page);
    } else {
        state.set_viewer_navigation_direction(0);
        show_capture_notice(
            win,
            if direction > 0 {
                "Dernière capture lisible de cette recherche."
            } else {
                "Première capture lisible de cette recherche."
            },
            crate::config::NOTICE_INFO,
        );
    }
}
