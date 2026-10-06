//! Exercises production Slint bindings with no camera or background workers.

use iriscope_app::test_support::ControllerHarness;
use iriscope_core::session::Eye;
use iriscope_core::settings::{AppTheme, PhysicalButtonBehavior, VideoQualityPreference};
use std::path::PathBuf;

fn viewer_open_and_close_reset_the_previous_transform(harness: &ControllerHarness) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("iriscope-viewer-reset-{unique}.jpg"));
    std::fs::write(&path, b"queued photo snapshot").expect("capture file");
    let version = iriscope_core::library::capture_file_version(&path).expect("capture version");
    harness.set_viewer_transform(200, 50.0, 25.0);
    harness.open_viewer_capture(&path, &version);
    assert_eq!(
        harness.viewer_transform(),
        (100, 0.0, 0.0),
        "opening a capture must discard an earlier zoom and pan"
    );
    harness.set_viewer_transform(150, 30.0, 20.0);
    harness.close_viewer();
    assert_eq!(
        harness.viewer_transform(),
        (100, 0.0, 0.0),
        "the close callback used after errors must also reset the view"
    );
    std::fs::remove_file(path).expect("cleanup");
}

fn physical_button_still_stops_video_after_mode_or_storage_changes() {
    let harness = ControllerHarness::new(PathBuf::from("/tmp/iriscope-physical-button-routing"))
        .expect("Slint window");
    harness.set_patient("Alice", "Martin", "42", Eye::Left);
    harness.publish_frame(1);
    harness.seed_active_stream();
    harness.window().set_is_video_mode(true);
    harness.press_hardware_button(PhysicalButtonBehavior::FollowMode);
    assert!(harness.active_recording_generation().is_some());
    harness.window().set_is_video_mode(false);
    harness.set_patient_action_pending(true);
    harness.press_hardware_button(PhysicalButtonBehavior::FollowMode);
    assert!(harness.active_recording_generation().is_none());
    assert!(harness.recording_is_finalizing());
    assert_eq!(
        harness.photo_work_count(),
        0,
        "stopping cannot accidentally take a photo"
    );
}

fn unavailable_shutter_sound_cannot_report_an_accepted_photo_as_failed() {
    let harness = ControllerHarness::new(PathBuf::from("/tmp/iriscope-optional-sound"))
        .expect("Slint window");
    harness.set_patient("Alice", "Martin", "42", Eye::Left);
    harness.publish_frame(17);
    harness.saturate_background_jobs();
    harness.trigger_capture();
    assert_eq!(harness.photo_work_count(), 1, "the photo is accepted");
    assert_eq!(
        harness
            .queued_capture()
            .expect("accepted photo")
            .sequence_number,
        17
    );
    assert!(
        !harness.capture_notice().0,
        "skipping the optional shutter sound cannot invite a duplicate capture"
    );
}

fn captures_wait_for_patient_actions(harness: &ControllerHarness) {
    harness.seed_active_stream();
    harness.set_patient_action_pending(true);
    harness.publish_frame(18);
    harness.trigger_capture();
    assert_eq!(
        harness.photo_work_count(),
        1,
        "patient action blocks new physical-button photos"
    );
    assert_eq!(
        harness
            .queued_capture()
            .expect("original photo")
            .sequence_number,
        17
    );
    let (visible, tone, message) = harness.capture_notice();
    assert!(visible);
    assert_eq!(tone, 0, "pending patient action is informational");
    assert!(message.contains("Traitement du dossier patient en cours"));

    harness.toggle_recording();
    assert!(
        harness.active_recording_generation().is_none(),
        "patient action also blocks recording start"
    );
    assert!(
        harness
            .capture_notice()
            .2
            .contains("Traitement du dossier patient en cours")
    );

    harness.set_patient_action_pending(false);
    harness.toggle_recording();
    let generation = harness
        .active_recording_generation()
        .expect("recording starts after patient action");
    harness.set_patient_action_pending(true);
    harness.toggle_recording();
    assert!(
        harness.active_recording_generation().is_none(),
        "pending patient action still permits recording stop"
    );
    assert!(harness.recording_is_finalizing());
    harness.complete_recording_finalization(generation);
    harness.set_patient_action_pending(false);
}

