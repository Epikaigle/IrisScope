use std::{cell::Cell, rc::Rc, thread, time::Duration};

use slint::{
    ComponentHandle, LogicalPosition, PhysicalSize,
    platform::{Key, PointerEventButton, WindowEvent},
};

use iriscope_app::ui::{AppState, MainWindow};

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

fn wait_for_controls_timeout(window: &MainWindow) {
    // The headless backend needs an explicit timer pump after elapsed wall time.
    thread::sleep(Duration::from_millis(1100));
    slint::platform::update_timers_and_animations();
    assert!(
        !window.global::<AppState>().get_viewport_controls_visible(),
        "fullscreen overlays should time out after inactivity"
    );
}

fn escape(window: &MainWindow) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
}

fn permanent_toolbar_and_fullscreen_exit_follow_overlay_visibility(window: &MainWindow) {
    let image_position = LogicalPosition::new(700.0, 350.0);

    wait_for_controls_timeout(window);
    // At this width the permanent toolbar has two rows; fullscreen is last.
    click(window, 955.0, 688.0);
    assert!(
        window.get_iris_fullscreen(),
        "the windowed toolbar should remain usable after overlay inactivity"
    );
    window
        .global::<AppState>()
        .set_viewport_controls_visible(false);
    click(window, 930.0, 35.0);
    assert!(
        window.get_iris_fullscreen(),
        "the fullscreen exit overlay should be hidden when controls are hidden"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: image_position,
    });
    assert!(window.global::<AppState>().get_viewport_controls_visible());
    click(window, 930.0, 35.0);
    assert!(
        !window.get_iris_fullscreen(),
        "moving over the image should reveal the fullscreen exit control"
    );
}

fn wheel_zoom_and_resize_keep_image_pan_bounded(window: &MainWindow) {
    let image_position = LogicalPosition::new(700.0, 350.0);
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            // Preview starts at y72: its upper edge must remain interactive
            // when the image height is reduced to make room for the toolbar.
            position: LogicalPosition::new(700.0, 80.0),
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

    window.window().set_size(PhysicalSize::new(1920, 1080));
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1000.0, 350.0),
    });
    window.set_zoom_pan_x(700.0);
    window.set_zoom_pan_y(400.0);
    assert!(window.get_zoom_pan_x() > 232.0);
    assert!(window.get_zoom_pan_y() > 212.0);
    window.window().set_size(PhysicalSize::new(800, 600));
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(600.0, 300.0),
    });
    // Preview: 800 - 24 padding - 300 sidebar - 12 gap; image: 600 -
    // 60 header - 24 padding - 92 toolbar. At 200%, pan is half that size.
    assert!(
        window.get_zoom_pan_x().abs() <= 232.1,
        "shrinking the preview should clamp horizontal pan"
    );
    assert!(
        window.get_zoom_pan_y().abs() <= 212.1,
        "shrinking the preview should clamp vertical pan"
    );
    window.window().set_size(PhysicalSize::new(1024, 720));
    window.set_zoom_level(100);
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: image_position,
    });
    assert!(window.get_zoom_pan_x().abs() < f32::EPSILON);
    assert!(window.get_zoom_pan_y().abs() < f32::EPSILON);
}

fn arrow_keys_move_the_focused_zoomed_image(window: &MainWindow) {
    window.set_zoom_level(150);
    click(window, 700.0, 350.0);
    for key in [Key::RightArrow, Key::UpArrow] {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }
    assert!((window.get_zoom_pan_x() + 24.0).abs() < f32::EPSILON);
    assert!((window.get_zoom_pan_y() - 24.0).abs() < f32::EPSILON);
    window.set_zoom_level(100);
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(700.0, 350.0),
    });
    assert!(window.get_zoom_pan_x().abs() < f32::EPSILON);
    assert!(window.get_zoom_pan_y().abs() < f32::EPSILON);
}

