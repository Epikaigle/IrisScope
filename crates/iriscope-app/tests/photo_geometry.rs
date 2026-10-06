//! Pixel scale, cursor anchoring and annotation coordinates through the real pane.
use slint::{
    ComponentHandle, LogicalPosition, PhysicalSize, Rgb8Pixel, SharedPixelBuffer,
    platform::{PointerEventButton, WindowEvent},
};
use std::{cell::Cell, rc::Rc};
slint::slint! {
    import { PhotoPane } from "../../../ui/photo-pane.slint";
    import { AppState } from "../../../ui/state.slint";
    export { AppState } from "../../../ui/state.slint";
    export { AnnotationData } from "../../../ui/models.slint";
    export component PhotoWindow inherits Window {
        in-out property <image> photo;
        in-out property <bool> fit <=> pane.fit;
        in-out property <int> zoom <=> pane.zoom;
        out property <length> image-width: pane.rendered-width;
        out property <length> image-height: pane.rendered-height;
        out property <length> image-left: pane.rendered-left;
        out property <length> image-top: pane.rendered-top;
        public function zoom-at(value:int,x:length,y:length){pane.zoom-at(value,x,y);}
        pane := PhotoPane {width: 100%;height:100%;photo:root.photo;}
    }
}
fn approx(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.05,
        "actual={actual}, expected={expected}"
    );
}
fn click(window: &PhotoWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}
#[test]
fn native_pixels_cursor_zoom_and_rotated_annotations_match_the_photo() {
    let window = PhotoWindow::new().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    window.window().set_size(PhysicalSize::new(1200, 800));
    window.global::<AppState>().set_viewer_device_scale(2.0);
    window.set_photo(slint::Image::from_rgb8(
        SharedPixelBuffer::<Rgb8Pixel>::new(1280, 1024),
    ));
    window.show().unwrap();
    slint::platform::update_timers_and_animations();
    approx(window.get_image_width(), 500.0);
    approx(window.get_image_height(), 400.0);
    window.set_fit(false);
    window.set_zoom(100);
    approx(window.get_image_width(), 640.0);
    approx(window.get_image_height(), 512.0);
    let original_x = (400.0 - window.get_image_left()) / window.get_image_width();
    let original_y = (200.0 - window.get_image_top()) / window.get_image_height();
    window.invoke_zoom_at(120, 400.0, 200.0);
    approx(
        (400.0 - window.get_image_left()) / window.get_image_width(),
        original_x,
    );
    approx(
        (200.0 - window.get_image_top()) / window.get_image_height(),
        original_y,
    );
    window.invoke_zoom_at(900, 300.0, 200.0);
    assert_eq!(window.get_zoom(), 800);
    window.invoke_zoom_at(1, 300.0, 200.0);
    assert_eq!(window.get_zoom(), 10);
    let captured = Rc::new(Cell::new((0, 0.0, 0.0)));
    let callback = captured.clone();
    window
        .global::<AppState>()
        .on_add_viewer_annotation(move |kind, x, y, _, _| callback.set((kind, x, y)));
    let state = window.global::<AppState>();
    state.set_viewer_tool(3);
    state.set_viewer_rotation(90);
    window.set_photo(slint::Image::from_rgb8(
        SharedPixelBuffer::<Rgb8Pixel>::new(1024, 1280),
    ));
    window.set_fit(true);
    click(
        &window,
        window.get_image_left() + window.get_image_width() * 0.6,
        window.get_image_top() + window.get_image_height() * 0.4,
    );
    let (kind, x, y) = captured.get();
    assert_eq!(kind, 3);
    approx(x, 0.4);
    approx(y, 0.4);
    state.set_viewer_mirror(true);
    click(
        &window,
        window.get_image_left() + window.get_image_width() * 0.7,
        window.get_image_top() + window.get_image_height() * 0.3,
    );
    let (_, x, y) = captured.get();
    approx(x, 0.3);
    approx(y, 0.7);
    state.set_viewer_show_original(true);
    click(
        &window,
        window.get_image_left() + window.get_image_width() * 0.1,
        window.get_image_top() + window.get_image_height() * 0.2,
    );
    let (_, x, y) = captured.get();
    approx(x, 0.1);
    approx(y, 0.2);
    assert_precise_rotation_coordinates(&window, &captured);
    assert_arrow_region(&window);
    assert_loupe_magnifies_pointer(&window);
    window.hide().unwrap();
}

