use std::{cell::Cell, rc::Rc};

use slint::{
    ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};

use iriscope_app::ui::{AppState, LibraryItemData, MainWindow};

fn control_key(window: &MainWindow, key: &str) {
    modified_key(window, key, false);
}

fn modified_key(window: &MainWindow, key: &str, shift: bool) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    if shift {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
    }
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    if shift {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
    }
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn shifted_digits_navigate_without_bypassing_text_or_overlay_guards() {
    let window = MainWindow::new().expect("create shifted-shortcut interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    window.set_current_tab(4);
    // French AZERTY layouts require Shift to produce these digit characters.
    modified_key(&window, "1", true);
    assert_eq!(window.get_current_tab(), 0);
    modified_key(&window, "2", true);
    assert!(window.get_image_controls_open());
    escape(&window);
    modified_key(&window, "3", true);
    assert_eq!(window.get_current_tab(), 2);
    modified_key(&window, "4", true);
    assert!(window.get_library_map_open());
    escape(&window);
    modified_key(&window, "5", true);
    assert_eq!(window.get_current_tab(), 4);

    window.global::<AppState>().set_focused_text_fields(1);
    modified_key(&window, "1", true);
    assert_eq!(window.get_current_tab(), 4, "editing keeps its active page");
    window.global::<AppState>().set_focused_text_fields(0);
    window.set_viewer_open(true);
    modified_key(&window, "1", true);
    assert_eq!(
        window.get_current_tab(),
        4,
        "viewer keeps navigation blocked"
    );
    escape(&window);
    window.set_library_map_open(true);
    modified_key(&window, "1", true);
    assert_eq!(window.get_current_tab(), 4, "map keeps navigation blocked");
    escape(&window);
    modified_key(&window, "1", true);
    assert_eq!(window.get_current_tab(), 0);
}

fn escape(window: &MainWindow) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
}

fn camera_panel_and_image_popup_have_clickable_controls(window: &MainWindow) {
    // At 1024×720 the image toolbar has two rows below the preview.
    click(window, 875.0, 684.0);
    assert!(window.get_image_controls_open());
    window.set_is_streaming(true);
    window.set_zoom_level(150);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(365.0, 220.0),
        button: PointerEventButton::Left,
    });
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(430.0, 240.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(430.0, 240.0),
            button: PointerEventButton::Left,
        });
    assert!(window.get_zoom_pan_x().abs() < f32::EPSILON);
    assert!(window.get_zoom_pan_y().abs() < f32::EPSILON);
    window.set_zoom_level(100);
    window.set_is_streaming(false);
    click(window, 660.0, 116.0);
    assert!(!window.get_image_controls_open());

    click(window, 262.0, 104.0);
    assert!(!window.get_sidebar_visible());
    click(window, 105.0, 108.0);
    assert!(window.get_sidebar_visible());
}

