//! Exercises the shared controls independently of page geometry and camera hardware.

use slint::{
    ComponentHandle, LogicalPosition, PhysicalSize,
    platform::{Key, PointerEventButton, WindowEvent},
};

slint::slint! {
    import { AppButton, AppSlider, HelpState, HelpOverlay } from "../../../ui/controls.slint";

    export component ControlWindow inherits Window {
        in-out property <bool> controls-enabled: true;
        in-out property <int> first-clicks: 0;
        in-out property <int> second-clicks: 0;
        in-out property <int> slider-changes: 0;
        in-out property <int> slider-releases: 0;
        in-out property <float> slider-value <=> slider.value;
        in-out property <float> slider-maximum: 30;
        out property <bool> tooltip-open: HelpState.text != "";
        out property <bool> first-has-focus: first.has-focus;

        first := AppButton {
            x: 20px; y: 20px; width: 120px; height: 36px;
            text: "Capture";
            tooltip: "Prendre une photo du dossier actif";
            enabled: root.controls-enabled;
            clicked => { root.first-clicks += 1; }
        }
        AppButton {
            x: 160px; y: 20px; width: 120px; height: 36px;
            text: "Autre action";
            clicked => { root.second-clicks += 1; }
        }
        slider := AppSlider {
            x: 20px; y: 100px; width: 300px; height: 32px;
            label: "Contraste";
            minimum: 10; maximum: root.slider-maximum; step: 5; value: 20;
            enabled: root.controls-enabled;
            changed => { root.slider-changes += 1; }
            released(value) => { root.slider-releases += 1; }
        }
        HelpOverlay { available-width: root.width; available-height: root.height; }
    }
}

fn click(window: &ControlWindow, x: f32, y: f32) {
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

fn press(window: &ControlWindow, text: slint::SharedString) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text });
}

fn release(window: &ControlWindow, text: slint::SharedString) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn buttons_cancel_activation_on_repetition_focus_loss_and_disable(window: &ControlWindow) {
    click(window, 70.0, 38.0);
    assert!(
        window.get_first_has_focus(),
        "pointer interaction sets keyboard focus"
    );
    assert_eq!(window.get_first_clicks(), 1);
    press(window, " ".into());
    for _ in 0..3 {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressRepeated { text: " ".into() });
    }
    assert_eq!(
        window.get_first_clicks(),
        1,
        "holding Space cannot repeat captures"
    );
    release(window, " ".into());
    assert_eq!(window.get_first_clicks(), 2);
    release(window, " ".into());
    assert_eq!(
        window.get_first_clicks(),
        2,
        "unpaired release cannot capture"
    );

    press(window, " ".into());
    release(window, "\n".into());
    assert_eq!(
        window.get_first_clicks(),
        2,
        "a different key cannot release Space"
    );
    release(window, " ".into());
    assert_eq!(window.get_first_clicks(), 3);

    press(window, " ".into());
    click(window, 210.0, 38.0);
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressRepeated { text: " ".into() });
    release(window, " ".into());
    assert_eq!(
        window.get_first_clicks(),
        3,
        "losing focus cancels activation"
    );
    assert_eq!(
        window.get_second_clicks(),
        1,
        "a repeated key cannot arm a newly focused button"
    );

    click(window, 70.0, 38.0);
    press(window, "\n".into());
    window.set_controls_enabled(false);
    slint::platform::update_timers_and_animations();
    release(window, "\n".into());
    window.set_controls_enabled(true);
    slint::platform::update_timers_and_animations();
    release(window, "\n".into());
    assert_eq!(
        window.get_first_clicks(),
        4,
        "disabling a held button cancels activation"
    );
}

#[allow(clippy::float_cmp)] // These values are exact integer camera steps.
fn sliders_respect_native_steps_bounds_and_disable(window: &ControlWindow) {
    click(window, 170.0, 116.0);
    window.set_slider_value(20.0);
    window.set_slider_releases(0);
    press(window, Key::RightArrow.into());
    assert_eq!(
        window.get_slider_value(),
        25.0,
        "arrow respects the camera's step"
    );
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressRepeated {
            text: Key::RightArrow.into(),
        });
    assert_eq!(window.get_slider_value(), 30.0);
    release(window, Key::RightArrow.into());
    assert_eq!(window.get_slider_releases(), 1);
    press(window, Key::RightArrow.into());
    assert_eq!(
        window.get_slider_value(),
        30.0,
        "value stays inside the range"
    );
    release(window, Key::RightArrow.into());
    press(window, Key::Home.into());
    release(window, Key::Home.into());
    assert_eq!(window.get_slider_value(), 10.0);
    press(window, Key::End.into());
    release(window, Key::End.into());
    assert_eq!(window.get_slider_value(), 30.0);

    click(window, 125.0, 116.0);
    assert_eq!(
        window.get_slider_value(),
        15.0,
        "pointer snaps to a supported value"
    );
    window.set_controls_enabled(false);
    press(window, Key::RightArrow.into());
    release(window, Key::RightArrow.into());
    click(window, 300.0, 116.0);
    assert_eq!(
        window.get_slider_value(),
        15.0,
        "disabled sliders ignore interaction"
    );

    window.set_controls_enabled(true);
    window.set_slider_maximum(28.0);
    click(window, 300.0, 116.0);
    assert_eq!(window.get_slider_value(), 25.0);
    press(window, Key::Home.into());
    release(window, Key::Home.into());
    assert_eq!(window.get_slider_value(), 10.0);
    press(window, Key::End.into());
    release(window, Key::End.into());
    assert_eq!(
        window.get_slider_value(),
        25.0,
        "End and pointer stop at the final supported step below a nonaligned maximum"
    );
}

fn tooltip_preserves_clicks_and_keyboard_focus(window: &ControlWindow) {
    window.show().expect("show controls");
    click(window, 70.0, 38.0);
    let before = window.get_first_clicks();
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(70.0, 38.0),
    });
    slint::platform::update_timers_and_animations();
    std::thread::sleep(std::time::Duration::from_millis(700));
    slint::platform::update_timers_and_animations();
    assert!(window.get_tooltip_open(), "hover must reveal the help text");
    assert!(
        window.get_first_has_focus(),
        "help must not steal keyboard focus"
    );
    click(window, 70.0, 38.0);
    slint::platform::update_timers_and_animations();
    assert_eq!(
        window.get_first_clicks(),
        before + 1,
        "help must not consume the first click"
    );
    assert!(!window.get_tooltip_open());
    window.hide().expect("hide controls");
}

#[test]
fn shared_controls_handle_keyboard_repetition_focus_cancellation_and_camera_steps() {
    let window = ControlWindow::new().expect("create shared controls");
    window.window().set_size(PhysicalSize::new(360, 180));
    buttons_cancel_activation_on_repetition_focus_loss_and_disable(&window);
    sliders_respect_native_steps_bounds_and_disable(&window);
    tooltip_preserves_clicks_and_keyboard_focus(&window);
}
