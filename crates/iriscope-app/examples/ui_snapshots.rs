//! Deterministic UI fixtures for visual regression checks; no camera or user data.
#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss // Snapshot coordinates are positive and bounded by the fixture window.
)]
use iriscope_app::ui::{
    AnnotationData, AppState, CalendarDayData, CameraControlUiData, ComparisonCandidateData,
    ConsultationDayData, DiagnosticData, LibraryItemData, MainWindow, PatientCandidateData,
    SessionPhotoData,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{
    ComponentHandle, Image, LogicalPosition, LogicalSize, ModelRc, Rgb8Pixel, SharedPixelBuffer,
    VecModel,
};
use std::{cell::Cell, path::Path, rc::Rc};

#[path = "../tests/support/software_platform.rs"]
mod software_platform;

fn fixture_image() -> Image {
    // Optional showcase photos never change the default regression fixtures.
    if let Some(path) = std::env::var_os("IRISCOPE_SNAPSHOT_IMAGE") {
        return Image::load_from_path(Path::new(&path)).expect("showcase image");
    }
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(640, 480);
    for (index, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        let x = index % 640;
        let y = index / 640;
        *pixel = Rgb8Pixel::new((x * 255 / 640) as u8, (y * 255 / 480) as u8, 100);
    }
    Image::from_rgb8(pixels)
}

fn main() {
    if std::env::var_os("IRISCOPE_SNAPSHOT_OFFSCREEN").is_some() {
        software_platform::init();
    }
    let output = std::env::args().nth(1).expect("output directory");
    let requested_scale = std::env::var("SLINT_SCALE_FACTOR")
        .unwrap_or_else(|_| "1".to_owned())
        .parse::<f32>()
        .expect("scale factor");
    std::fs::create_dir_all(&output).unwrap();
    let scenarios = [
        "camera-idle",
        "camera-active",
        "camera-after-capture",
        "camera-narrow-after-capture",
        "camera-mirror-popup",
        "camera-click-selection",
        "camera-rotation",
        "camera-angle-popup",
        "camera-presentation-idle",
        "viewer-rotation",
        "viewer-angle-popup",
        "library-menu",
        "library-sparse",
        "viewer-tools-menu",
        "viewer-view-menu",
        "dossier-empty",
        "camera-collapsed",
        "camera-recording",
        "camera-controls",
        "camera-fullscreen",
        "camera-fullscreen-photo",
        "camera-fullscreen-blocked",
        "camera-fullscreen-hidden",
        "patient-search",
        "library-empty",
        "library-grid",
        "library-list",
        "library-assignment",
        "library-error",
        "settings",
        "settings-bottom",
        "settings-backup",
        "settings-reminder",
        "references-empty",
        "references-loaded",
        "viewer-photo",
        "viewer-notes",
        "viewer-photo-export",
        "dossier-session",
        "calendar-filter",
        "viewer-references",
        "viewer-display",
        "viewer-chooser",
        "viewer-zoom",
        "viewer-loupe",
        "viewer-comparison",
        "patient-edit",
        "library-query",
        "viewer-video",
        "viewer-export",
        "capture-notice",
        "camera-presentation",
        "presentation-notes-error",
        "camera-resized",
        "viewer-presentation",
        "viewer-image-only",
        "viewer-loading-notes",
        "viewer-resized",
        "library-small",
        "library-custom",
        "library-minimum",
        "library-large",
        "library-list-large",
        "settings-advanced",
    ];
    let requested_scenes = std::env::var("IRISCOPE_SNAPSHOT_SCENES").ok();
    let scenarios: Vec<_> = scenarios
        .into_iter()
        .filter(|scene| {
            requested_scenes
                .as_ref()
                .is_none_or(|selection| selection.split(',').any(|name| name == *scene))
        })
        .collect();
    assert!(!scenarios.is_empty(), "no matching snapshot scenarios");
    let requested_sizes = std::env::var("IRISCOPE_SNAPSHOT_SIZES").ok();
    let mut captures = 0;
    for theme in [1, 2] {
        for (width, height) in [(800, 600), (1024, 720), (1360, 860), (1920, 1080)] {
            if requested_sizes.as_ref().is_some_and(|selection| {
                !selection
                    .split(',')
                    .any(|size| size == format!("{width}x{height}"))
            }) {
                continue;
            }
            for scenario in scenarios.iter().copied() {
                let window = MainWindow::new().unwrap();
                // Snapshot rendering does not pump the native desktop event loop.
                // Deliver the same event a backend sends on creation or monitor change.
                window
                    .window()
                    .dispatch_event(WindowEvent::ScaleFactorChanged {
                        scale_factor: requested_scale,
                    });
                window
                    .window()
                    .set_size(LogicalSize::new(width as f32, height as f32));
                window.set_settings_theme(theme);
                window
                    .global::<AppState>()
                    .set_app_version(env!("CARGO_PKG_VERSION").into());
                window.set_status_text("DE400 non détecté".into());
                window.set_settings_capture_directory("/workspace/Images/IrisScope".into());
                window
                    .set_settings_filename_template("{prenom}_{nom}_{oeil}_{date}_{heure}".into());
                let state = window.global::<AppState>();
                let selected_patient = Rc::new(Cell::new(false));
                let image = fixture_image();
                if scenario != "camera-idle"
                    && !scenario.starts_with("library")
                    && !scenario.starts_with("references")
                {
                    window.set_is_streaming(true);
                    window.set_camera_connected(true);
                    window.set_status_text("DE400 connecté · 1280 × 1024 MJPEG".into());
                    window.set_live_frame(image.clone());
                    window.set_selected_eye(1);
                }
                match scenario {
                    "camera-collapsed" => window.set_sidebar_visible(false),
                    "camera-recording" | "camera-fullscreen" => {
                        window.set_is_recording(true);
                        window.set_recording_duration("59:58".into());
                        window.set_session_photo_count(999);
                        window.set_is_frozen(true);
                        if scenario == "camera-fullscreen" {
                            window.set_iris_fullscreen(true);
                        }
                    }
                    "camera-fullscreen-photo"
                    | "camera-fullscreen-blocked"
                    | "camera-fullscreen-hidden" => {
                        window.set_iris_fullscreen(true);
                        if scenario == "camera-fullscreen-photo" {
                            window.set_session_photo_count(1);
                            window.set_show_last_capture(true);
                            window.set_has_last_capture(true);
                            window.set_capture_notice_tone(1);
                            window.set_last_capture_message(
                                "Photo enregistrée : Gauche_2026-10-05_14-42-16.jpg".into(),
                            );
                        }
                        if scenario == "camera-fullscreen-blocked" {
                            window.set_selected_eye(0);
                        }
                        if scenario == "camera-fullscreen-hidden" {
                            state.set_viewport_controls_visible(false);
                        }
                    }
                    "camera-controls" => {
                        window.set_image_controls_open(true);
                        window.set_camera_controls(ModelRc::new(VecModel::from(vec![
                            CameraControlUiData {
                                key: "brightness".into(),
                                name: "Luminosité".into(),
                                kind: 0,
                                minimum: 0.,
                                maximum: 100.,
                                step: 1.,
                                value: 50.,
                                value_label: "50".into(),
                                ..Default::default()
                            },
                            CameraControlUiData {
                                key: "wb".into(),
                                name: "Balance des blancs automatique".into(),
                                kind: 1,
                                boolean_value: true,
                                ..Default::default()
                            },
                            CameraControlUiData {
                                key: "mode".into(),
                                name: "Fréquence du secteur électrique".into(),
                                kind: 2,
                                menu_label: "Mode automatique du périphérique (50/60 Hz)".into(),
                                menu_options: ModelRc::new(VecModel::from(vec![
                                    "Automatique".into(),
                                    "50 Hz".into(),
                                    "60 Hz".into(),
                                ])),
                                menu_index: 0,
                                ..Default::default()
                            },
                            CameraControlUiData {
                                key: "exposure".into(),
                                name: "Durée d’exposition en microsecondes".into(),
                                kind: 3,
                                value_label: "100000".into(),
                                ..Default::default()
                            },
                            CameraControlUiData {
                                key: "ro".into(),
                                name: "Réglage du périphérique en lecture seule".into(),
                                kind: 0,
                                read_only: true,
                                ..Default::default()
                            },
                        ])));
                    }
                    "patient-search" => {
                        let selected = selected_patient.clone();
                        state.on_select_patient(move |id| selected.set(id == "42"));
                        window.set_patient_first_name("Marie-Christine".into());
                        window.set_patient_last_name("de la Roche-Saint-André".into());
                        window.set_patient_candidates(ModelRc::new(VecModel::from(vec![
                            PatientCandidateData {
                                id: "42".into(),
                                dossier_number: "D-000042".into(),
                                first_name: "Marie-Christine".into(),
                                last_name: "de la Roche-Saint-André".into(),
                                last_capture: "05/10/2026 à 14:42".into(),
                            },
                        ])));
                    }
                    "settings" | "settings-bottom" | "settings-backup" | "settings-reminder"
                    | "settings-advanced" => {
                        window.set_current_tab(4);
                        window.set_diagnostics(DiagnosticData {
                            device_name:
                                "Infoxelle Co., Ltd. Digital Microscope / Firefly DE400 OEM".into(),
                            device_id: "/dev/v4l/by-id/usb-Infoxelle_DE400-video-index0".into(),
                            backend_name: "V4L2".into(),
                            usb_info: "USB 2.0 · VID 21cd / PID 603b".into(),
                            active_format: "MJPEG".into(),
                            active_resolution: "1280 × 1024".into(),
                            active_fps: "8 FPS".into(),
                            measured_fps: "7,9 FPS".into(),
                            decode_time_ms: "3,2 ms".into(),
                            frame_count: 12345,
                            dropped_frames: 12,
                        });
                        window.set_settings_feedback(
                            "Paramètres enregistrés dans le dossier local.".into(),
                        );
                        window.set_settings_iridology_map_path(
                            "/workspace/Images/References/carte-iris.jpg".into(),
                        );
                        window.set_settings_iridology_symbols_path(
                            "/workspace/Images/References/signes-symboles.jpg".into(),
                        );
                        if scenario == "settings-backup" {
                            state.set_storage_busy(true);
                            state.set_storage_progress_known(true);
                            state.set_storage_progress(42);
                            state.set_storage_feedback(
                                "Copie vérifiée : 12 / 48 fichiers · 420 / 1000 Mio".into(),
                            );
                        }
                        if scenario == "settings-reminder" {
                            state.set_backup_reminder_enabled(true);
                            state.set_backup_reminder(
                                "Votre dernière sauvegarde date d’au moins 7 jours.".into(),
                            );
                            state.set_backup_last_status("Dernière sauvegarde réussie : 27/09/2026 à 14:42 · 1248 fichiers\nD:/Mes sauvegardes IrisScope/Dossier personnel et captures/IrisScope-sauvegarde-2026-09-27".into());
                        }
                    }
                    "references-empty" | "references-loaded" => {
                        window.set_library_map_open(true);
                        if scenario == "references-loaded" {
                            state.set_has_iridology_map(true);
                            state.set_has_iridology_symbols(true);
                            state.set_iridology_map_image(image.clone());
                            state.set_iridology_symbols_image(image.clone());
                            window.set_settings_iridology_symbols_path("/tmp/symboles.png".into());
                        }
                    }
                    "viewer-tools-menu"
                    | "viewer-view-menu"
                    | "viewer-photo"
                    | "viewer-notes"
                    | "viewer-presentation"
                    | "viewer-image-only"
                    | "viewer-loading-notes"
                    | "viewer-resized"
                    | "viewer-photo-export"
                    | "viewer-references"
                    | "viewer-rotation"
                    | "viewer-angle-popup"
                    | "viewer-display"
                    | "viewer-chooser"
                    | "viewer-zoom"
                    | "viewer-loupe"
                    | "viewer-video"
                    | "viewer-comparison"
                    | "viewer-export" => {
                        window.set_viewer_open(true);
                        window.set_viewer_image(image.clone());
                        let state = window.global::<AppState>();
                        state.set_viewer_caption("Œil gauche · 2026-10-05 14:42 · D-000042".into());
                        state.set_viewer_original(image.clone());
                        state.set_viewer_device_scale(requested_scale);
                        state.set_viewer_dossier("D-000042".into());
                        if scenario == "viewer-photo-export" {
                            state.set_viewer_panel(5);
                        }
                        if scenario == "viewer-notes" {
                            state.set_viewer_saved_at("14:42".into());
                            state.set_viewer_panel(1);
                            state.set_viewer_notes(
                                "Observation de la zone repérée.
Éclairage et cadrage à conserver à la prochaine prise de vue."
                                    .into(),
                            );
                            state.set_viewer_annotations(ModelRc::new(VecModel::from(vec![
                                AnnotationData {
                                    kind: 1,
                                    x: 0.5,
                                    y: 0.5,
                                    end_x: 0.7,
                                    end_y: 0.5,
                                    ..Default::default()
                                },
                                AnnotationData {
                                    kind: 2,
                                    x: 0.1,
                                    y: 0.1,
                                    end_x: 0.4,
                                    end_y: 0.4,
                                    ..Default::default()
                                },
                                AnnotationData {
                                    kind: 4,
                                    x: 0.4,
                                    y: 0.7,
                                    end_x: 0.4,
                                    end_y: 0.7,
                                    text: "Zone observée".into(),
                                },
                            ])));
                            state.set_viewer_can_undo(true);
                        }
                        if scenario == "viewer-references" {
                            state.set_viewer_panel(2);
                            state.set_has_iridology_map(true);
                            state.set_iridology_map_image(image.clone());
                            state.set_viewer_guides(true);
                        }
                        if matches!(scenario, "viewer-rotation" | "viewer-angle-popup") {
                            state.set_viewer_panel(3);
                            state.set_viewer_rotation(37);
                            let pixels = image.to_rgb8().unwrap();
                            let (w, h, rgb) = iriscope_imaging::apply_transforms(
                                pixels.as_bytes(),
                                pixels.width(),
                                pixels.height(),
                                37,
                                false,
                                false,
                            );
                            window.set_viewer_image(Image::from_rgb8(
                                SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, w, h),
                            ));
                        }
                        if scenario == "viewer-display" {
                            state.set_viewer_panel(3);
                            state.set_viewer_brightness(20);
                            state.set_viewer_mirror(true);
                        }
                        if scenario == "viewer-chooser" {
                            state.set_viewer_panel(4);
                            state.set_comparison_candidates(ModelRc::new(VecModel::from(vec![
                                ComparisonCandidateData {
                                    caption: "20/09/2026 10:30 · Gauche · Retenue".into(),
                                    thumbnail: image.clone(),
                                    ..Default::default()
                                },
                                ComparisonCandidateData {
                                    caption: "12/08/2026 14:45 · Gauche".into(),
                                    thumbnail: image.clone(),
                                    ..Default::default()
                                },
                            ])));
                        }
                        if scenario == "viewer-zoom" {
                            state.set_viewer_fit(false);
                            state.set_viewer_zoom(250);
                        }
                        if scenario == "viewer-loupe" {
                            state.set_viewer_loupe(true);
                        }

                        if scenario == "viewer-comparison" {
                            state.set_comparison_image(image.clone());
                            state.set_comparison_caption(
                                "Œil gauche · 2026-09-20 10:30 · D-000042".into(),
                            );
                            state.set_comparison_available(true);
                            state.set_comparison_enabled(true);
                        }
                        if scenario == "viewer-video" || scenario == "viewer-export" {
                            window.set_viewer_is_video(true);
                            window.set_viewer_video_position("35:42".into());
                            window.set_viewer_video_duration("59:58".into());
                            window.set_viewer_video_progress(600.);
                        }
                        if scenario == "viewer-export" {
                            state.set_export_busy(true);
                            state.set_export_progress_known(true);
                            state.set_export_progress(42);
                            state.set_export_feedback(
                                "Export MP4 : 42 % · vidéo originale conservée".into(),
                            );
                        }
                    }
                    "capture-notice" => {
                        window.set_show_last_capture(true);
                        window.set_capture_notice_tone(1);
                        window.set_has_last_capture(true);
                        window.set_last_capture_message("Photo enregistrée : Marie_Christine_de_la_Roche_Saint_Andre_Gauche_2026-10-05_14-42-16.jpg".into());
                    }
                    _ => {}
                }
                if scenario == "patient-edit" {
                    let state = window.global::<AppState>();
                    state.set_patient_id("42".into());
                    state.set_patient_dossier_number("D-000042".into());
                    state.set_patient_edit_first("Marie-Christine".into());
                    state.set_patient_edit_last("de la Roche Saint-André".into());
                    state.set_patient_edit_open(true);
                }
                window
                    .global::<AppState>()
                    .set_storage_status("Espace disponible : 25,4 Gio".into());
                if scenario == "library-query" {
                    let state = window.global::<AppState>();
                    state.set_library_query_dossier("D-000042".into());
                    state.set_library_query_from("2026-09-01".into());
                    state.set_library_query_to("2026-10-05".into());
                    state.set_library_query_eye(1);
                    state.set_library_query_active(true);
                }
                if scenario.starts_with("library") {
                    window.set_current_tab(2);
                    if scenario == "library-error" {
                        window.set_library_error("Impossible de lire le dossier /workspace/Images/IrisScope : accès refusé. Vérifiez les autorisations du dossier de captures.".into());
                    } else if scenario != "library-empty" {
                        let items = (0..if scenario == "library-sparse" { 2 } else { 8 })
                            .map(|n| LibraryItemData {
                                id: n.to_string().into(),
                                file_path: format!("/tmp/capture-{n}.jpg").into(),
                                file_version: "1".into(),
                                title: if n % 2 == 0 {
                                    "Photo de l’iris gauche".into()
                                } else {
                                    "Vidéo de l’iris droit".into()
                                },
                                date_time: "05/10/2026 à 14:42".into(),
                                dossier_number: if n % 3 == 0 {
                                    "D-000042".into()
                                } else {
                                    "".into()
                                },
                                eye_label: if n % 2 == 0 {
                                    "Gauche".into()
                                } else {
                                    "Droit".into()
                                },
                                thumbnail: image.clone(),
                                has_thumbnail: true,
                                is_video: n % 2 == 1,
                                can_open_in_app: true,
                                is_current_patient: n % 3 == 0,
                            })
                            .collect::<Vec<_>>();
                        window.set_library_items(ModelRc::new(VecModel::from(items)));
                        window.set_library_total_count(108);
                        window.set_library_page_summary("Captures 1–100 sur 108".into());
                        if scenario == "library-list" {
                            window.set_library_view(1);
                        }
                        if scenario == "library-assignment" {
                            window.set_patient_id("42".into());
                            window.set_patient_dossier_number("D-000042".into());
                            window.set_selected_library_path("/tmp/capture-1.jpg".into());
                            window.set_selected_library_file_version("1".into());
                            window.set_pending_recordings_count(2);
                        }
                    }
                }
                if scenario == "dossier-session" {
                    state.set_consultation_open(true);
                    state.set_consultation_title("Émilie Martin · D-000042".into());
                    state.set_consultation_patient_id("42".into());
                    state.set_dossier_search("Martin".into());
                    state.set_consultation_day("2026-10-06".into());
                    state.set_consultation_notes(
                        "Conditions d’éclairage conservées.
Observations générales de la séance."
                            .into(),
                    );
                    state.set_consultation_saved_at("14:42".into());
                    state.set_dossier_candidates(ModelRc::new(VecModel::from(vec![
                        PatientCandidateData {
                            id: "42".into(),
                            dossier_number: "D-000042".into(),
                            first_name: "Émilie".into(),
                            last_name: "Martin".into(),
                            ..Default::default()
                        },
                    ])));
                    state.set_consultation_days(ModelRc::new(VecModel::from(vec![
                        ConsultationDayData {
                            date: "2026-10-06".into(),
                            label: "06/10/2026".into(),
                            left: 2,
                            right: 1,
                            videos: 0,
                        },
                        ConsultationDayData {
                            date: "2026-08-12".into(),
                            label: "12/08/2026".into(),
                            left: 1,
                            right: 1,
                            videos: 0,
                        },
                    ])));
                    let photo = SessionPhotoData {
                        caption: "14:42 · Gauche · Retenue".into(),
                        thumbnail: image.clone(),
                        eye: 1,
                        retained: true,
                        ..Default::default()
                    };
                    state.set_consultation_left_photos(ModelRc::new(VecModel::from(vec![
                        photo.clone(),
                    ])));
                    state.set_consultation_right_photos(ModelRc::new(VecModel::from(vec![
                        SessionPhotoData {
                            caption: "14:44 · Droit".into(),
                            eye: 2,
                            retained: false,
                            ..photo
                        },
                    ])));
                }
                if scenario == "calendar-filter" {
                    state.set_current_tab(2);
                    state.set_calendar_caption("Février 2024".into());
                    state.set_calendar_days(ModelRc::new(VecModel::from(
                        (0..42)
                            .map(|i| {
                                let day = i - 2;
                                CalendarDayData {
                                    day: if day <= 0 {
                                        31 + day
                                    } else if day > 29 {
                                        day - 29
                                    } else {
                                        day
                                    },
                                    date: format!("{day:02}/02/2024").into(),
                                    enabled: (1..=29).contains(&day),
                                    selected: day == 10,
                                }
                            })
                            .collect::<Vec<_>>(),
                    )));
                }
                window.show().unwrap();
                if matches!(scenario, "camera-rotation" | "camera-angle-popup") {
                    state.set_rotation_angle(37);
                }
                if scenario == "dossier-empty" {
                    state.set_consultation_open(true);
                }
                if scenario == "camera-presentation-idle" {
                    state.set_iris_fullscreen(true);
                    state.set_presentation_mode(true);
                }
                if scenario == "camera-presentation" {
                    state.set_iris_fullscreen(true);
                    state.set_presentation_mode(true);
                    window.set_patient_first_name("Alice".into());
                    window.set_patient_last_name("Martin".into());
                    window.set_patient_dossier_number("D-000042".into());
                    window.set_is_recording(true);
                    window.set_recording_duration("00:42".into());
                }
                if scenario == "presentation-notes-error" {
                    state.set_iris_fullscreen(true);
                    state.set_presentation_mode(true);
                    state.set_consultation_open(true);
                    state.set_consultation_paused(true);
                    state.set_consultation_title("D-000042 · Alice Martin".into());
                    state.set_consultation_notes("Notes privées".into());
                }
                if scenario == "camera-narrow-after-capture" {
                    state.set_camera_panel_width(260);
                }
                if matches!(
                    scenario,
                    "camera-after-capture" | "camera-narrow-after-capture"
                ) {
                    // Flush theme/property initialization before comparing geometry.
                    slint::platform::update_timers_and_animations();
                    let _ = window.window().take_snapshot().unwrap();
                    slint::platform::update_timers_and_animations();
                    let before = window.window().take_snapshot().unwrap();
                    window.set_has_last_capture(true);
                    window.set_has_last_capture_thumbnail(true);
                    window.set_last_capture_thumbnail(image.clone());
                    window.set_last_capture_file_name("Gauche_2026-10-08_14-00-00.png".into());
                    window
                        .window()
                        .dispatch_event(WindowEvent::PointerScrolled {
                            position: LogicalPosition::new(200.0, 310.0),
                            delta_x: 0.0,
                            delta_y: -120.0,
                        });
                    let after = window.window().take_snapshot().unwrap();
                    for y in (84.0 * requested_scale) as usize..(240.0 * requested_scale) as usize {
                        for x in
                            (24.0 * requested_scale) as usize..(260.0 * requested_scale) as usize
                        {
                            let offset = y * after.width() as usize + x;
                            let before_pixel = before.as_slice()[offset];
                            let after_pixel = after.as_slice()[offset];
                            assert_eq!(
                                (before_pixel.r, before_pixel.g, before_pixel.b),
                                (after_pixel.r, after_pixel.g, after_pixel.b),
                                "capture and scrolling must keep patient fields fixed at {x},{y}, {width}x{height}, theme={theme}"
                            );
                        }
                    }
                }
                if scenario == "camera-resized" {
                    state.set_camera_panel_width(420);
                }
                if scenario == "viewer-presentation" {
                    state.set_viewer_eye_label("Œil gauche".into());
                    state.set_viewer_notes("Notes privées à masquer".into());
                    state.set_viewer_panel(1);
                    state.set_presentation_mode(true);
                }
                if scenario == "viewer-loading-notes" {
                    state.set_viewer_loading(true);
                    state.set_viewer_panel(1);
                    state.set_viewer_notes("Notes non encore lues".into());
                    window.set_viewer_image(slint::Image::default());
                }
                if scenario == "viewer-image-only" {
                    state.set_image_only(true);
                    state.set_photo_controls_visible(false);
                }
                if scenario == "viewer-resized" {
                    state.set_viewer_panel(2);
                    state.set_viewer_panel_width(400);
                }
                if scenario == "library-small" {
                    state.set_thumbnail_size(0);
                }
                if matches!(scenario, "library-custom" | "library-sparse") {
                    state.set_thumbnail_width(187);
                }
                if scenario == "library-minimum" {
                    state.set_thumbnail_width(80);
                }
                if scenario == "library-large" || scenario == "library-list-large" {
                    state.set_thumbnail_size(2);
                }
                if scenario == "library-list-large" {
                    state.set_library_view(1);
                }
                if scenario == "settings-advanced" {
                    state.set_advanced_settings_expanded(true);
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                            position: LogicalPosition::new((width / 2) as f32, (height / 2) as f32),
                            delta_x: 0.0,
                            delta_y: -5000.0,
                        });
                }
                if scenario == "settings-bottom" {
                    window.window().dispatch_event(WindowEvent::PointerMoved {
                        position: LogicalPosition::new((width / 2) as f32, (height / 2) as f32),
                    });
                    window
                        .window()
                        .dispatch_event(WindowEvent::PointerScrolled {
                            position: LogicalPosition::new((width / 2) as f32, (height / 2) as f32),
                            delta_x: 0.,
                            delta_y: -5000.,
                        });
                }
                if scenario == "viewer-loupe" {
                    window.window().dispatch_event(WindowEvent::PointerMoved {
                        position: LogicalPosition::new(width as f32 / 2.0, height as f32 / 2.0),
                    });
                }
                if scenario == "library-menu" {
                    let position =
                        LogicalPosition::new(435.0, if width < 924 { 172.0 } else { 180.0 });
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
                if matches!(scenario, "viewer-tools-menu" | "viewer-view-menu") {
                    let offset = if scenario == "viewer-tools-menu" {
                        285.0
                    } else {
                        214.0
                    } + if width < 900 { 0.0 } else { 8.0 };
                    let position = LogicalPosition::new(
                        width as f32 - offset,
                        if width < 900 { 30.0 } else { 38.0 },
                    );
                    for pressed in [true, false] {
                        window.window().dispatch_event(if pressed {
                            WindowEvent::PointerPressed {
                                position,
                                button: PointerEventButton::Left,
                            }
                        } else {
                            WindowEvent::PointerReleased {
                                position,
                                button: PointerEventButton::Left,
                            }
                        });
                    }
                }
                if scenario == "calendar-filter" {
                    let position =
                        LogicalPosition::new(355.0, if width < 924 { 238.0 } else { 246.0 });
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
                    assert!(
                        state.get_calendar_open(),
                        "calendar fixture must open its contextual popup"
                    );
                }
                if scenario == "camera-angle-popup" {
                    let position = LogicalPosition::new(width as f32 - 356.0, height as f32 - 36.0);
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
                if scenario == "camera-mirror-popup" {
                    let position = LogicalPosition::new(width as f32 - 290.0, height as f32 - 36.0);
                    for pressed in [true, false] {
                        window.window().dispatch_event(if pressed {
                            WindowEvent::PointerPressed {
                                position,
                                button: PointerEventButton::Left,
                            }
                        } else {
                            WindowEvent::PointerReleased {
                                position,
                                button: PointerEventButton::Left,
                            }
                        });
                    }
                }
                if scenario == "camera-click-selection" {
                    let position = LogicalPosition::new(245.0, height as f32 - 164.0);
                    for pressed in [true, false] {
                        window.window().dispatch_event(if pressed {
                            WindowEvent::PointerPressed {
                                position,
                                button: PointerEventButton::Left,
                            }
                        } else {
                            WindowEvent::PointerReleased {
                                position,
                                button: PointerEventButton::Left,
                            }
                        });
                    }
                    assert_eq!(
                        window.get_selected_eye(),
                        2,
                        "the right-eye selector must remain clickable"
                    );
                }
                if scenario == "viewer-angle-popup" {
                    let panel_width = if width < 1000 { 252.0 } else { 300.0 };
                    let position = LogicalPosition::new(width as f32 - panel_width + 52.0, 336.0);
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
                slint::platform::update_timers_and_animations();
                let mut snapshot = window.window().take_snapshot().unwrap();
                // The software backend can leave the unused alpha byte at zero
                // for an opaque window. Export the visible RGB data as opaque PNG.
                for pixel in snapshot.make_mut_slice() {
                    pixel.a = 255;
                }
                assert!(
                    (window.window().scale_factor() - requested_scale).abs() < f32::EPSILON,
                    "the renderer must apply the requested desktop scaling"
                );
                if scenario != "camera-fullscreen" {
                    let expected =
                        LogicalSize::new(width as f32, height as f32).to_physical(requested_scale);
                    assert_eq!(
                        (snapshot.width(), snapshot.height()),
                        (expected.width, expected.height)
                    );
                }
                assert_eq!(snapshot.width(), window.window().size().width);
                assert_eq!(snapshot.height(), window.window().size().height);
                if scenario == "viewer-comparison" {
                    let inset = if width < 900 { 16.0 } else { 24.0 };
                    let pane = (width as f32 - 2.0 * inset - 12.0) / 2.0;
                    let row = ((height as f32 / 2.0) * requested_scale).round() as usize;
                    let sample = |x: f32| {
                        snapshot.as_slice()[row * snapshot.width() as usize
                            + (x * requested_scale).round() as usize]
                    };
                    let left = sample(inset + pane / 2.0);
                    assert!(
                        left.r > 60 && left.b.abs_diff(100) < 4,
                        "the reference image must be visible"
                    );
                    let right = sample(inset + pane + 12.0 + pane / 2.0);
                    assert!(
                        left.r.abs_diff(right.r) < 4
                            && left.g.abs_diff(right.g) < 4
                            && left.b.abs_diff(right.b) < 4,
                        "identical reference and current photos must occupy separate, equal panes"
                    );
                    let gap = sample(width as f32 / 2.0);
                    assert!(
                        if theme == 1 {
                            gap.r > 220 && gap.g > 220 && gap.b > 220
                        } else {
                            gap.r < 60 && gap.g < 60 && gap.b < 60
                        },
                        "photos must not overlap the separator"
                    );
                }
                if scenario.starts_with("viewer-") {
                    let chrome = snapshot.as_slice()[0];
                    assert!(
                        if theme == 1 {
                            chrome.r > 220 && chrome.g > 220
                        } else {
                            chrome.r < 60 && chrome.g < 60
                        },
                        "viewer chrome must follow the application theme"
                    );
                }
                if scenario == "library-sparse"
                    && std::env::var_os("IRISCOPE_SNAPSHOT_IMAGE").is_none()
                {
                    let inset = if width < 900 { 16.0 } else { 24.0 };
                    let x = ((inset + 20.0) * requested_scale).round() as usize;
                    let y = ((if width < 924 { 330.0 } else { 338.0 }) * requested_scale).round()
                        as usize;
                    let first = snapshot.as_slice()[y * snapshot.width() as usize + x];
                    assert!(
                        first.b.abs_diff(100) < 4,
                        "a sparse library must keep its first thumbnail against the left content inset"
                    );
                }
                let file = Path::new(&output).join(format!(
                    "{scenario}-{width}x{height}-{}.png",
                    if theme == 1 { "light" } else { "dark" }
                ));
                image::save_buffer(
                    &file,
                    snapshot.as_bytes(),
                    snapshot.width(),
                    snapshot.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
                captures += 1;
                if matches!(scenario, "viewer-tools-menu" | "viewer-view-menu") {
                    let position = LogicalPosition::new(
                        width as f32
                            - if scenario == "viewer-tools-menu" {
                                355.0
                            } else {
                                250.0
                            },
                        if width < 900 { 104.0 } else { 112.0 },
                    );
                    for pressed in [true, false] {
                        window.window().dispatch_event(if pressed {
                            WindowEvent::PointerPressed {
                                position,
                                button: PointerEventButton::Left,
                            }
                        } else {
                            WindowEvent::PointerReleased {
                                position,
                                button: PointerEventButton::Left,
                            }
                        });
                    }
                    if scenario == "viewer-tools-menu" {
                        assert_eq!(
                            state.get_viewer_panel(),
                            4,
                            "Tools must open the comparison panel"
                        );
                    } else {
                        assert_eq!(
                            state.get_viewer_zoom(),
                            100,
                            "View must select actual image size"
                        );
                        assert!(!state.get_viewer_fit());
                    }
                }
                if scenario == "patient-search" {
                    let position = LogicalPosition::new(250., 316.);
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
                    assert!(
                        selected_patient.get(),
                        "Centered patient choice must be clickable at {width}x{height}"
                    );
                }
                window.hide().unwrap();
            }
        }
    }
    println!(
        "Captured {captures} images for {} interface scenarios in two themes.",
        scenarios.len()
    );
}