fn library_modes_open_capture_and_close_map() {
    let window = MainWindow::new().expect("create library interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    window.set_current_tab(2);
    assert_eq!(window.get_current_tab(), 2);
    window.set_library_total_count(1);
    window.set_library_items(ModelRc::new(VecModel::from(vec![LibraryItemData {
        id: "sample".into(),
        file_path: "/tmp/sample.jpg".into(),
        file_version: "card-version".into(),
        title: "Patient (Œil gauche)".into(),
        date_time: "24/09/2026 10:00".into(),
        dossier_number: "D-000001".into(),
        eye_label: "Gauche".into(),
        thumbnail: slint::Image::default(),
        has_thumbnail: false,
        is_video: false,
        can_open_in_app: true,
        is_current_patient: true,
    }])));
    let opens = Rc::new(Cell::new(0));
    window.global::<AppState>().on_open_capture_file({
        let opens = opens.clone();
        move |path, version| {
            assert_eq!(path.as_str(), "/tmp/sample.jpg");
            assert_eq!(version.as_str(), "card-version");
            opens.set(opens.get() + 1);
        }
    });
    let external_opens = Rc::new(Cell::new(0));
    window.global::<AppState>().on_open_capture_externally({
        let external_opens = external_opens.clone();
        move |path, version| {
            assert_eq!(path.as_str(), "/tmp/sample.jpg");
            assert_eq!(version.as_str(), "card-version");
            external_opens.set(external_opens.get() + 1);
        }
    });

    // The view switch now shares the filter row below the page header.
    click(&window, 974.0, 168.0);
    assert_eq!(window.get_library_view(), 1);
    // Focus the card through its thumbnail, away from the open action buttons.
    click(&window, 40.0, 210.0);
    card_keyboard_selection_preserves_capture_identity(&window, " ");
    assert_eq!((opens.get(), external_opens.get()), (0, 0));
    click(&window, 875.0, 246.0);
    assert_eq!(opens.get(), 1);
    click(&window, 950.0, 246.0);
    assert_eq!(external_opens.get(), 1);

    click(&window, 924.0, 168.0);
    assert_eq!(window.get_library_view(), 0);
    click(&window, 200.0, 300.0);
    card_keyboard_selection_preserves_capture_identity(&window, "\n");
    assert_eq!((opens.get(), external_opens.get()), (1, 1));
    click(&window, 760.0, 112.0);
    assert!(window.get_library_map_open());
    click(&window, 960.0, 50.0);
    assert!(!window.get_library_map_open());
}

fn card_keyboard_selection_preserves_capture_identity(window: &MainWindow, key: &str) {
    // Retain card focus while clearing the previous pointer selection so that
    // stale selection cannot make the keyboard assertion pass.
    let state = window.global::<AppState>();
    assert_eq!(
        state.get_selected_library_path().as_str(),
        "/tmp/sample.jpg",
        "pointer should first select the card in view {}",
        window.get_library_view()
    );
    state.set_selected_library_path("".into());
    state.set_selected_library_file_version("".into());
    state.set_selected_library_dossier_number("".into());
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    assert_eq!(
        state.get_selected_library_path().as_str(),
        "/tmp/sample.jpg",
        "keyboard should select the card in view {}",
        window.get_library_view()
    );
    assert_eq!(
        state.get_selected_library_file_version().as_str(),
        "card-version"
    );
    assert_eq!(
        state.get_selected_library_dossier_number().as_str(),
        "D-000001"
    );
}

fn click(window: &MainWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

fn pending_patient_action_blocks_capture_but_allows_recording_stop() {
    let window = MainWindow::new().expect("create pending-patient interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    window.set_is_streaming(true);
    window.set_selected_eye(1);
    window.set_patient_action_pending(true);
    let photos = Rc::new(Cell::new(0));
    let recordings = Rc::new(Cell::new(0));
    window.global::<AppState>().on_trigger_capture({
        let calls = photos.clone();
        move || calls.set(calls.get() + 1)
    });
    window.global::<AppState>().on_toggle_recording({
        let calls = recordings.clone();
        move || calls.set(calls.get() + 1)
    });

    control_key(&window, "p");
    control_key(&window, "r");
    click(&window, 160.0, 638.0);
    assert_eq!((photos.get(), recordings.get()), (0, 0));

    // A pending patient operation must never prevent stopping an existing video.
    window.set_is_recording(true);
    control_key(&window, "r");
    click(&window, 160.0, 638.0);
    assert_eq!((photos.get(), recordings.get()), (0, 2));

    window.set_is_recording(false);
    window.set_patient_action_pending(false);
    control_key(&window, "p");
    control_key(&window, "r");
    click(&window, 160.0, 638.0);
    assert_eq!((photos.get(), recordings.get()), (2, 3));
}

#[test]
fn shortcuts_follow_active_page_capture_guards_and_viewer() {
    let window = MainWindow::new().expect("create interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    camera_panel_and_image_popup_have_clickable_controls(&window);
    let photos = Rc::new(Cell::new(0));
    let recordings = Rc::new(Cell::new(0));
    let freezes = Rc::new(Cell::new(0));
    let viewer_closes = Rc::new(Cell::new(0));
    window.global::<AppState>().on_trigger_capture({
        let calls = photos.clone();
        move || calls.set(calls.get() + 1)
    });
    window.global::<AppState>().on_toggle_recording({
        let calls = recordings.clone();
        move || calls.set(calls.get() + 1)
    });
    window.global::<AppState>().on_toggle_freeze({
        let calls = freezes.clone();
        move || calls.set(calls.get() + 1)
    });
    window.global::<AppState>().on_close_viewer({
        let calls = viewer_closes.clone();
        move || calls.set(calls.get() + 1)
    });

    // Camera and session guards match the visible capture controls.
    control_key(&window, "p");
    control_key(&window, "r");
    assert_eq!((photos.get(), recordings.get()), (0, 0));
    window.set_is_streaming(true);
    // Anonymous captures still require an eye, not a patient name.
    window.set_selected_eye(1);
    control_key(&window, "p");
    control_key(&window, "r");
    control_key(&window, "f");
    assert_eq!((photos.get(), recordings.get(), freezes.get()), (1, 1, 1));

    window.set_recording_finalizing(true);
    control_key(&window, "p");
    control_key(&window, "r");
    assert_eq!((photos.get(), recordings.get()), (1, 1));
    window.set_recording_finalizing(false);

    // Capture remains fixed at the bottom of the patient panel.
    click(&window, 160.0, 638.0);
    assert_eq!(photos.get(), 2);
    window.set_viewer_open(true);
    click(&window, 160.0, 638.0);
    assert_eq!(photos.get(), 2);
    window.set_viewer_open(false);

    control_key(&window, "2");
    assert_eq!(window.get_current_tab(), 0);
    assert!(window.get_image_controls_open());
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!window.get_image_controls_open());

    control_key(&window, "3");
    assert_eq!(window.get_current_tab(), 2);
    control_key(&window, "4");
    assert!(window.get_library_map_open());
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!window.get_library_map_open());
    control_key(&window, "p");
    control_key(&window, "r");
    control_key(&window, "f");
    assert_eq!((photos.get(), recordings.get(), freezes.get()), (2, 1, 1));

    window.set_viewer_open(true);
    click(&window, 222.0, 30.0);
    assert_eq!(window.get_current_tab(), 2);
    click(&window, 960.0, 48.0);
    assert!(!window.get_viewer_open());
    assert_eq!(viewer_closes.get(), 1);
    window.set_viewer_open(true);
    control_key(&window, "1");
    assert_eq!(window.get_current_tab(), 2);
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!window.get_viewer_open());
    assert_eq!(viewer_closes.get(), 2);
    control_key(&window, "1");
    assert_eq!(window.get_current_tab(), 0);
    library_modes_open_capture_and_close_map();
    pending_patient_action_blocks_capture_but_allows_recording_stop();
    shifted_digits_navigate_without_bypassing_text_or_overlay_guards();
}