fn hidden_preview_stays_paused_when_unfreezing(harness: &ControllerHarness) {
    harness.set_preview_context(0, false, false);
    assert!(harness.preview_is_active());
    for (viewer_open, map_open) in [(true, false), (false, true)] {
        harness.set_preview_context(0, viewer_open, map_open);
        assert!(!harness.preview_is_active(), "overlay suspends decoding");
        harness.toggle_freeze();
        harness.toggle_freeze();
        assert!(
            !harness.preview_is_active(),
            "unfreezing cannot resume decoding behind an overlay"
        );
    }
    harness.set_preview_context(2, false, false);
    harness.toggle_freeze();
    harness.toggle_freeze();
    assert!(
        !harness.preview_is_active(),
        "library keeps decoding paused"
    );
    harness.set_preview_context(0, false, false);
    assert!(
        harness.preview_is_active(),
        "returning to live camera resumes preview"
    );
    harness.toggle_freeze();
    harness.set_preview_context(0, true, false);
    harness.set_preview_context(0, false, false);
    assert!(
        !harness.preview_is_active(),
        "closing a viewer cannot unfreeze a frozen camera"
    );
    harness.toggle_freeze();
    assert!(harness.preview_is_active());
}

fn unchanged_settings_retry_failed_saves_without_restarting_stream(harness: &ControllerHarness) {
    let before = harness.settings_observation();
    harness.report_settings_save_error();
    harness.select_theme(before.displayed_theme);
    let retried_theme = harness.settings_observation();
    assert_eq!(retried_theme.theme, before.theme);
    assert_eq!(retried_theme.displayed_theme, before.displayed_theme);
    assert_eq!(retried_theme.save_generation, before.save_generation + 1);
    assert!(retried_theme.save_dirty);
    assert!(!retried_theme.displayed_save_error);
    harness.select_theme(retried_theme.displayed_theme);
    assert_eq!(harness.settings_observation(), retried_theme);

    harness.update_video_quality(1);
    let balanced = harness.settings_observation();
    assert_eq!(balanced.video_quality, VideoQualityPreference::Balanced);
    assert_eq!(balanced.displayed_video_quality, 1);
    assert_eq!(balanced.save_generation, retried_theme.save_generation + 1);
    assert!(balanced.stream_restart_requested);
    harness.acknowledge_stream_restart();
    let restarted = harness.settings_observation();
    harness.update_video_quality(1);
    harness.update_video_quality(9);
    assert_eq!(harness.settings_observation(), restarted);

    harness.report_settings_save_error();
    harness.update_video_quality(1);
    let retried_quality = harness.settings_observation();
    assert_eq!(retried_quality.video_quality, restarted.video_quality);
    assert_eq!(retried_quality.displayed_video_quality, 1);
    assert_eq!(
        retried_quality.save_generation,
        restarted.save_generation + 1
    );
    assert!(retried_quality.save_dirty);
    assert!(!retried_quality.displayed_save_error);
    assert!(
        !retried_quality.stream_restart_requested,
        "retrying the existing quality only saves settings"
    );
    harness.update_video_quality(1);
    assert_eq!(harness.settings_observation(), retried_quality);
}