fn assert_arrow_region(window: &PhotoWindow) {
    let state = window.global::<AppState>();
    state.set_viewer_rotation(0);
    state.set_viewer_mirror(false);
    state.set_viewer_show_original(false);
    window.set_photo(slint::Image::from_rgb8(
        SharedPixelBuffer::<Rgb8Pixel>::new(1280, 1024),
    ));
    state.set_viewer_annotations(slint::ModelRc::new(slint::VecModel::from(vec![
        AnnotationData {
            kind: 2,
            x: 0.1,
            y: 0.1,
            end_x: 0.4,
            end_y: 0.4,
            ..Default::default()
        },
    ])));
    window.set_fit(true);
    let pixels = window.window().take_snapshot().unwrap();
    let mut marked = 0;
    for (index, pixel) in pixels.as_slice().iter().enumerate() {
        if pixel.r > 220 && pixel.g > 150 && pixel.b < 180 {
            let x = index % usize::try_from(pixels.width()).unwrap();
            let y = index / usize::try_from(pixels.width()).unwrap();
            assert!(
                (190..=510).contains(&x) && (70..=330).contains(&y),
                "arrow escapes its marked region at {x},{y}"
            );
            marked += 1;
        }
    }
    assert!(
        marked > 100,
        "the positioned arrow must be visible in the rendered image"
    );
}

fn assert_loupe_magnifies_pointer(window: &PhotoWindow) {
    let state = window.global::<AppState>();
    state.set_viewer_annotations(slint::ModelRc::default());
    state.set_viewer_tool(0);
    state.set_viewer_loupe(true);
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1280, 1024);
    for (index, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        *pixel = Rgb8Pixel::new(
            u8::try_from(index % 1280 / 5).unwrap(),
            u8::try_from(index / 1280 / 4).unwrap(),
            100,
        );
    }
    window.set_photo(slint::Image::from_rgb8(pixels));
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(300.0, 200.0),
    });
    let snapshot = window.window().take_snapshot().unwrap();
    if let Ok(path) = std::env::var("IRISCOPE_TEST_VIEWER_SNAPSHOT_OUTPUT") {
        image::save_buffer(
            path,
            snapshot.as_bytes(),
            snapshot.width(),
            snapshot.height(),
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    let pixel = |x: usize, y: usize| {
        snapshot.as_slice()[y * usize::try_from(snapshot.width()).unwrap() + x]
    };
    let source = pixel(600, 400);
    let magnified = pixel(820, 604);
    assert!(
        source.r.abs_diff(magnified.r) <= 1,
        "source={source:?}, loupe={magnified:?}"
    );
    assert!(
        source.g.abs_diff(magnified.g) <= 1,
        "source={source:?}, loupe={magnified:?}"
    );
    assert_eq!(magnified.b, 100);
    let ordinary_delta = pixel(700, 400).r - source.r;
    let magnified_delta = pixel(920, 604).r - magnified.r;
    assert!(
        (f32::from(ordinary_delta) / f32::from(magnified_delta) - 2.5).abs() < 0.3,
        "loupe must magnify the actual photo around the pointer by 2.5"
    );
}

fn assert_precise_rotation_coordinates(window: &PhotoWindow, captured: &Cell<(i32, f32, f32)>) {
    let state = window.global::<AppState>();
    state.set_viewer_show_original(false);
    state.set_viewer_rotation(37);
    state.set_viewer_original(slint::Image::from_rgb8(
        SharedPixelBuffer::<Rgb8Pixel>::new(320, 240),
    ));
    window.set_photo(slint::Image::from_rgb8(
        SharedPixelBuffer::<Rgb8Pixel>::new(400, 385),
    ));
    window.set_fit(true);
    for mirror in [false, true] {
        state.set_viewer_mirror(mirror);
        let (dx, dy) =
            iriscope_imaging::RotationGeometry::new(320, 240, 37).project(0.2, 0.7, mirror);
        click(
            window,
            window.get_image_left() + window.get_image_width() * dx,
            window.get_image_top() + window.get_image_height() * dy,
        );
        let (kind, x, y) = captured.get();
        assert_eq!(kind, 3);
        approx(x, 0.2);
        approx(y, 0.7);
    }
    captured.set((0, 0.0, 0.0));
    click(
        window,
        window.get_image_left() + window.get_image_width() * 0.01,
        window.get_image_top() + window.get_image_height() * 0.01,
    );
    assert_eq!(
        captured.get().0,
        0,
        "new black corners must not accept annotations outside the original image"
    );
    state.set_viewer_original(slint::Image::default());
}
