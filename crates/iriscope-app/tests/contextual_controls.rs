//! Contextual popups and wheel/slider zoom through real Slint event dispatch.
#[path = "support/software_platform.rs"]
mod software_platform;
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};

slint::slint! {
    import { DateField } from "../../../ui/date-field.slint";
    import { ReferencePane } from "../../../ui/reference-pane.slint";
    import { ZoomControl } from "../../../ui/zoom-control.slint";
    import { PopupBounds } from "../../../ui/popup-bounds.slint";
    import { AppState } from "../../../ui/state.slint";
    export { AppState } from "../../../ui/state.slint";
    export { CalendarDayData } from "../../../ui/models.slint";
    export component ContextWindow inherits Window {
        width: 600px; height: 760px;
        init => { PopupBounds.width = root.width; PopupBounds.height = root.height; }
        changed width => { PopupBounds.width = root.width; }
        changed height => { PopupBounds.height = root.height; }
        in property <image> reference-image;
        in-out property <int> zoom <=> reference.zoom;
        in-out property <length> pan-x <=> reference.pan-x;
        in-out property <length> pan-y <=> reference.pan-y;
        out property <length> upper-left: upper.popup-left;
        out property <length> upper-top: upper.popup-top;
        out property <length> lower-left: lower.popup-left;
        out property <length> lower-top: lower.popup-top;
        background: #203040;
        reference := ReferencePane {
            x: 20px; y: 60px; width: 400px; height: 200px;
            source: root.reference-image;
        }
        ZoomControl { x: 20px; y: 280px; width: 400px; zoom <=> root.zoom; maximum: 400; }
        upper := DateField { x: 20px; y: 340px; width: 230px; label: "Du"; target: 1; value <=> AppState.library-query-from; }
        lower := DateField { x: 340px; y: 560px; width: 240px; label: "Au"; target: 2; value <=> AppState.library-query-to; }
    }
}

fn click(window: &ContextWindow, x: f32, y: f32) {
    for pressed in [true, false] {
        let position = LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        window.window().dispatch_event(if pressed {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }
}

fn key(window: &ContextWindow, key: Key) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
}

fn wheel(window: &ContextWindow, delta: f32) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(220.0, 160.0),
            delta_x: 0.0,
            delta_y: delta,
        });
    slint::platform::update_timers_and_animations();
}

fn reference_width(window: &ContextWindow) -> u32 {
    let capture = window.window().take_snapshot().unwrap();
    let mut left = u32::MAX;
    let mut right = 0;
    for x in 20..420 {
        let pixel = capture.as_slice()[(160 * capture.width() + x) as usize];
        if pixel.r > 240 && pixel.g < 5 && pixel.b > 240 {
            left = left.min(x);
            right = right.max(x);
        }
    }
    assert!(left < right, "the reference must actually render");
    right - left
}

fn verify_wheel_and_slider_stay_synchronized(window: &ContextWindow) {
    let before = reference_width(window);
    wheel(window, 60.0);
    assert_eq!(window.get_zoom(), 110, "wheel works from fitted view");
    assert!(
        reference_width(window) > before,
        "wheel enlarges the rendered image"
    );
    click(window, 75.0, 296.0);
    let manually_selected = window.get_zoom();
    assert!(manually_selected > 100);
    wheel(window, 60.0);
    assert_eq!(window.get_zoom(), manually_selected + 10);
    key(window, Key::RightArrow);
    assert_eq!(
        window.get_zoom(),
        manually_selected + 11,
        "the focused slider must follow wheel changes after its binding was modified"
    );
    window.set_zoom(200);
    let _ = reference_width(window);
    click(window, 220.0, 160.0);
    key(window, Key::RightArrow);
    assert!((window.get_pan_x() + 24.0).abs() < f32::EPSILON);
    window.set_zoom(100);
    let _ = reference_width(window);
    slint::platform::update_timers_and_animations();
    assert!(window.get_pan_x().abs() < f32::EPSILON);
    assert!(window.get_pan_y().abs() < f32::EPSILON);
    wheel(window, -60.0);
    assert_eq!(window.get_zoom(), 100);
    window.set_zoom(400);
    wheel(window, 60.0);
    assert_eq!(window.get_zoom(), 400);
}

