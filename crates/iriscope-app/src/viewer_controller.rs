use crate::config::NOTICE_ERROR;
use crate::playback::{ViewerOpenRequest, show_capture_notice, video_frame_for_progress};
use crate::runtime::AppRuntime;
use crate::ui::{AppState, MainWindow};
use iriscope_core::library::CaptureFileVersion;
use slint::ComponentHandle;
use std::sync::Arc;
use std::sync::atomic::Ordering;

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
            win.set_viewer_image(slint::Image::default());
            win.set_viewer_is_video(!is_photo);
            win.set_viewer_loading(true);
            win.set_viewer_open(true);
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
            win.set_viewer_loading(false);
            win.set_viewer_is_video(false);
            win.set_viewer_video_playing(false);
            win.set_viewer_video_progress(0.0);
            win.set_viewer_video_position("00:00".into());
            win.set_viewer_video_duration("00:00".into());
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
