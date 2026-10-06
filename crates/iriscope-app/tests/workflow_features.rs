//! User-visible query, capture guards, navigation and photo comparison through production bindings.
use iriscope_app::{
    test_support::ControllerHarness,
    ui::{AppState, LibraryItemData},
};
use iriscope_core::{
    library::{LibraryQuery, capture_file_version},
    session::Eye,
};
use slint::{ComponentHandle, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
fn verify_library_queries(harness: &ControllerHarness) {
    let win = harness.window();
    let state = win.global::<AppState>();
    state.set_library_query_dossier("D-000042".into());
    state.set_library_query_from("2026-09-01".into());
    state.set_library_query_to("2026-10-05".into());
    state.set_library_query_eye(1);
    state.invoke_apply_library_query();
    let applied = LibraryQuery::parse("D-000042", "2026-09-01", "2026-10-05", 1, 0).unwrap();
    assert_eq!(harness.applied_library_query(), applied);
    state.set_library_query_from("2026-02-30".into());
    state.invoke_apply_library_query();
    assert!(!state.get_library_query_error().is_empty());
    assert_eq!(
        harness.applied_library_query(),
        applied,
        "invalid input preserves the previous search"
    );
    state.set_library_query_from("2026-09-01".into());
    state.invoke_apply_library_query();
    assert!(state.get_library_query_error().is_empty());
}

fn verify_editor_guards_and_keyboard_focus(harness: &ControllerHarness) {
    let win = harness.window();
    let state = win.global::<AppState>();
    state.set_patient_edit_open(true);
    harness.trigger_capture();
    assert!(harness.queued_capture().is_none());
    harness.toggle_recording();
    assert!(harness.active_recording_generation().is_none());
    state.set_focused_text_fields(1);
    state.invoke_dismiss_top_overlay();
    assert!(!state.get_patient_edit_open());
    assert_eq!(state.get_focused_text_fields(), 0);
    for (pressed, text) in [
        (true, slint::platform::Key::Control.into()),
        (true, "3".into()),
        (false, "3".into()),
        (false, slint::platform::Key::Control.into()),
    ] {
        win.window().dispatch_event(if pressed {
            slint::platform::WindowEvent::KeyPressed { text }
        } else {
            slint::platform::WindowEvent::KeyReleased { text }
        });
    }
    assert_eq!(
        win.get_current_tab(),
        2,
        "closing the editor restores keyboard navigation"
    );
}

fn verify_capture_receipt(harness: &ControllerHarness, row: &LibraryItemData) {
    let win = harness.window();
    win.window().set_size(slint::PhysicalSize::new(800, 600));
    win.set_current_tab(0);
    win.set_last_capture_path(row.file_path.clone());
    win.set_last_capture_file_version(row.file_version.clone());
    win.set_has_last_capture(true);
    win.set_capture_notice_tone(1);
    win.set_last_capture_message("Photo enregistrée : Marie_Christine_de_la_Roche_Saint_Andre_Gauche_2026-10-05_14-42-16.jpg".into());
    win.set_show_last_capture(true);
    for pressed in [true, false] {
        let position = slint::LogicalPosition::new(691.0, 123.0);
        let button = slint::platform::PointerEventButton::Left;
        win.window().dispatch_event(if pressed {
            slint::platform::WindowEvent::PointerPressed { position, button }
        } else {
            slint::platform::WindowEvent::PointerReleased { position, button }
        });
    }
    assert!(win.get_viewer_open(), "Voir opens the saved photo");
    assert!(
        !win.get_show_last_capture(),
        "the receipt must not cover viewer controls"
    );
    win.global::<AppState>().invoke_close_viewer();
}

#[test]
fn queries_capture_guards_and_comparison_use_the_production_callbacks() {
    let directory = std::env::temp_dir().join(format!(
        "iriscope-workflow-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let harness = ControllerHarness::new(directory.clone()).unwrap();
    verify_library_queries(&harness);
    let win = harness.window();
    let state = win.global::<AppState>();

    harness.set_patient("Alice", "Martin", "42", Eye::Left);
    harness.publish_frame(1);
    state.set_storage_busy(true);
    harness.trigger_capture();
    assert!(harness.queued_capture().is_none());
    state.set_storage_busy(false);
    state.set_storage_critical(true);
    harness.trigger_capture();
    assert!(harness.queued_capture().is_none());
    harness.seed_active_stream();
    harness.toggle_recording();
    assert!(harness.active_recording_generation().is_none());
    state.set_storage_critical(false);
    let recording = harness.seed_recording();
    state.set_storage_critical(true);
    harness.toggle_recording();
    assert!(
        harness.active_recording_generation().is_none(),
        "critical storage must still allow stopping"
    );
    harness.complete_recording_finalization(recording);
    state.set_storage_critical(false);
    verify_editor_guards_and_keyboard_focus(&harness);

    let mut rows = Vec::new();
    for number in 0..3 {
        let path = directory.join(format!("capture-{number}.jpg"));
        fs::write(&path, [number; 12]).unwrap();
        rows.push(LibraryItemData {
            file_path: path.to_str().unwrap().into(),
            file_version: capture_file_version(&path).unwrap().token().into(),
            can_open_in_app: true,
            eye_label: "Œil gauche".into(),
            date_time: format!("2026-10-0{} 12:00", number + 1).into(),
            dossier_number: "D-000042".into(),
            ..Default::default()
        });
    }
    win.set_library_items(ModelRc::new(VecModel::from(rows.clone())));
    win.set_library_loading(false);
    win.set_library_total_count(3);
    state.invoke_open_capture_file(rows[0].file_path.clone(), rows[0].file_version.clone());
    win.set_viewer_loading(false); // Simulate completion of the verified reader, which the harness does not start.
    let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&[10_u8, 20, 30], 1, 1);
    win.set_viewer_image(slint::Image::from_rgb8(pixels));
    state.invoke_keep_comparison_image();
    assert!(state.get_comparison_available());
    assert!(!state.get_comparison_enabled());
    state.invoke_navigate_viewer(1);
    assert_eq!(state.get_viewer_path(), rows[1].file_path);
    assert!(state.get_comparison_enabled());
    assert_eq!(state.get_comparison_path(), rows[0].file_path);
    win.set_viewer_loading(false);
    state.invoke_navigate_viewer(-1);
    assert_eq!(state.get_viewer_path(), rows[0].file_path);
    assert!(
        !state.get_comparison_enabled(),
        "the reference is not compared with itself"
    );
    state.invoke_clear_comparison_image();
    assert!(!state.get_comparison_available());
    state.invoke_close_viewer();
    verify_capture_receipt(&harness, &rows[0]);
    drop(harness);
    fs::remove_dir_all(directory).unwrap();
}
