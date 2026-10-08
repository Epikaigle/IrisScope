//! Exercises the shared controls independently of page geometry and camera hardware.
#[path = "support/software_platform.rs"]
mod software_platform;

use slint::{
    ComponentHandle, LogicalPosition, PhysicalSize,
    platform::{Key, PointerEventButton, WindowEvent},
};

slint::slint! {
    import { AppButton, AppSlider, HelpState, HelpOverlay } from "../../../ui/controls.slint";
    import { PopupBounds } from "../../../ui/popup-bounds.slint";
    import { RotationControl } from "../../../ui/rotation-control.slint";
    import { MirrorControl } from "../../../ui/mirror-control.slint";
    import { AppComboBox } from "../../../ui/combo-box.slint";
    import { ThumbnailSizeControl } from "../../../ui/thumbnail-size.slint";
    import { ScrollView } from "std-widgets.slint";
    export { AppState } from "../../../ui/state.slint";

    export component ControlWindow inherits Window {
        init => { PopupBounds.width = root.width; PopupBounds.height = root.height; }
        changed width => { PopupBounds.width = root.width; }
        changed height => { PopupBounds.height = root.height; }
        out property <length> rotation-left: rotation.popup-left;
        out property <length> rotation-top: rotation.popup-top;
        out property <length> bottom-left: bottom-rotation.popup-left;
        out property <length> bottom-top: bottom-rotation.popup-top;
        out property <int> bottom-angle: bottom-rotation.angle;
        in-out property <bool> controls-enabled: true;
        in-out property <int> first-clicks: 0;
        in-out property <int> second-clicks: 0;
        in-out property <int> slider-changes: 0;
        in-out property <int> slider-releases: 0;
        in-out property <float> slider-value <=> slider.value;
        in-out property <float> slider-maximum: 30;
        out property <bool> tooltip-open: HelpState.text != "";
        out property <bool> first-has-focus: first.has-focus;
        out property <bool> first-focus-visible: first.keyboard-focus-visible;
        out property <length> mirror-left: mirror.popup-left;
        out property <length> mirror-top: mirror.popup-top;
        out property <bool> mirror-horizontal: mirror.horizontal;
        out property <bool> mirror-vertical: mirror.vertical;
        out property <bool> mirror-has-focus: mirror.has-focus;
        out property <bool> mirror-focus-visible: mirror.keyboard-focus-visible;
        in-out property <int> rotation-angle <=> rotation.angle;
        in-out property <int> rotation-changes: 0;
        in-out property <int> menu-index <=> menu.current-index;
        in-out property <length> menu-scroll-y <=> menus.viewport-y;
        callback focus-menu();
        focus-menu => { menu.focus(); }

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
        menus := ScrollView {
            x: 20px; y: 170px; width: 300px; height: 100px;
            viewport-height: 480px;
            horizontal-scrollbar-policy: always-off;
            Rectangle {
                height: 480px;
                menu := AppComboBox {
                    x: 0px; y: 0px; width: 250px; height: 36px;
                    model: ["Maximale", "Équilibrée", "Fluide"];
                }
            }
        }
        ThumbnailSizeControl { x: 20px; y: 290px; width: 300px; height: 36px; }
        rotation := RotationControl { x: 20px; y: 350px; width: 150px; height: 32px; adjusted => {root.rotation-changes += 1;} }
        bottom-rotation := RotationControl { x: 280px; y: 560px; width: 60px; height: 32px; short-label: true; }
        mirror := MirrorControl { x: 240px; y: 510px; width: 90px; height: 32px; above-clearance: 52px; }
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
    assert!(
        !window.get_first_focus_visible(),
        "mouse clicks do not add a keyboard outline"
    );
    press(window, " ".into());
    assert!(
        window.get_first_focus_visible(),
        "keyboard activation retains a visible focus indicator"
    );
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

fn menus_scroll_the_page_without_changing_the_selected_value(window: &ControlWindow) {
    window.show().unwrap();
    let _ = window.window().take_snapshot().unwrap();
    window.set_menu_index(1);
    window.invoke_focus_menu();
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(100.0, 188.0),
            delta_x: 0.0,
            delta_y: -60.0,
        });
    assert_eq!(
        window.get_menu_index(),
        1,
        "a focused closed menu must never change from the wheel"
    );
    assert!(
        window.get_menu_scroll_y() < 0.0,
        "the surrounding page must still scroll"
    );
    window.set_menu_scroll_y(0.0);
    click(window, 100.0, 188.0);
    click(window, 100.0, 304.0);
    assert_eq!(
        window.get_menu_index(),
        2,
        "explicit popup selection still works"
    );
    let state = window.global::<AppState>();
    state.set_thumbnail_width(187);
    click(window, 200.0, 306.0);
    state.set_thumbnail_width(187);
    slint::platform::update_timers_and_animations();
    press(window, Key::RightArrow.into());
    release(window, Key::RightArrow.into());
    assert_eq!(
        state.get_thumbnail_width(),
        188,
        "thumbnail keyboard adjustment is exactly one pixel"
    );
    press(window, Key::Home.into());
    release(window, Key::Home.into());
    assert_eq!(state.get_thumbnail_width(), 80);
    press(window, Key::End.into());
    release(window, Key::End.into());
    assert_eq!(state.get_thumbnail_width(), 480);
    state.set_thumbnail_width(0);
    state.set_thumbnail_size(0);
    slint::platform::update_timers_and_animations();
    press(window, Key::RightArrow.into());
    release(window, Key::RightArrow.into());
    assert_eq!(
        state.get_thumbnail_width(),
        145,
        "preset selection also updates the slider"
    );
}

