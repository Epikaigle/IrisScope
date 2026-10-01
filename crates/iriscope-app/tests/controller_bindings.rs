//! Exercises production Slint bindings with no camera or background workers.

use iriscope_app::test_support::ControllerHarness;
use iriscope_core::session::Eye;
use std::path::PathBuf;

#[test]
fn production_callbacks_reach_capture_settings_and_viewer_state() {
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

    let template = "{prenom}_{nom}_{oeil}_{date}";
    let before = harness.settings_observation();
    harness.update_filename_template(template);
    let after = harness.settings_observation();
    assert_eq!(after.filename_template, template);
    assert_eq!(after.displayed_filename_template, template);
    assert!(after.save_dirty, "settings save must be queued");
    assert_eq!(after.save_generation, before.save_generation + 1);

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