#[test]
fn production_callbacks_reach_capture_settings_and_viewer_state() {
    physical_button_still_stops_video_after_mode_or_storage_changes();
    unavailable_shutter_sound_cannot_report_an_accepted_photo_as_failed();
    // CI runs this test under Xvfb on Linux. No capture directory is created,
    // because the photo and settings workers are intentionally not started.
    let capture_directory = PathBuf::from("/tmp/iriscope-controller-bindings");
    let harness = ControllerHarness::new(capture_directory.clone()).expect("Slint window");

    harness.set_patient("Alice", "Martin", "42", Eye::Left);
    harness.trigger_capture();
    assert!(harness.queued_capture().is_none(), "no camera frame yet");

    harness.publish_frame(17);
    harness.trigger_capture();
    let queued = harness
        .queued_capture()
        .expect("capture queued by Slint binding");
    assert_eq!(queued.sequence_number, 17);
    assert_eq!(queued.directory, capture_directory);
    assert_eq!(queued.first_name, "Alice");
    assert_eq!(queued.last_name, "Martin");
    assert_eq!(queued.patient_id, Some(42));
    assert_eq!(queued.eye, Eye::Left);
    assert_eq!(queued.context_generation, 0);
    captures_wait_for_patient_actions(&harness);
    hidden_preview_stays_paused_when_unfreezing(&harness);

    let template = "{prenom}_{nom}_{oeil}_{date}";
    let before = harness.settings_observation();
    harness.update_filename_template(template);
    let after = harness.settings_observation();
    assert_eq!(after.filename_template, template);
    assert_eq!(after.displayed_filename_template, template);
    assert!(after.save_dirty, "settings save must be queued");
    assert_eq!(after.save_generation, before.save_generation + 1);

    harness.select_theme(2);
    let dark_theme = harness.settings_observation();
    assert_eq!(dark_theme.theme, AppTheme::Dark);
    assert_eq!(dark_theme.displayed_theme, 2);
    assert_eq!(dark_theme.save_generation, after.save_generation + 1);
    harness.select_theme(2);
    harness.select_theme(9);
    assert_eq!(
        harness.settings_observation(),
        dark_theme,
        "reselecting a theme or using an invalid index cannot enqueue another save"
    );
    unchanged_settings_retry_failed_saves_without_restarting_stream(&harness);

    harness.lookup_patient_by_dossier("dossier inconnu");
    let (visible, tone, message) = harness.capture_notice();
    assert!(visible, "patient errors must appear on the camera page");
    assert_eq!(tone, 2, "patient lookup failure uses the error notice tone");
    assert!(message.contains("Numéro de dossier invalide"));

    viewer_open_and_close_reset_the_previous_transform(&harness);
    harness.seed_playing_viewer();
    let before = harness.viewer_observation();
    assert_eq!(before.generation, 4);
    assert!(before.playing);
    assert_eq!(before.frame_count, 8);
    assert_eq!(before.requested_frame, Some(3));
    harness.close_viewer();
    let after = harness.viewer_observation();
    assert_eq!(after.generation, before.generation + 1);
    assert!(!after.playing);
    assert_eq!(after.frame_count, 0);
    assert_eq!(after.seek_epoch, before.seek_epoch + 1);
    assert_eq!(after.requested_frame, None);
    assert!(!after.displayed_playing);

    let recording_generation = harness.seed_recording();
    harness.show_window().expect("visible Slint window");
    assert!(harness.window_visible());
    assert_eq!(harness.photo_work_count(), 1);
    harness
        .request_window_close()
        .expect("close request with pending photo");
    assert!(harness.is_closing());
    assert!(harness.window_visible(), "close waits for the queued photo");
    assert!(harness.recording_is_finalizing());
    harness.publish_frame(18);
    harness.trigger_capture();
    assert_eq!(harness.photo_work_count(), 1, "no new photo after close");

    assert!(harness.complete_queued_photo_without_saving());
    assert_eq!(harness.photo_work_count(), 0);
    harness
        .request_window_close()
        .expect("close request while recording finalizes");
    assert!(harness.window_visible(), "close waits for the recording");
    harness.complete_recording_finalization(recording_generation);
    harness
        .request_window_close()
        .expect("close request after capture completion");
    assert!(!harness.window_visible());
}