fn verify_calendar_remains_anchored_and_dismisses(window: &ContextWindow) {
    let state = window.global::<AppState>();
    let before = window.window().take_snapshot().unwrap().as_slice()[0];
    click(window, 205.0, 378.0);
    assert!(state.get_calendar_open());
    assert_eq!(state.get_calendar_target(), 1);
    let left = window.get_upper_left();
    let top = window.get_upper_top();
    assert!((8.0..=272.0).contains(&left));
    assert!(
        top > 390.0,
        "enough space opens below the date field: {top}"
    );
    let snapshot = window.window().take_snapshot().unwrap();
    assert_eq!(
        before,
        snapshot.as_slice()[0],
        "opening a calendar cannot dim or hide the page"
    );
    click(window, left + 285.0, top + 28.0);
    assert_eq!(state.get_calendar_caption(), "Mars 2024");
    assert!(
        state.get_calendar_open(),
        "month navigation must keep the popup open"
    );
    key(window, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(
        !state.get_calendar_open(),
        "Escape also clears the navigation guard"
    );
    click(window, 530.0, 598.0);
    assert!(state.get_calendar_open());
    assert_eq!(state.get_calendar_target(), 2);
    let left = window.get_lower_left();
    let top = window.get_lower_top();
    assert!((8.0..=272.0).contains(&left));
    assert!(
        top >= 8.0 && top + 288.0 <= 574.0,
        "bottom anchors open upward inside the window"
    );
    click(window, left + 118.0, top + 111.0);
    slint::platform::update_timers_and_animations();
    assert_eq!(state.get_library_query_to(), "10/02/2024");
    assert!(
        !state.get_calendar_open(),
        "choosing a day closes only the calendar"
    );
    click(window, 530.0, 598.0);
    assert!(state.get_calendar_open());
    click(window, 5.0, 5.0);
    slint::platform::update_timers_and_animations();
    assert!(
        !state.get_calendar_open(),
        "outside clicks clear the shared guard"
    );
}

#[test]
fn wheel_zoom_and_contextual_calendars_follow_their_controls() {
    software_platform::init();
    let window = ContextWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(600.0, 760.0));
    let mut photo = SharedPixelBuffer::<Rgb8Pixel>::new(320, 240);
    photo.make_mut_slice().fill(Rgb8Pixel::new(255, 0, 255));
    window.set_reference_image(slint::Image::from_rgb8(photo));
    let state = window.global::<AppState>();
    state.set_calendar_caption("Février 2024".into());
    state.set_calendar_days(ModelRc::new(VecModel::from(
        (1..=42)
            .map(|day| CalendarDayData {
                day,
                date: format!("{day:02}/02/2024").into(),
                enabled: true,
                selected: false,
            })
            .collect::<Vec<_>>(),
    )));
    let weak = window.as_weak();
    state.on_calendar_step(move |_| {
        weak.upgrade()
            .unwrap()
            .global::<AppState>()
            .set_calendar_caption("Mars 2024".into());
    });
    let weak = window.as_weak();
    state.on_select_calendar_date(move |value| {
        let window = weak.upgrade().unwrap();
        let state = window.global::<AppState>();
        if state.get_calendar_target() == 1 {
            state.set_library_query_from(value);
        } else {
            state.set_library_query_to(value);
        }
        state.set_calendar_open(false);
    });
    window.show().unwrap();
    verify_wheel_and_slider_stay_synchronized(&window);
    verify_calendar_remains_anchored_and_dismisses(&window);
    window.hide().unwrap();
}
