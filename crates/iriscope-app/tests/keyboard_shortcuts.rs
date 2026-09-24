use std::{cell::Cell, rc::Rc};

use slint::{
    ComponentHandle, LogicalPosition, PhysicalSize,
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

    click(&window, 850.0, 322.0);
    assert_eq!(photos.get(), 2);
    window.set_viewer_open(true);
    click(&window, 850.0, 322.0);
    assert_eq!(photos.get(), 2);
    window.set_viewer_open(false);

    control_key(&window, "2");
    assert_eq!(window.get_current_tab(), 1);
    control_key(&window, "p");
    control_key(&window, "r");
    control_key(&window, "f");
    assert_eq!((photos.get(), recordings.get(), freezes.get()), (2, 1, 1));

    window.set_viewer_open(true);
    click(&window, 160.0, 26.0);
    assert_eq!(window.get_current_tab(), 1);
    click(&window, 960.0, 42.0);
    assert!(!window.get_viewer_open());
    assert_eq!(viewer_closes.get(), 1);
    window.set_viewer_open(true);
    control_key(&window, "1");
    assert_eq!(window.get_current_tab(), 1);
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
}
