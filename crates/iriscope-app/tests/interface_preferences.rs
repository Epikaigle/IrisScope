//! Real controller persistence, pointer resizing, privacy rendering and photo controls.
#![cfg(target_os = "linux")]
use iriscope_app::{
    test_support::ControllerHarness,
    ui::{AppState, MainWindow},
};
use iriscope_core::settings::{AppSettings, InterfacePreferences};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Rgb8Pixel, SharedPixelBuffer,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

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
fn rgb(window: &MainWindow) -> Vec<u8> {
    window
        .window()
        .take_snapshot()
        .unwrap()
        .as_bytes()
        .chunks_exact(4)
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect()
}
fn wait_saved(path: &Path, expected: &InterfacePreferences) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if AppSettings::try_load_from_file(path).is_ok_and(|s| s.interface == *expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "display preferences were not persisted"
        );
        thread::sleep(Duration::from_millis(25));
        slint::platform::update_timers_and_animations();
    }
}

fn verify_presentation_camera_shortcuts_and_failed_notes(window: &MainWindow) {
    let state = window.global::<AppState>();
    state.invoke_toggle_presentation();
    state.invoke_focus_main_controls();
    for key in ["3", "4", "5"] {
        for (down, text) in [
            (true, Key::Control.into()),
            (true, key.into()),
            (false, key.into()),
            (false, Key::Control.into()),
        ] {
            window.window().dispatch_event(if down {
                WindowEvent::KeyPressed { text }
            } else {
                WindowEvent::KeyReleased { text }
            });
        }
        assert_eq!(window.get_current_tab(), 0);
        assert!(
            !state.get_library_map_open(),
            "presentation blocks shortcuts to private views"
        );
    }
    state.set_consultation_open(true);
    state.set_consultation_paused(true);
    state.set_consultation_dirty(true);
    state.set_consultation_title("Alice · D-000042".into());
    state.set_consultation_notes("Notes privées".into());
    thread::sleep(Duration::from_millis(160));
    slint::platform::update_timers_and_animations();
    let before = rgb(window);
    state.set_consultation_title("Bob · D-999999".into());
    state.set_consultation_notes("Brouillon privé conservé".into());
    assert_eq!(
        before,
        rgb(window),
        "failed session notes remain masked behind presentation"
    );
    state.invoke_dismiss_top_overlay();
    assert!(
        !state.get_presentation_mode(),
        "Escape must leave presentation despite a hidden failed-save dialog"
    );
    assert!(state.get_consultation_open());
    assert_eq!(state.get_consultation_notes(), "Brouillon privé conservé");
    state.set_consultation_open(false);
    state.set_consultation_dirty(false);
    state.set_consultation_paused(false);
}

fn verify_privacy_and_image_only(window: &MainWindow) {
    let state = window.global::<AppState>();
    state.set_image_only(true);
    window.set_viewer_open(true);
    let image = SharedPixelBuffer::<Rgb8Pixel>::new(32, 24);
    window.set_viewer_image(slint::Image::from_rgb8(image));
    state.set_viewer_eye_label("Œil gauche".into());
    state.set_viewer_caption("Alice Martin · D-000042 · alice.jpg".into());
    state.set_viewer_notes("Notes privées inchangées".into());
    state.invoke_toggle_presentation();
    assert!(state.get_presentation_mode());
    state.invoke_navigate_to(2);
    assert_eq!(
        window.get_current_tab(),
        0,
        "presentation must not reveal the library"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(600.0, 350.0),
    });
    thread::sleep(Duration::from_millis(3100));
    slint::platform::update_timers_and_animations();
    assert!(
        !state.get_photo_controls_visible(),
        "idle photo toolbars must release image space"
    );
    let before = rgb(window);
    state.set_viewer_caption("Bob Dupont · D-999999 · bob.jpg".into());
    state.set_viewer_notes("Autres notes privées inchangées".into());
    state.set_comparison_caption("D-999999 · nom-prive.jpg".into());
    assert_eq!(
        before,
        rgb(window),
        "private metadata must not change the presentation rendering"
    );
    assert_eq!(
        state.get_viewer_notes(),
        "Autres notes privées inchangées",
        "presentation preserves data"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(620.0, 350.0),
    });
    assert!(
        state.get_photo_controls_visible(),
        "mouse movement restores the controls"
    );
    state.set_photo_controls_visible(false);
    click(window, 1308.0, 28.0);
    assert!(
        state.get_photo_controls_visible(),
        "the Commandes button remains reachable"
    );
    let _ = rgb(window);
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1310.0, 82.0),
    });
    thread::sleep(Duration::from_millis(3100));
    slint::platform::update_timers_and_animations();
    assert!(
        state.get_photo_controls_visible(),
        "hovering the fullscreen button keeps photo controls visible"
    );
    state.invoke_focus_main_controls();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!state.get_image_only());
    assert!(
        window.get_viewer_open(),
        "Escape leaves Image seule before closing the photo"
    );
    state.invoke_toggle_image_only();
    state.invoke_select_photo_panel(2);
    assert!(
        !state.get_image_only(),
        "opening a photo tool restores the complete view"
    );
    state.invoke_toggle_presentation();
    verify_viewer_resize(window);
    state.invoke_dismiss_viewer();
}