#[test]
fn shared_controls_handle_keyboard_repetition_focus_cancellation_and_camera_steps() {
    software_platform::init();
    let window = ControlWindow::new().expect("create shared controls");
    window.window().set_size(PhysicalSize::new(360, 600));
    menus_scroll_the_page_without_changing_the_selected_value(&window);
    buttons_cancel_activation_on_repetition_focus_loss_and_disable(&window);
    sliders_respect_native_steps_bounds_and_disable(&window);
    tooltip_preserves_clicks_and_keyboard_focus(&window);
    precise_rotation_updates_during_drag_and_uses_one_degree_steps(&window);
    mirror_menu_stays_above_the_toolbar_and_both_choices_work(&window);
}

fn mirror_menu_stays_above_the_toolbar_and_both_choices_work(window: &ControlWindow) {
    window.show().unwrap();
    click(window, 285.0, 526.0);
    let _ = window.window().take_snapshot().unwrap();
    let left = window.get_mirror_left();
    let top = window.get_mirror_top();
    assert!(left >= 8.0 && left + 180.0 <= 352.0);
    assert!(
        top >= 8.0 && top + 84.0 < 510.0 - 48.0,
        "the mirror menu must clear both toolbar rows"
    );
    click(window, left + 60.0, top + 24.0);
    assert!(window.get_mirror_horizontal());
    click(window, left + 60.0, top + 60.0);
    assert!(window.get_mirror_vertical());
    click(window, 350.0, 100.0);
    assert!(
        window.get_mirror_has_focus(),
        "closing a popup restores its anchor focus"
    );
    assert!(
        !window.get_mirror_focus_visible(),
        "a mouse-dismissed popup adds no keyboard outline"
    );
    press(window, Key::Tab.into());
    release(window, Key::Tab.into());
    assert!(
        window.get_first_focus_visible(),
        "Tab restores the keyboard focus indicator"
    );
    press(window, Key::Escape.into());
    release(window, Key::Escape.into());
    window.hide().unwrap();
}

fn precise_rotation_updates_during_drag_and_uses_one_degree_steps(window: &ControlWindow) {
    window.set_controls_enabled(true);
    click(window, 70.0, 366.0);
    let _ = window.window().take_snapshot().unwrap();
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(60.0, 236.0),
        button: PointerEventButton::Left,
    });
    let first = window.get_rotation_angle();
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(120.0, 236.0),
    });
    assert!(
        window.get_rotation_angle() > first,
        "rotation must update before releasing the slider"
    );
    assert!(window.get_rotation_changes() > 1);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(120.0, 236.0),
            button: PointerEventButton::Left,
        });
    window.set_rotation_angle(37);
    slint::platform::update_timers_and_animations();
    press(window, Key::RightArrow.into());
    release(window, Key::RightArrow.into());
    assert_eq!(
        window.get_rotation_angle(),
        38,
        "model changes must also update the rotation slider"
    );
    press(window, Key::Home.into());
    release(window, Key::Home.into());
    assert_eq!(window.get_rotation_angle(), 0);
    press(window, Key::End.into());
    release(window, Key::End.into());
    assert_eq!(window.get_rotation_angle(), 359);
    click(window, 88.0, 196.0);
    press(window, Key::Control.into());
    press(window, "a".into());
    release(window, "a".into());
    release(window, Key::Control.into());
    press(window, "37".into());
    release(window, "37".into());
    press(window, Key::Return.into());
    release(window, Key::Return.into());
    assert_eq!(
        window.get_rotation_angle(),
        37,
        "an exact typed angle must also update the slider"
    );
    click(window, 335.0, 450.0);
    assert!((8.0..=72.0).contains(&window.get_rotation_left()));
    assert!((window.get_rotation_top() - 168.0).abs() < f32::EPSILON);
    click(window, 310.0, 576.0);
    let _ = window.window().take_snapshot().unwrap();
    assert!((8.0..=72.0).contains(&window.get_bottom_left()));
    assert!(
        window.get_bottom_top() + 174.0 < 560.0,
        "bottom toolbar remains uncovered"
    );
    click(window, 150.0, 446.0);
    assert!(
        window.get_bottom_angle() > 0,
        "the upward popup's slider must remain reachable"
    );
    click(window, 335.0, 350.0);
}
