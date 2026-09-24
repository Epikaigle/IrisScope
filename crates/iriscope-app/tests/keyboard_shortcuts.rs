use std::{cell::Cell, rc::Rc};

use slint::{
    ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};

slint::include_modules!();

fn control_key(window: &MainWindow, key: &str) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn camera_panel_and_image_popup_have_clickable_controls(window: &MainWindow) {
    click(window, 246.0, 26.0);
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
    click(window, 607.0, 124.0);
    assert!(!window.get_image_controls_open());

    click(window, 585.0, 26.0);
    assert!(!window.get_sidebar_visible());
    click(window, 585.0, 26.0);
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
        title: "Patient (Œil gauche)".into(),
        date_time: "24/09/2026 10:00".into(),
        eye_label: "Gauche".into(),
        thumbnail: slint::Image::default(),
        has_thumbnail: false,
        is_video: false,
        is_current_patient: true,
    }])));
    let opens = Rc::new(Cell::new(0));
    window.on_open_capture_file({
        let opens = opens.clone();
        move |path| {
            assert_eq!(path.as_str(), "/tmp/sample.jpg");
            opens.set(opens.get() + 1);
        }
    });

    click(&window, 960.0, 240.0);
    assert_eq!(window.get_library_view(), 1);
    click(&window, 950.0, 330.0);
    assert_eq!(opens.get(), 1);

    click(&window, 897.0, 240.0);
    assert_eq!(window.get_library_view(), 0);
    click(&window, 685.0, 102.0);
    assert!(window.get_library_map_open());
    click(&window, 956.0, 50.0);
    assert!(!window.get_library_map_open());
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

#[test]
fn shortcuts_follow_active_page_capture_guards_and_viewer() {
    let window = MainWindow::new().expect("create interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    camera_panel_and_image_popup_have_clickable_controls(&window);
    let photos = Rc::new(Cell::new(0));
    let recordings = Rc::new(Cell::new(0));
    let freezes = Rc::new(Cell::new(0));
    let viewer_closes = Rc::new(Cell::new(0));
    window.on_trigger_capture({
        let calls = photos.clone();
        move || calls.set(calls.get() + 1)
    });
    window.on_toggle_recording({
        let calls = recordings.clone();
        move || calls.set(calls.get() + 1)
    });
    window.on_toggle_freeze({
        let calls = freezes.clone();
        move || calls.set(calls.get() + 1)
    });
    window.on_close_viewer({
        let calls = viewer_closes.clone();
        move || calls.set(calls.get() + 1)
    });

    // Camera and session guards match the visible capture controls.
    control_key(&window, "p");
    control_key(&window, "r");
    assert_eq!((photos.get(), recordings.get()), (0, 0));
    window.set_is_streaming(true);
    window.set_patient_first_name("Ada".into());
    window.set_patient_last_name("Lovelace".into());
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

    click(&window, 160.0, 388.0);
    assert_eq!(photos.get(), 2);
    window.set_viewer_open(true);
    click(&window, 160.0, 388.0);
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
    click(&window, 160.0, 26.0);
    assert_eq!(window.get_current_tab(), 2);
    click(&window, 960.0, 42.0);
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
}
