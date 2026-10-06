//! Abrupt process termination exercises real lock release and restart, not an error mock.
use iriscope_core::library::{
    BackupPhase, backup_library, backup_library_with_progress, restore_library,
};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
#[test]
fn read_only_destination_fails_without_changing_source_data() {
    use std::os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    };
    if let Some(root) = std::env::var_os("IRISCOPE_READONLY_FIXTURE") {
        let root = PathBuf::from(root);
        let error =
            backup_library(&root.join("captures"), &root.join("read-only"), &|| false).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read(root.join("captures/original.jpg")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read_dir(root.join("read-only")).unwrap().count(), 0);
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "iriscope-readonly-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("captures")).unwrap();
    fs::create_dir(root.join("read-only")).unwrap();
    fs::write(root.join("captures/original.jpg"), b"original").unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(root.join("captures"), fs::Permissions::from_mode(0o777)).unwrap();
    fs::set_permissions(root.join("read-only"), fs::Permissions::from_mode(0o555)).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "read_only_destination_fails_without_changing_source_data",
            "--nocapture",
        ])
        .env("IRISCOPE_READONLY_FIXTURE", &root);
    if fs::metadata(&root).unwrap().uid() == 0 {
        command.uid(65534);
    }
    let result = command.output().unwrap();
    fs::set_permissions(root.join("read-only"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn killed_copy_is_never_published_and_restart_preserves_the_source() {
    if let Some(root) = std::env::var_os("IRISCOPE_BACKUP_CRASH_FIXTURE") {
        let root = PathBuf::from(root);
        backup_library_with_progress(
            &root.join("captures"),
            &root.join("backups"),
            &|| false,
            &mut |value| {
                if value.phase == BackupPhase::Copying && value.bytes_completed > 0 {
                    fs::write(root.join("copy-started"), b"ready").unwrap();
                    loop {
                        thread::park();
                    }
                }
            },
        )
        .unwrap();
        panic!("the parent must kill this subprocess during the copy");
    }
    let root = std::env::temp_dir().join(format!(
        "iriscope-killed-backup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("captures")).unwrap();
    fs::create_dir(root.join("backups")).unwrap();
    let original = vec![53_u8; 3 * 1024 * 1024];
    fs::write(root.join("captures/original.avi"), &original).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "killed_copy_is_never_published_and_restart_preserves_the_source",
            "--nocapture",
        ])
        .env("IRISCOPE_BACKUP_CRASH_FIXTURE", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join("copy-started").exists() && Instant::now() < deadline {
        assert!(
            child.try_wait().unwrap().is_none(),
            "copy subprocess exited before the interruption"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let ready = root.join("copy-started").exists();
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    assert!(
        ready,
        "copy did not reach the controlled interruption point"
    );
    assert_eq!(
        fs::read(root.join("captures/original.avi")).unwrap(),
        original
    );
    let incomplete: Vec<_> = fs::read_dir(root.join("backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(incomplete.len(), 1);
    assert!(
        incomplete[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".en-cours")
    );
    assert!(restore_library(&incomplete[0], &root.join("backups"), &|| false).is_err());
    let complete =
        backup_library(&root.join("captures"), &root.join("backups"), &|| false).unwrap();
    let restored = restore_library(&complete.directory, &root.join("backups"), &|| false).unwrap();
    assert_eq!(
        fs::read(restored.directory.join("original.avi")).unwrap(),
        original
    );
    assert_eq!(
        fs::read(root.join("captures/original.avi")).unwrap(),
        original
    );
    fs::remove_dir_all(root).unwrap();
}
