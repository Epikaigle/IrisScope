//! Real production file jobs under Slint, with picker results simulated in an isolated subprocess.
#![cfg(target_os = "linux")]
use iriscope_app::{
    test_support::ControllerHarness,
    ui::{AppState, MainWindow},
};
use iriscope_core::{
    capabilities::FrameRate,
    library::{capture_file_version, create_patient, get_patient},
    settings::AppSettings,
    video::AviMjpegWriter,
};
use slint::ComponentHandle;
use std::{
    cell::Cell,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn wait_until(window: &MainWindow, condition: impl Fn(&MainWindow) -> bool + 'static) {
    let weak = window.as_weak();
    let passed = Rc::new(Cell::new(false));
    let completed = passed.clone();
    let deadline = Instant::now() + Duration::from_secs(15);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(10),
        move || {
            let done = weak.upgrade().is_some_and(|win| condition(&win));
            if done || Instant::now() >= deadline {
                completed.set(done);
                slint::quit_event_loop().unwrap();
            }
        },
    );
    window.show().unwrap();
    slint::run_event_loop_until_quit().unwrap();
    assert!(
        passed.get(),
        "file operation did not reach the expected UI state: busy={} picker={} folder={} storage={} export={}",
        window.global::<AppState>().get_storage_busy(),
        window.global::<AppState>().get_file_dialog_pending(),
        window.get_settings_capture_directory(),
        window.global::<AppState>().get_storage_feedback(),
        window.global::<AppState>().get_export_feedback()
    );
}
fn verify_backup_and_restore(harness: &ControllerHarness, root: &Path) {
    let win = harness.window();
    fs::write(root.join("action"), "backup").unwrap();
    win.global::<AppState>().invoke_backup_library();
    assert!(win.global::<AppState>().get_storage_busy());
    wait_until(win, |win| !win.global::<AppState>().get_storage_busy());
    assert!(
        !win.global::<AppState>().get_storage_feedback_error(),
        "{}",
        win.global::<AppState>().get_storage_feedback()
    );
    assert_eq!(harness.current_settings().backup_history.len(), 1);
    let backup = harness
        .current_settings()
        .last_backup()
        .unwrap()
        .destination
        .clone();
    assert_eq!(
        fs::read(backup.join("photo.jpg")).unwrap(),
        b"original photo"
    );
    fs::write(root.join("action"), "restore").unwrap();
    win.global::<AppState>().invoke_restore_library();
    let original = root.join("captures");
    wait_until(win, move |win| {
        !win.global::<AppState>().get_storage_busy()
            && win.get_settings_capture_directory().as_str() != original.to_str().unwrap()
    });
    let restored = harness.current_settings().capture_directory;
    assert!(
        restored
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("IrisScope-restauree-")
    );
    assert_eq!(
        fs::read(restored.join("photo.jpg")).unwrap(),
        b"original photo"
    );
    assert_eq!(
        get_patient(&restored, 1).unwrap().unwrap().first_name,
        "Émilie"
    );
    assert!(
        win.global::<AppState>()
            .get_backup_last_status()
            .contains("Aucune sauvegarde"),
        "a backup of another directory must not be shown as current"
    );
}
fn verify_cancellation_and_errors(harness: &ControllerHarness, root: &Path) {
    let win = harness.window();
    let large = harness
        .current_settings()
        .capture_directory
        .join("large.bin");
    fs::File::create(&large)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    let before = fs::read_dir(root.join("backups")).unwrap().count();
    let cancelled = Rc::new(Cell::new(false));
    let observed = cancelled.clone();
    fs::write(root.join("action"), "backup").unwrap();
    win.global::<AppState>().invoke_backup_library();
    wait_until(win, move |win| {
        let state = win.global::<AppState>();
        if state.get_storage_progress() > 0 && state.get_storage_busy() && !observed.replace(true) {
            state.invoke_cancel_storage_operation();
        }
        !state.get_storage_busy()
    });
    assert!(cancelled.get());
    assert!(!win.global::<AppState>().get_storage_feedback_error());
    assert!(
        win.global::<AppState>()
            .get_storage_feedback()
            .contains("annulée")
    );
    assert_eq!(fs::metadata(&large).unwrap().len(), 32 * 1024 * 1024);
    assert_eq!(fs::read_dir(root.join("backups")).unwrap().count(), before);
    fs::write(root.join("not-a-folder"), b"unrelated file").unwrap();
    fs::write(root.join("action"), "bad-parent").unwrap();
    win.global::<AppState>().invoke_backup_library();
    wait_until(win, |win| !win.global::<AppState>().get_storage_busy());
    assert!(win.global::<AppState>().get_storage_feedback_error());
    assert!(!win.global::<AppState>().get_file_dialog_pending());
    assert_eq!(
        fs::read(root.join("not-a-folder")).unwrap(),
        b"unrelated file"
    );
    fs::remove_file(large).unwrap();
}
fn verify_export(harness: &ControllerHarness, root: &Path) {
    let win = harness.window();
    let source = harness
        .current_settings()
        .capture_directory
        .join("vidéo.avi");
    let jpeg = iriscope_imaging::encode_rgb8_jpeg(&vec![92_u8; 64 * 64 * 3], 64, 64, 80).unwrap();
    let mut writer =
        AviMjpegWriter::create(&source, 64, 64, FrameRate::new(8, 1).unwrap()).unwrap();
    for _ in 0..16 {
        writer.write_frame(&jpeg).unwrap();
    }
    writer.finish().unwrap();
    let original = fs::read(&source).unwrap();
    win.set_viewer_open(true);
    win.set_viewer_is_video(true);
    win.global::<AppState>()
        .set_viewer_path(source.to_str().unwrap().into());
    win.global::<AppState>()
        .set_viewer_file_version(capture_file_version(&source).unwrap().token().into());
    fs::write(root.join("action"), "export").unwrap();
    win.global::<AppState>().invoke_export_viewer_mp4();
    wait_until(win, |win| !win.global::<AppState>().get_export_busy());
    assert!(
        !win.global::<AppState>().get_export_feedback_error(),
        "{}",
        win.global::<AppState>().get_export_feedback()
    );
    assert_eq!(win.global::<AppState>().get_export_progress(), 100);
    assert!(root.join("export.mp4").is_file());
    assert_eq!(fs::read(source).unwrap(), original);
    win.global::<AppState>().invoke_close_viewer();
}
fn verify_close_during_copy(harness: &ControllerHarness, root: &Path) {
    let win = harness.window();
    let source = harness
        .current_settings()
        .capture_directory
        .join("closing.bin");
    fs::File::create(&source)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    let before = fs::read_dir(root.join("backups")).unwrap().count();
    fs::write(root.join("action"), "backup").unwrap();
    win.global::<AppState>().invoke_backup_library();
    let closed = Rc::new(Cell::new(false));
    let requested = closed.clone();
    let expired = Rc::new(Cell::new(false));
    let timed_out = expired.clone();
    let weak = win.as_weak();
    let deadline = Instant::now() + Duration::from_secs(15);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(10),
        move || {
            let win = weak.upgrade().unwrap();
            let state = win.global::<AppState>();
            if state.get_storage_busy()
                && state.get_storage_progress() > 0
                && !requested.replace(true)
            {
                win.window()
                    .try_dispatch_event(slint::platform::WindowEvent::CloseRequested)
                    .unwrap();
            }
            if Instant::now() >= deadline {
                timed_out.set(true);
                slint::quit_event_loop().unwrap();
            }
        },
    );
    win.show().unwrap();
    slint::run_event_loop_until_quit().unwrap();
    assert!(
        closed.get(),
        "the close request must interrupt a running copy"
    );
    assert!(
        !expired.get(),
        "the production close guard must finish cleanup"
    );
    assert!(harness.is_closing());
    assert_eq!(fs::metadata(source).unwrap().len(), 32 * 1024 * 1024);
    assert_eq!(fs::read_dir(root.join("backups")).unwrap().count(), before);
}
fn verify_close_before_workers(root: &Path) {
    let settings = AppSettings {
        capture_directory: root.join("captures"),
        ..AppSettings::default()
    };
    let mut harness =
        ControllerHarness::with_settings(settings, root.join("queued-settings.json")).unwrap();
    fs::write(root.join("action"), "queued-close").unwrap();
    harness.show_window().unwrap();
    harness
        .window()
        .global::<AppState>()
        .invoke_backup_library();
    assert!(harness.window().global::<AppState>().get_storage_busy());
    harness.request_window_close().unwrap();
    assert!(harness.is_closing());
    harness.start_file_workers();
    let expired = Rc::new(Cell::new(false));
    let timeout = expired.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_secs(15),
        move || {
            timeout.set(true);
            slint::quit_event_loop().unwrap();
        },
    );
    slint::run_event_loop_until_quit().unwrap();
    assert!(!expired.get());
    assert!(
        !root.join("unexpected-picker").exists(),
        "closing must cancel queued work before opening a picker"
    );
}
fn run_child(root: &Path) {
    verify_close_before_workers(root);
    let settings_path = root.join("settings.json");
    let settings = AppSettings {
        capture_directory: root.join("captures"),
        ..AppSettings::default()
    };
    settings.save_to_file(&settings_path).unwrap();
    let mut harness = ControllerHarness::with_settings(settings, settings_path.clone()).unwrap();
    harness.start_file_workers();
    harness.window().set_current_tab(4);
    harness
        .window()
        .global::<AppState>()
        .invoke_set_backup_reminder(true);
    assert!(
        harness
            .window()
            .global::<AppState>()
            .get_backup_reminder_enabled()
    );
    assert!(
        !harness
            .window()
            .global::<AppState>()
            .get_backup_reminder()
            .is_empty()
    );
    verify_backup_and_restore(&harness, root);
    verify_cancellation_and_errors(&harness, root);
    verify_export(&harness, root);
    verify_close_during_copy(&harness, root);
    drop(harness);
    let saved = AppSettings::try_load_from_file(&settings_path).unwrap();
    assert!(saved.backup_reminder_enabled);
    assert_eq!(saved.backup_history.len(), 1);
    assert_ne!(saved.capture_directory, root.join("captures"));
}
#[test]
fn file_operations_update_ui_persist_history_and_cancel_safely() {
    if let Some(root) = std::env::var_os("IRISCOPE_FILE_WORKFLOW_ROOT") {
        run_child(Path::new(&root));
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "iriscope-file-workflow-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("captures")).unwrap();
    fs::create_dir(root.join("backups")).unwrap();
    fs::create_dir(root.join("bin")).unwrap();
    create_patient(&root.join("captures"), "Émilie", "Martin").unwrap();
    fs::write(root.join("captures/photo.jpg"), b"original photo").unwrap();
    let picker = root.join("bin/zenity");
    fs::write(&picker, "#!/usr/bin/env python3\nimport os,sys\nfrom pathlib import Path\nr=Path(os.environ['IRISCOPE_FILE_WORKFLOW_ROOT'])\na=(r/'action').read_text()\nif a=='queued-close':\n (r/'unexpected-picker').write_text('opened')\n sys.exit(0)\nif '--save' in sys.argv: print(r/'export.mp4')\nelif a=='restore':\n p=r/'restore-picker-count'\n n=int(p.read_text()) if p.exists() else 0\n p.write_text(str(n+1))\n print(next((r/'backups').glob('IrisScope-sauvegarde-*')) if n==0 else r/'backups')\nelif a=='bad-parent': print(r/'not-a-folder')\nelse: print(r/'backups')\n").unwrap();
    fs::set_permissions(&picker, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "file_operations_update_ui_persist_history_and_cancel_safely",
            "--nocapture",
        ])
        .env("IRISCOPE_FILE_WORKFLOW_ROOT", &root)
        .env("PATH", path)
        .env("SLINT_BACKEND", "winit-software")
        .env("SLINT_STYLE", "fluent")
        .env("RUST_BACKTRACE", "1")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}/no-session", root.display()),
        )
        .output()
        .unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