fn fullscreen_photo_capture_respects_patient_and_finalization_guards(window: &MainWindow) {
    window.set_selected_eye(1);
    let photos = Rc::new(Cell::new(0));
    window.global::<AppState>().on_trigger_capture({
        let photos = photos.clone();
        move || photos.set(photos.get() + 1)
    });

    click(window, 955.0, 688.0);
    assert!(window.get_iris_fullscreen());
    assert!(window.window().is_fullscreen());
    click(window, 512.0, 676.0);
    assert_eq!(photos.get(), 1);

    wait_for_controls_timeout(window);
    escape(window);
    assert!(
        !window.get_iris_fullscreen(),
        "Escape should still work after a focused capture button times out"
    );
    assert!(!window.window().is_fullscreen());
    click(window, 955.0, 688.0);
    assert!(window.get_iris_fullscreen());

    window.set_patient_action_pending(true);
    click(window, 512.0, 676.0);
    assert_eq!(photos.get(), 1, "patient lookup should block capture");
    window.set_patient_action_pending(false);
    click(window, 512.0, 676.0);
    assert_eq!(
        photos.get(),
        2,
        "capture should resume after patient lookup"
    );

    window.set_patient_first_name("Jean".into());
    window.set_patient_last_name("Dupont".into());
    click(window, 512.0, 676.0);
    assert_eq!(
        photos.get(),
        2,
        "an unresolved identity should block capture"
    );
    window.set_patient_id("123".into());
    click(window, 512.0, 676.0);
    assert_eq!(photos.get(), 3, "a selected patient should allow capture");
    window.set_selected_eye(0);
    click(window, 512.0, 676.0);
    assert_eq!(photos.get(), 3, "capture should require an eye");
    window.set_selected_eye(1);
    window.set_recording_finalizing(true);
    click(window, 512.0, 676.0);
    assert_eq!(photos.get(), 3, "video finalization should block capture");
    window.set_recording_finalizing(false);
}

fn fullscreen_recording_can_stop_during_patient_actions(window: &MainWindow) {
    let recordings = Rc::new(Cell::new(0));
    window.global::<AppState>().on_toggle_recording({
        let recordings = recordings.clone();
        move || recordings.set(recordings.get() + 1)
    });
    window.set_is_video_mode(true);
    click(window, 512.0, 676.0);
    assert_eq!(recordings.get(), 1);
    window.set_patient_action_pending(true);
    click(window, 512.0, 676.0);
    assert_eq!(
        recordings.get(),
        1,
        "patient lookup should block video start"
    );
    window.set_is_recording(true);
    click(window, 512.0, 676.0);
    assert_eq!(
        recordings.get(),
        2,
        "stopping a video should remain available"
    );
    window.set_is_recording(false);
    window.set_patient_action_pending(false);
    window.set_is_video_mode(false);
}

fn fullscreen_exit_stays_available_after_stream_loss(window: &MainWindow) {
    let image_position = LogicalPosition::new(700.0, 350.0);
    click(window, 930.0, 35.0);
    assert!(!window.get_iris_fullscreen());
    click(window, 955.0, 688.0);
    assert!(window.get_iris_fullscreen());

    window.set_is_streaming(false);
    wait_for_controls_timeout(window);
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: image_position,
    });
    assert!(
        window.global::<AppState>().get_viewport_controls_visible(),
        "hover should reveal fullscreen exit even after the stream is lost"
    );
    click(window, 930.0, 35.0);
    assert!(!window.get_iris_fullscreen());
    window.set_is_streaming(true);
    click(window, 955.0, 688.0);
    assert!(window.get_iris_fullscreen());
    escape(window);
    assert!(!window.get_iris_fullscreen());
    assert!(!window.window().is_fullscreen());
}

#[test]
fn permanent_toolbar_zoom_and_guarded_fullscreen_capture_work_from_the_live_view() {
    let window = MainWindow::new().expect("create camera interface");
    window.window().set_size(PhysicalSize::new(1024, 720));
    window.set_is_streaming(true);

    permanent_toolbar_and_fullscreen_exit_follow_overlay_visibility(&window);
    wheel_zoom_and_resize_keep_image_pan_bounded(&window);
    arrow_keys_move_the_focused_zoomed_image(&window);
    fullscreen_photo_capture_respects_patient_and_finalization_guards(&window);
    fullscreen_recording_can_stop_during_patient_actions(&window);
    fullscreen_exit_stays_available_after_stream_loss(&window);
}