fn verify_viewer_resize(window: &MainWindow) {
    let _ = rgb(window);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(946.0, 350.0),
        button: PointerEventButton::Left,
    });
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(906.0, 350.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(906.0, 350.0),
            button: PointerEventButton::Left,
        });
    assert_eq!(
        window.global::<AppState>().get_viewer_panel_width(),
        420,
        "dragging a right-hand panel left enlarges it"
    );
}

fn verify_restart_and_reset(settings: AppSettings, path: &Path) {
    let mut harness = ControllerHarness::with_settings(settings, path.to_owned()).unwrap();
    harness.start_file_workers();
    let state = harness.window().global::<AppState>();
    assert!(!state.get_sidebar_visible());
    assert_eq!(state.get_camera_panel_width(), 416);
    assert_eq!(state.get_viewer_panel_width(), 420);
    assert_eq!(state.get_consultation_panel_width(), 320);
    assert_eq!(state.get_library_view(), 1);
    assert_eq!(state.get_thumbnail_size(), 2);
    assert_eq!(state.get_preferred_photo_panel(), 2);
    assert!(state.get_advanced_settings_expanded());
    let before = harness.current_settings();
    state.set_viewer_notes("Brouillon conservé".into());
    state.invoke_reset_interface();
    wait_saved(path, &InterfacePreferences::default());
    let after = harness.current_settings();
    assert_eq!(after.capture_directory, before.capture_directory);
    assert_eq!(after.filename_template, before.filename_template);
    assert_eq!(after.theme, before.theme);
    assert_eq!(state.get_viewer_notes(), "Brouillon conservé");
    state.set_remember_layout(false);
    state.set_sidebar_visible(false);
    state.set_camera_panel_width(450);
    state.set_thumbnail_size(0);
    state.invoke_interface_edited();
    wait_saved(
        path,
        &InterfacePreferences {
            remember_layout: false,
            thumbnail_size: 0,
            ..InterfacePreferences::default()
        },
    );
}

#[test]
fn display_preferences_survive_restart_and_controls_preserve_private_data() {
    let directory = std::env::temp_dir().join(format!("iriscope-interface-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("settings.json");
    let mut harness = ControllerHarness::with_settings(
        AppSettings {
            capture_directory: directory.clone(),
            ..AppSettings::default()
        },
        path.clone(),
    )
    .unwrap();
    harness.start_file_workers();
    let window = harness.window();
    window.window().set_size(LogicalSize::new(1360.0, 860.0));
    let state = window.global::<AppState>();
    let _ = rgb(window); // Force the initial native layout before pointer interaction.
    // 12 px inset + 316 px panel + 12 px spacing + center of the grip.
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(344.0, 350.0),
        button: PointerEventButton::Left,
    });
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(444.0, 350.0),
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(444.0, 350.0),
            button: PointerEventButton::Left,
        });
    assert_eq!(
        state.get_camera_panel_width(),
        416,
        "dragging the actual separator adjusts the patient panel"
    );
    state.set_sidebar_visible(false);
    state.set_viewer_panel_width(380);
    state.set_consultation_panel_width(320);
    state.set_library_view(1);
    state.set_thumbnail_size(2);
    state.set_advanced_settings_expanded(true);
    state.invoke_select_photo_panel(2);
    verify_presentation_camera_shortcuts_and_failed_notes(window);
    verify_privacy_and_image_only(window);
    let expected = harness.current_settings().interface;
    wait_saved(&path, &expected);
    drop(harness);
    verify_restart_and_reset(AppSettings::try_load_from_file(&path).unwrap(), &path);
    fs::remove_dir_all(directory).unwrap();
}
