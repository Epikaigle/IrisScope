//! Render the real viewer while saving notes and dragging its zoom slider.
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Rgb8Pixel, SharedPixelBuffer,
    platform::{PointerEventButton, WindowEvent},
};

slint::slint! {
    import { ViewerOverlay } from "../../../ui/viewer-overlay.slint";
    export { AppState } from "../../../ui/state.slint";
    export component ViewerWindow inherits Window {
        width: 1000px; height: 700px;
        ViewerOverlay { width: parent.width; height: parent.height; }
    }
}

fn photo_bounds(window: &ViewerWindow) -> (u32, u32, u32, u32) {
    let capture = window.window().take_snapshot().unwrap();
    let mut bounds = (u32::MAX, u32::MAX, 0, 0);
    let mut count = 0;
    for y in 0..capture.height() {
        for x in 0..capture.width() {
            let pixel = capture.as_slice()[(y * capture.width() + x) as usize];
            if pixel.r > 240 && pixel.g < 5 && pixel.b > 240 {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x);
                bounds.3 = bounds.3.max(y);
                count += 1;
            }
        }
    }
    assert!(count > 1000, "the test photo must be rendered");
    bounds
}

fn verify_save_status_preserves_photo_geometry(window: &ViewerWindow) {
    let state = window.global::<AppState>();
    state.set_viewer_panel(1);
    let before = photo_bounds(window);
    state.set_viewer_review_dirty(true);
    state.set_viewer_feedback("Observation modifiée…".into());
    assert_eq!(
        before,
        photo_bounds(window),
        "pending notes cannot move the photo"
    );
    state.set_viewer_review_busy(true);
    assert_eq!(before, photo_bounds(window), "saving cannot move the photo");
    state.set_viewer_review_busy(false);
    state.set_viewer_review_dirty(false);
    state.set_viewer_saved_at("14:30".into());
    state.set_viewer_feedback("Observation enregistrée".into());
    assert_eq!(
        before,
        photo_bounds(window),
        "saved notes cannot move the photo"
    );
    state.set_viewer_feedback_error(true);
    state.set_viewer_feedback("Une erreur très longue lors de l’enregistrement de l’observation, avec des détails qui dépassent la largeur disponible, doit rester accessible sans décaler la photo.".into());
    assert_eq!(
        before,
        photo_bounds(window),
        "a long error cannot resize the photo"
    );
    state.set_viewer_feedback_error(false);
    state.set_viewer_feedback("".into());
}

fn verify_zoom_changes_before_pointer_release(window: &ViewerWindow) {
    let state = window.global::<AppState>();
    state.set_viewer_panel(0);
    state.set_viewer_fit(false);
    state.set_viewer_zoom(100);
    let _ = photo_bounds(window);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(260.0, 82.0),
        button: PointerEventButton::Left,
    });
    let first = state.get_viewer_zoom();
    let first_bounds = photo_bounds(window);
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(290.0, 82.0),
    });
    let second = state.get_viewer_zoom();
    let second_bounds = photo_bounds(window);
    assert!(second > first, "dragging must update zoom before releasing");
    assert_ne!(
        first_bounds, second_bounds,
        "the rendered image follows the drag"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(320.0, 82.0),
    });
    assert!(
        state.get_viewer_zoom() > second,
        "each new position changes zoom"
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: LogicalPosition::new(320.0, 82.0),
            button: PointerEventButton::Left,
        });
}

#[test]
fn autosave_keeps_the_photo_still_and_zoom_renders_during_drag() {
    let window = ViewerWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1000.0, 700.0));
    let state = window.global::<AppState>();
    let mut photo = SharedPixelBuffer::<Rgb8Pixel>::new(320, 240);
    photo.make_mut_slice().fill(Rgb8Pixel::new(255, 0, 255));
    state.set_viewer_image(slint::Image::from_rgb8(photo));
    state.set_viewer_open(true);
    window.show().unwrap();
    verify_save_status_preserves_photo_geometry(&window);
    verify_zoom_changes_before_pointer_release(&window);
    window.hide().unwrap();
}
