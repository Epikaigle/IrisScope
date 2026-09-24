use std::{cell::Cell, rc::Rc};

use slint::{
    ComponentHandle, LogicalPosition, PhysicalSize,
    platform::{Key, PointerEventButton, WindowEvent},
};

slint::include_modules!();

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
fn wheel_zoom_and_fullscreen_capture_work_from_the_live_view() {
    let window = MainWindow::new().expect("create camera interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    window.set_is_streaming(true);
    let image_position = LogicalPosition::new(700.0, 350.0);

    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: image_position,
            delta_x: 0.0,
            delta_y: 60.0,
        });
    assert_eq!(window.get_zoom_level(), 110);
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: image_position,
            delta_x: 0.0,
            delta_y: -60.0,
        });
    assert_eq!(window.get_zoom_level(), 100);

    window.set_zoom_level(200);
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: image_position,
            delta_x: 0.0,
            delta_y: 60.0,
        });
    assert_eq!(window.get_zoom_level(), 200);
    window.set_zoom_level(100);

    window.set_patient_first_name("Ada".into());
    window.set_patient_last_name("Lovelace".into());
    window.set_selected_eye(1);
    let photos = Rc::new(Cell::new(0));
    window.on_trigger_capture({
        let photos = photos.clone();
        move || photos.set(photos.get() + 1)
    });

    click(&window, 930.0, 108.0);
    assert!(window.get_iris_fullscreen());
    assert!(window.window().is_fullscreen());
    click(&window, 512.0, 676.0);
    assert_eq!(photos.get(), 1);

    click(&window, 930.0, 35.0);
    assert!(!window.get_iris_fullscreen());
    click(&window, 930.0, 108.0);
    assert!(window.get_iris_fullscreen());

    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!window.get_iris_fullscreen());
    assert!(!window.window().is_fullscreen());
}
