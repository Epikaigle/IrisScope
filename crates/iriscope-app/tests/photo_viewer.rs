//! End-to-end photo tools with the real reader, controllers, autosave and exports.
#![cfg(target_os = "linux")]
#![allow(clippy::too_many_lines)] // One isolated workflow owns the complete reader/save/export lifecycle.
use iriscope_app::{
    test_support::ControllerHarness,
    ui::{AppState, LibraryItemData, MainWindow},
};
use iriscope_core::{
    library::{self, CaptureKind},
    session::{CaptureSession, Eye},
    storage::CaptureTimestamp,
};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::Cell,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn wait(window: &MainWindow, condition: impl Fn(&MainWindow) -> bool + 'static) {
    let weak = window.as_weak();
    let done = Rc::new(Cell::new(false));
    let passed = done.clone();
    let deadline = Instant::now() + Duration::from_secs(15);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(10),
        move || {
            if let Some(win) = weak.upgrade()
                && condition(&win)
            {
                passed.set(true);
                slint::quit_event_loop().unwrap();
            } else if Instant::now() > deadline {
                slint::quit_event_loop().unwrap();
            }
        },
    );
    window.show().unwrap();
    slint::run_event_loop_until_quit().unwrap();
    assert!(
        done.get(),
        "{} / {}",
        window.global::<AppState>().get_viewer_feedback(),
        window.global::<AppState>().get_export_feedback()
    );
}
fn open(win: &MainWindow, row: &LibraryItemData) {
    win.global::<AppState>()
        .invoke_open_capture_file(row.file_path.clone(), row.file_version.clone());
    wait(win, |w| !w.get_viewer_loading() && w.get_viewer_open());
}
fn escape(win: &MainWindow) {
    win.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    win.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
}
fn child(root: &Path) {
    let captures = root.join("captures");
    let patient = library::create_patient(&captures, "Émilie", "Martin").unwrap();
    let mut rows = Vec::new();
    for number in 0..3 {
        let path = captures.join(format!("photo-{number}.png"));
        let image = image::RgbImage::from_fn(120, 80, |x, y| {
            image::Rgb([u8::try_from(x).unwrap(), u8::try_from(y).unwrap(), 40])
        });
        image.save(&path).unwrap();
        let eye = if number < 2 { Eye::Left } else { Eye::Right };
        let mut session = CaptureSession::new("Émilie", "Martin", eye);
        session.set_patient_id(Some(patient.id));
        library::record_capture_metadata(
            &captures,
            &path,
            &session,
            CaptureKind::Photo,
            CaptureTimestamp::now(),
        )
        .unwrap();
        rows.push(LibraryItemData {
            file_path: path.to_string_lossy().into_owned().into(),
            file_version: library::capture_file_version(&path).unwrap().token().into(),
            dossier_number: patient.dossier_number.clone().into(),
            eye_label: if eye == Eye::Left {
                "Œil gauche"
            } else {
                "Œil droit"
            }
            .into(),
            date_time: format!("2026-10-0{} 12:00", number + 1).into(),
            can_open_in_app: true,
            ..Default::default()
        });
    }
    let original = fs::read(rows[0].file_path.as_str()).unwrap();
    let mut harness = ControllerHarness::new(captures).unwrap();
    harness.start_file_workers();
    harness.start_viewer_worker();
    let win = harness.window();
    win.set_library_items(ModelRc::new(VecModel::from(rows.clone())));
    win.set_library_loading(false);
    let state = win.global::<AppState>();
    state.set_iris_fullscreen(true);
    state.invoke_select_photo_panel(1);
    state.invoke_open_capture_file(rows[0].file_path.clone(), rows[0].file_version.clone());
    assert!(state.get_viewer_loading());
    assert_eq!(
        state.get_viewer_panel(),
        0,
        "remembered notes must stay closed until the review has been read"
    );
    wait(win, |w| !w.get_viewer_loading() && w.get_viewer_open());
    assert_eq!(
        state.get_viewer_panel(),
        1,
        "the remembered notes panel returns after loading"
    );
    assert!(state.get_viewer_fullscreen());
    assert!(!state.get_iris_fullscreen());
    state.invoke_dismiss_top_overlay();
    assert!(!state.get_viewer_fullscreen());
    assert_eq!(
        (
            state.get_viewer_original().size().width,
            state.get_viewer_original().size().height
        ),
        (120, 80)
    );
    state.set_viewer_retained(true);
    state.invoke_viewer_review_edited();
    state.set_viewer_panel(1);
    state.set_viewer_notes("Éclairage · œil gauche · observation datée".into());
    state.invoke_viewer_review_edited();
    state.invoke_add_viewer_annotation(1, 0.5, 0.5, 0.7, 0.5);
    state.set_viewer_annotation_text("Zone observée".into());
    state.invoke_add_viewer_annotation(4, 0.1, 0.1, 0.1, 0.1);
    assert_eq!(state.get_viewer_annotations().row_count(), 2);
    state.invoke_delete_viewer_annotation(0);
    assert_eq!(state.get_viewer_annotations().row_count(), 1);
    state.invoke_undo_viewer_annotation();
    assert_eq!(state.get_viewer_annotations().row_count(), 2);
    wait(win, |w| !w.global::<AppState>().get_viewer_review_dirty());
    let version = library::CaptureFileVersion::from_token(rows[0].file_version.as_str()).unwrap();
    let saved =
        library::load_photo_review(Path::new(rows[0].file_path.as_str()), &version, &|| false)
            .unwrap();
    assert_eq!(saved.annotations.len(), 2);
    assert!(saved.notes.contains("œil"));
    state.set_viewer_notes("Première version".into());
    state.invoke_viewer_review_edited();
    state.invoke_save_viewer_review();
    assert!(state.get_viewer_review_busy());
    state.set_viewer_notes("Saisie poursuivie pendant l’enregistrement".into());
    state.invoke_viewer_review_edited();
    wait(win, |w| !w.global::<AppState>().get_viewer_review_dirty());
    assert_eq!(
        library::load_photo_review(Path::new(rows[0].file_path.as_str()), &version, &|| false)
            .unwrap()
            .notes,
        "Saisie poursuivie pendant l’enregistrement"
    );
    state.set_viewer_notes("Note enregistrée avant navigation".into());
    state.invoke_viewer_review_edited();
    state.invoke_open_capture_file(rows[1].file_path.clone(), rows[1].file_version.clone());
    let expected = rows[1].file_path.clone();
    wait(win, move |w| {
        !w.get_viewer_loading() && w.global::<AppState>().get_viewer_path() == expected
    });
    assert!(state.get_viewer_notes().is_empty());
    open(win, &rows[0]);
    assert_eq!(
        state.get_viewer_notes(),
        "Note enregistrée avant navigation"
    );
    state.invoke_find_comparison_photos();
    wait(win, |w| !w.global::<AppState>().get_comparison_loading());
    assert_eq!(
        state.get_comparison_candidates().row_count(),
        1,
        "only the same patient and eye"
    );
    let reference = state.get_comparison_candidates().row_data(0).unwrap();
    state.invoke_select_comparison_photo(reference.path, reference.version, reference.caption);
    wait(win, |w| w.global::<AppState>().get_comparison_enabled());
    state.set_comparison_linked(false);
    state.set_viewer_fit(false);
    state.set_comparison_fit(false);
    state.set_viewer_zoom(800);
    state.set_comparison_zoom(600);
    state.set_viewer_pan_x(20.0);
    state.set_comparison_pan_x(20.0);
    state.set_viewer_effective_zoom(800.0);
    state.set_comparison_effective_zoom(600.0);
    state.invoke_toggle_comparison_link();
    assert!((state.get_comparison_zoom_ratio() - 0.75).abs() < 0.01);
    assert!((state.get_comparison_pan_x() - 20.0).abs() < 0.1);
    state.set_viewer_pan_x(10.0);
    state.invoke_link_view(false);
    wait(win, |w| {
        (w.global::<AppState>().get_comparison_pan_x() - 12.5).abs() < 0.1
    });
    state.set_comparison_linked(false);
    state.set_viewer_fit(true);
    state.set_comparison_fit(true);
    state.set_viewer_pan_x(0.0);
    state.set_comparison_pan_x(0.0);
    state.set_viewer_panel(3);
    state.set_viewer_brightness(20);
    state.set_viewer_rotation(90);
    state.invoke_viewer_display_changed();
    wait(win, |w| !w.global::<AppState>().get_viewer_adjusting());
    assert_eq!(
        (
            win.get_viewer_image().size().width,
            win.get_viewer_image().size().height
        ),
        (80, 120)
    );
    state.set_viewer_show_original(true);
    state.invoke_viewer_display_changed();
    wait(win, |w| !w.global::<AppState>().get_viewer_adjusting());
    assert_eq!(
        (
            win.get_viewer_image().size().width,
            win.get_viewer_image().size().height
        ),
        (120, 80)
    );
    state.set_viewer_show_original(false);
    state.invoke_viewer_display_changed();
    wait(win, |w| !w.global::<AppState>().get_viewer_adjusting());
    state.set_viewer_fullscreen(true);
    state.invoke_dismiss_top_overlay();
    assert!(!state.get_viewer_fullscreen());
    assert!(
        win.get_viewer_open(),
        "Escape exits fullscreen before closing photo"
    );
    state.set_viewer_panel(4);
    fs::write(root.join("choice"), "copy.png").unwrap();
    state.invoke_export_viewer_photo(0);
    wait(win, |w| !w.global::<AppState>().get_export_busy());
    assert!(
        !state.get_export_feedback_error(),
        "{}",
        state.get_export_feedback()
    );
    assert_eq!(
        image::open(root.join("copy.png"))
            .unwrap()
            .to_rgb8()
            .dimensions(),
        (80, 120)
    );
    assert_eq!(fs::read(rows[0].file_path.as_str()).unwrap(), original);
    fs::write(root.join("choice"), "observations.pdf").unwrap();
    state.invoke_export_viewer_photo(1);
    wait(win, |w| !w.global::<AppState>().get_export_busy());
    assert!(
        !state.get_export_feedback_error(),
        "{}",
        state.get_export_feedback()
    );
    let pdf = fs::read(root.join("observations.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(String::from_utf8_lossy(&pdf).contains("/Im1"));
    state.invoke_export_viewer_photo(1);
    wait(win, |w| !w.global::<AppState>().get_export_busy());
    assert!(
        state.get_export_feedback_error(),
        "existing export must not be overwritten"
    );
    assert_eq!(fs::read(root.join("observations.pdf")).unwrap(), pdf);
    state.set_viewer_notes("Dernière note avant fermeture".into());
    state.invoke_viewer_review_edited();
    state.invoke_close_viewer();
    wait(win, |w| !w.get_viewer_open());
    state.set_library_query_from("10/02/2024".into());
    state.invoke_open_calendar(1);
    assert_eq!(state.get_calendar_caption(), "Février 2024");
    assert!(
        state
            .get_calendar_days()
            .iter()
            .any(|d| d.day == 29 && d.enabled)
    );
    state.invoke_calendar_step(1);
    assert_eq!(state.get_calendar_caption(), "Mars 2024");
    assert!(!state.get_calendar_days().iter().any(|d| d.selected));
    state.invoke_select_calendar_date("12/03/2024".into());
    assert_eq!(state.get_library_query_from(), "12/03/2024");
    assert!(!state.get_calendar_open());
    state.invoke_open_calendar(1);
    escape(win);
    wait(win, |w| !w.global::<AppState>().get_calendar_open());
    state.invoke_open_consultation();
    escape(win);
    wait(win, |w| !w.global::<AppState>().get_consultation_open());
    state.invoke_open_consultation();
    state.set_dossier_search("Martin".into());
    state.invoke_search_dossiers();
    wait(win, |w| !w.global::<AppState>().get_dossier_search_busy());
    assert_eq!(state.get_dossier_candidates().row_count(), 1);
    state.invoke_select_dossier(patient.id.to_string().into());
    wait(win, |w| {
        !w.global::<AppState>().get_consultation_loading()
            && w.global::<AppState>().get_consultation_photos().row_count() == 3
    });
    assert_eq!(state.get_consultation_left_photos().row_count(), 2);
    assert_eq!(state.get_consultation_right_photos().row_count(), 1);
    assert!(
        state
            .get_consultation_photos()
            .iter()
            .any(|p| p.retained && p.thumbnail.size().width > 0)
    );
    open(win, &rows[0]);
    assert!(state.get_viewer_retained());
    state.set_viewer_retained(false);
    state.invoke_viewer_review_edited();
    state.invoke_close_viewer();
    wait(win, |w| {
        !w.get_viewer_open()
            && !w.global::<AppState>().get_consultation_loading()
            && w.global::<AppState>().get_consultation_photos().row_count() == 3
            && !w
                .global::<AppState>()
                .get_consultation_photos()
                .iter()
                .any(|p| p.retained)
    });
    state.set_consultation_notes("Première saisie".into());
    state.invoke_consultation_edited();
    state.invoke_save_consultation_notes();
    state.set_consultation_notes("Saisie poursuivie pendant l’enregistrement".into());
    state.invoke_consultation_edited();
    wait(win, |w| !w.global::<AppState>().get_consultation_dirty());
    let directory = Path::new(state.get_consultation_directory().as_str()).to_path_buf();
    let day = state.get_consultation_day().to_string();
    assert_eq!(
        library::load_consultation_notes(&directory, patient.id, &day)
            .unwrap()
            .notes,
        "Saisie poursuivie pendant l’enregistrement"
    );
    state.set_consultation_notes("x".repeat(32_001).into());
    state.invoke_consultation_edited();
    state.invoke_save_consultation_notes();
    wait(win, |w| w.global::<AppState>().get_consultation_paused());
    assert!(state.get_consultation_dirty());
    assert!(state.get_consultation_open());
    state.set_consultation_notes("Notes générales avant fermeture".into());
    state.invoke_consultation_edited();
    state.invoke_close_consultation();
    wait(win, |w| !w.global::<AppState>().get_consultation_open());
    assert_eq!(
        library::load_consultation_notes(&directory, patient.id, &day)
            .unwrap()
            .notes,
        "Notes générales avant fermeture"
    );
    win.set_patient_id(patient.id.to_string().into());
    win.set_patient_first_name("Émilie".into());
    win.set_patient_last_name("Martin".into());
    win.set_selected_eye(1);
    state.invoke_session_changed();
    state.set_session_left_count(1);
    win.set_session_photo_count(1);
    win.set_selected_eye(2);
    state.invoke_session_changed();
    assert_eq!(state.get_session_left_count(), 1);
    assert_eq!(win.get_session_photo_count(), 1);
    state.invoke_clear_session();
    assert_eq!(state.get_session_left_count(), 0);
    open(win, &rows[0]);
    assert_eq!(state.get_viewer_notes(), "Dernière note avant fermeture");
    fs::write(rows[0].file_path.as_str(), b"changed external photo").unwrap();
    state.set_viewer_notes("Brouillon à conserver".into());
    state.invoke_viewer_review_edited();
    state.invoke_close_viewer();
    wait(win, |w| w.global::<AppState>().get_viewer_feedback_error());
    assert!(win.get_viewer_open());
    assert!(state.get_viewer_review_dirty());
    assert_eq!(state.get_viewer_notes(), "Brouillon à conserver");
    state.set_viewer_review_dirty(false);
    state.invoke_close_viewer();
    drop(harness);
}
#[test]
fn photo_tools_preserve_observations_sources_and_export_copies() {
    if let Some(root) = std::env::var_os("IRISCOPE_PHOTO_WORKFLOW_ROOT") {
        child(Path::new(&root));
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "iriscope-photo-workflow-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("captures")).unwrap();
    fs::create_dir(root.join("bin")).unwrap();
    let picker = root.join("bin/zenity");
    fs::write(&picker,"#!/usr/bin/env python3\nimport os\nfrom pathlib import Path\nr=Path(os.environ['IRISCOPE_PHOTO_WORKFLOW_ROOT'])\nprint(r/(r/'choice').read_text())\n").unwrap();
    fs::set_permissions(picker, fs::Permissions::from_mode(0o755)).unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "photo_tools_preserve_observations_sources_and_export_copies",
            "--nocapture",
        ])
        .env("IRISCOPE_PHOTO_WORKFLOW_ROOT", &root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("SLINT_BACKEND", "winit-software")
        .env("SLINT_STYLE", "fluent")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-session", root.display()),
        )
        .output()
        .unwrap();
    if let Ok(destination) = std::env::var("IRISCOPE_TEST_PDF_OUTPUT") {
        let _ = fs::copy(root.join("observations.pdf"), destination);
    }
    fs::remove_dir_all(root).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
