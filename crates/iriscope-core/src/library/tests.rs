//! Library regression tests.
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    session::{CaptureSession, Eye},
    storage::CaptureTimestamp,
};

use super::{
    CaptureFileVersion, CaptureKind, LibraryEntry, LibraryFilter, assign_capture_to_patient,
    assign_capture_to_patient_if_unchanged, capture_file_version, create_patient, get_patient,
    load_library_index, present_library_items, record_capture_metadata,
    resolve_library_candidates_cancellable, save_indexed_capture, save_library_index,
    scan_library_directory, search_patients, try_scan_library_directory,
    try_scan_library_directory_metadata, upgrade_capture_fingerprints,
};

#[test]
fn homonyms_have_distinct_dossiers_and_captures_follow_selected_id() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-homonyms-{unique}"));
    let first = create_patient(&dir, "Émilie", "Martin").expect("first dossier");
    let second = create_patient(&dir, "Émilie", "Martin").expect("second dossier");
    assert_ne!(first.id, second.id);
    assert_ne!(first.dossier_number, second.dossier_number);
    assert_eq!(
        search_patients(&dir, "e\u{301}mi", "MAR")
            .expect("search")
            .len(),
        2
    );

    let file = dir.join("custom.jpg");
    std::fs::write(&file, b"image").expect("capture");
    let mut session = CaptureSession::new("Émilie", "Martin", Eye::Right);
    session.set_patient_id(Some(second.id));
    record_capture_metadata(
        &dir,
        &file,
        &session,
        CaptureKind::Photo,
        CaptureTimestamp::now(),
    )
    .expect("record");
    let entries = try_scan_library_directory(&dir).expect("scan");
    assert_eq!(entries[0].patient_id, Some(second.id));
    assert!(
        !present_library_items(
            &entries,
            &CaptureSession::new("Émilie", "Martin", Eye::Right),
            LibraryFilter::All
        )[0]
        .is_current_patient
    );
    let item = &present_library_items(&entries, &session, LibraryFilter::All)[0];
    assert!(item.is_current_patient);
    assert_eq!(
        item.dossier_number.as_deref(),
        Some(second.dossier_number.as_str())
    );
    assert!(
        super::get_patient(&dir, second.id)
            .expect("lookup")
            .expect("dossier")
            .last_capture
            .is_some()
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn legacy_capture_requires_explicit_dossier_assignment() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-legacy-assignment-{unique}"));
    let patient = create_patient(&dir, "Jean", "Dupont").expect("dossier");
    let file = dir.join("Jean_Dupont_Droit_2026-09-20_14-30-00.jpg");
    std::fs::write(&file, b"image").expect("legacy capture");
    assert_eq!(
        try_scan_library_directory(&dir).expect("scan")[0].patient_id,
        None
    );
    assign_capture_to_patient(&dir, &file, patient.id).expect("explicit assignment");
    assert_eq!(
        try_scan_library_directory(&dir).expect("scan")[0].patient_id,
        Some(patient.id)
    );
    assert!(assign_capture_to_patient(&dir, &file, patient.id).is_err());
    std::fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn metadata_cannot_attach_an_outside_file_to_a_library() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("iriscope-external-capture-{unique}"));
    let library = root.join("library");
    std::fs::create_dir_all(&library).expect("library");
    let external = root.join("external.jpg");
    std::fs::write(&external, b"other").expect("external file");
    let session = CaptureSession::new("Jean", "Dupont", Eye::Right);
    assert_eq!(
        record_capture_metadata(
            &library,
            &external,
            &session,
            CaptureKind::Photo,
            CaptureTimestamp::now()
        )
        .expect_err("external capture")
        .kind(),
        std::io::ErrorKind::InvalidInput,
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn library_scan_reports_unreadable_path_and_allows_missing_directory() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("iris_test_invalid_library_{unique}"));
    std::fs::write(&path, b"not a directory").expect("create regular file");
    assert!(try_scan_library_directory(&path).is_err());
    std::fs::remove_file(&path).expect("remove regular file");
    assert!(
        try_scan_library_directory(&path)
            .expect("missing directory")
            .is_empty()
    );
}

#[test]
fn privacy_anonymizes_other_patients() {
    let mut current_session = CaptureSession::new("Jean", "Dupont", Eye::Right);
    current_session.set_patient_id(Some(1));

    let entries = vec![
        LibraryEntry {
            file_path: PathBuf::from("/captures/Jean_Dupont_Droit_2026-09-20_14-30-00.jpg"),
            file_size: 0,
            file_version: None,
            patient_id: Some(1),
            kind: CaptureKind::Photo,
            first_name: Some("Jean".to_string()),
            last_name: Some("Dupont".to_string()),
            eye: Eye::Right,
            date_str: "2026-09-20".to_string(),
            time_str: "14:30:00".to_string(),
            modified_time: SystemTime::UNIX_EPOCH,
        },
        LibraryEntry {
            file_path: PathBuf::from("/captures/Marie_Curie_Gauche_2026-09-19_10-15-00.jpg"),
            file_size: 0,
            file_version: None,
            patient_id: Some(2),
            kind: CaptureKind::Photo,
            first_name: Some("Marie".to_string()),
            last_name: Some("Curie".to_string()),
            eye: Eye::Left,
            date_str: "2026-09-19".to_string(),
            time_str: "10:15:00".to_string(),
            modified_time: SystemTime::UNIX_EPOCH,
        },
    ];

    let presented = present_library_items(&entries, &current_session, LibraryFilter::All);

    assert_eq!(presented.len(), 2);
    // Current session item displays patient name
    assert!(presented[0].display_title.contains("Jean Dupont"));
    assert!(presented[0].is_current_patient);

    // Previous patient capture is anonymized
    assert_eq!(presented[1].display_title, "Photo Œil Gauche");
    assert!(!presented[1].display_title.contains("Marie"));
    assert!(!presented[1].display_title.contains("Curie"));
    assert!(!presented[1].is_current_patient);
}

#[test]
fn scan_directory_finds_and_sorts_files() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is valid")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iris_test_lib_{unique}"));
    std::fs::create_dir_all(&dir).expect("create test dir");

    let file1 = dir.join("Jean_Dupont_Droit_2026-09-20_14-30-00.jpg");
    std::fs::write(&file1, b"test").expect("write test file");

    let entries = scan_library_directory(&dir);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].first_name.as_deref(), Some("Jean"));
    assert_eq!(entries[0].last_name.as_deref(), Some("Dupont"));
    assert_eq!(entries[0].eye, Eye::Right);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_filename_does_not_confuse_a_person_named_iris_or_underscored_names() {
    let (first, last, eye, _, _) = super::parse_filename("Iris_Dupont_Droit_2026-09-20_14-30-00");
    assert_eq!(first.as_deref(), Some("Iris"));
    assert_eq!(last.as_deref(), Some("Dupont"));
    assert_eq!(eye, Eye::Right);

    let (first, last, eye, date, _) =
        super::parse_filename("Jean_Pierre_Du_Pont_Gauche_2026-09-20_14-30-00");
    assert_eq!(first, None);
    assert_eq!(last, None);
    assert_eq!(eye, Eye::Left);
    assert_eq!(date, "2026-09-20");
}

#[cfg(unix)]
#[test]
fn library_scan_ignores_symlinks_to_external_captures() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("iriscope-library-symlink-{unique}"));
    let library = root.join("library");
    std::fs::create_dir_all(&library).expect("library");
    let external = root.join("outside.jpg");
    std::fs::write(&external, b"outside").expect("external file");
    std::os::unix::fs::symlink(&external, library.join("outside.jpg")).expect("symlink");
    assert!(
        try_scan_library_directory(&library)
            .expect("scan")
            .is_empty()
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}
#[test]
fn index_preserves_identity_with_custom_filename() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is valid")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iris_test_index_{unique}"));
    std::fs::create_dir_all(&dir).expect("create test dir");

    let file = dir.join("capture-personnalisee-001.jpg");
    std::fs::write(&file, b"test").expect("write test file");
    let session = CaptureSession::new("Jean Pierre", "Du Pont", Eye::Left);
    let timestamp = CaptureTimestamp {
        year: 2026,
        month: 9,
        day: 20,
        hour: 18,
        minute: 45,
        second: 12,
    };

    record_capture_metadata(&dir, &file, &session, CaptureKind::Photo, timestamp)
        .expect("record capture metadata");

    let entries = scan_library_directory(&dir);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].first_name.as_deref(), Some("Jean Pierre"));
    assert_eq!(entries[0].last_name.as_deref(), Some("Du Pont"));
    assert_eq!(entries[0].eye, Eye::Left);
    assert_eq!(entries[0].date_str, "2026-09-20");
    assert_eq!(entries[0].time_str, "18:45:12");

    std::fs::remove_dir_all(dir).expect("remove test directory");
}

#[test]
fn concurrent_capture_metadata_updates_keep_every_entry() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is valid")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iris_test_index_concurrent_{unique}"));
    std::fs::create_dir_all(&dir).expect("create test directory");
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|number| {
            let dir = dir.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let file = dir.join(format!("capture-{number}.jpg"));
                std::fs::write(&file, b"test").expect("write capture");
                let session = CaptureSession::new(format!("Patient{number}"), "Test", Eye::Left);
                barrier.wait();
                record_capture_metadata(
                    &dir,
                    &file,
                    &session,
                    CaptureKind::Photo,
                    CaptureTimestamp::now(),
                )
                .expect("record metadata");
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().expect("capture thread completes");
    }
    let entries = scan_library_directory(&dir);
    assert_eq!(entries.len(), 8);
    for number in 0..8 {
        assert!(entries.iter().any(|entry| {
            entry.file_path.ends_with(format!("capture-{number}.jpg"))
                && entry.first_name.as_deref() == Some(format!("Patient{number}").as_str())
        }));
    }
    std::fs::remove_dir_all(dir).expect("remove test directory");
}

#[test]
fn corrupt_index_restores_dossiers_from_backup_before_capture_update() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-corrupt-index-{unique}"));
    std::fs::create_dir_all(&dir).expect("create directory");
    let dossier = create_patient(&dir, "Jean", "Dupont").expect("create backed-up dossier");
    let latest_dossier =
        create_patient(&dir, "Marie", "Dupont").expect("create latest backed-up dossier");
    let original = b"{ broken patient metadata";
    let index = dir.join(super::LIBRARY_INDEX_FILE);
    std::fs::write(&index, original).expect("write broken index");
    let file = dir.join("new.jpg");
    std::fs::write(&file, b"capture").expect("write capture");
    let mut session = CaptureSession::new("Jean", "Dupont", Eye::Left);
    session.set_patient_id(Some(dossier.id));
    assert_eq!(
        super::get_patient(&dir, dossier.id)
            .expect("backup lookup")
            .expect("dossier")
            .id,
        dossier.id
    );
    assert_eq!(
        super::get_patient(&dir, latest_dossier.id)
            .expect("latest backup lookup")
            .expect("latest dossier")
            .id,
        latest_dossier.id
    );
    record_capture_metadata(
        &dir,
        &file,
        &session,
        CaptureKind::Photo,
        CaptureTimestamp::now(),
    )
    .expect("metadata can be saved after quarantine");
    assert_eq!(try_scan_library_directory(&dir).expect("scan").len(), 1);
    let quarantined = std::fs::read_dir(&dir)
        .expect("read directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".corrupt-"))
        })
        .expect("quarantined index");
    assert_eq!(std::fs::read(quarantined).expect("read original"), original);
    assert!(index.exists());
    assert_eq!(
        try_scan_library_directory(&dir).expect("scan")[0].patient_id,
        Some(dossier.id)
    );
    std::fs::remove_dir_all(dir).expect("remove directory");
}

#[test]
fn corrupt_index_without_backup_is_never_silently_replaced() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-unrecoverable-index-{unique}"));
    std::fs::create_dir_all(&dir).expect("create directory");
    let invalid = b"{ broken patient metadata";
    let index = dir.join(super::LIBRARY_INDEX_FILE);
    std::fs::write(&index, invalid).expect("write broken index");
    let error = super::recover_library_index(&dir).expect_err("cannot infer dossier links");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&index).expect("preserved original"), invalid);
    assert!(super::create_patient(&dir, "Jean", "Dupont").is_err());
    assert_eq!(std::fs::read(&index).expect("still preserved"), invalid);
    std::fs::remove_dir_all(dir).expect("remove directory");
}

#[test]
fn interrupted_index_replacement_restores_backup() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-index-backup-{unique}"));
    std::fs::create_dir_all(&dir).expect("create directory");
    let old_file = dir.join("old.jpg");
    std::fs::write(&old_file, b"old").expect("write old file");
    let session = CaptureSession::new("Jean", "Dupont", Eye::Left);
    record_capture_metadata(
        &dir,
        &old_file,
        &session,
        CaptureKind::Photo,
        CaptureTimestamp::now(),
    )
    .expect("write index");
    std::fs::remove_file(dir.join(super::LIBRARY_INDEX_BACKUP_FILE))
        .expect("remove current backup before simulating interruption");
    std::fs::rename(
        dir.join(super::LIBRARY_INDEX_FILE),
        dir.join(super::LIBRARY_INDEX_BACKUP_FILE),
    )
    .expect("simulate interrupted replacement");
    let entries = try_scan_library_directory(&dir).expect("recover index");
    assert_eq!(entries[0].first_name.as_deref(), Some("Jean"));
    assert!(dir.join(super::LIBRARY_INDEX_BACKUP_FILE).exists());
    record_capture_metadata(
        &dir,
        &old_file,
        &session,
        CaptureKind::Photo,
        CaptureTimestamp::now(),
    )
    .expect("restore index on next write");
    assert!(dir.join(super::LIBRARY_INDEX_FILE).exists());
    std::fs::remove_dir_all(dir).expect("remove directory");
}

#[test]
fn replaced_file_does_not_inherit_patient_metadata() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-replaced-capture-{unique}"));
    std::fs::create_dir_all(&dir).expect("create directory");
    let file = dir.join("Jean_Dupont_Droit_2026-09-20_14-30-00.jpg");
    std::fs::write(&file, b"old").expect("write original");
    let session = CaptureSession::new("Jean", "Dupont", Eye::Right);
    record_capture_metadata(
        &dir,
        &file,
        &session,
        CaptureKind::Photo,
        CaptureTimestamp::now(),
    )
    .expect("index original");
    std::fs::write(&file, b"replacement with another size").expect("replace file");
    let entries = try_scan_library_directory(&dir).expect("scan");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].first_name, None);
    assert_eq!(entries[0].last_name, None);
    std::fs::remove_dir_all(dir).expect("remove directory");
}

#[test]
fn selected_dossier_matches_even_when_display_names_differ() {
    let mut session = CaptureSession::new("Émilie", "MARTIN", Eye::Left);
    session.set_patient_id(Some(1));
    let entry = LibraryEntry {
        file_path: PathBuf::from("capture.jpg"),
        file_size: 0,
        file_version: None,
        patient_id: Some(1),
        kind: CaptureKind::Photo,
        first_name: Some("e\u{301}milie".to_owned()),
        last_name: Some("Martin".to_owned()),
        eye: Eye::Left,
        date_str: String::new(),
        time_str: String::new(),
        modified_time: SystemTime::UNIX_EPOCH,
    };
    assert!(present_library_items(&[entry], &session, LibraryFilter::All)[0].is_current_patient);
}

#[test]
fn typed_partial_identity_never_selects_a_dossier() {
    let session = CaptureSession::new("Jean", "", Eye::Left);
    let partial = LibraryEntry {
        file_path: PathBuf::from("partial.jpg"),
        file_size: 0,
        file_version: None,
        patient_id: None,
        kind: CaptureKind::Photo,
        first_name: Some("JEAN".to_owned()),
        last_name: None,
        eye: Eye::Left,
        date_str: String::new(),
        time_str: String::new(),
        modified_time: SystemTime::UNIX_EPOCH,
    };
    let mut complete = partial.clone();
    complete.last_name = Some("Dupont".to_owned());
    let result = present_library_items(&[partial, complete], &session, LibraryFilter::All);
    assert!(!result[0].is_current_patient);
    assert_eq!(result[0].display_title, "Photo Œil Gauche");
    assert!(!result[1].is_current_patient);
}

#[test]
fn metadata_page_only_releases_dossier_after_sha_resolution() {
    let directory = fingerprint_test_directory("metadata-page");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"AAAAAAAAAAAAA").expect("capture");
    let patient_id = index_test_capture(&directory, &path);
    let candidates = try_scan_library_directory_metadata(&directory).expect("cheap scan");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].patient_id_hint, Some(patient_id));
    assert_eq!(
        resolve_library_candidates_cancellable(&directory, &candidates, &|| false)
            .expect("verified page")[0]
            .patient_id,
        Some(patient_id)
    );
    let original_modified = fs::metadata(&path)
        .expect("metadata")
        .modified()
        .expect("mtime");
    fs::write(&path, b"BBBBBBBBBBBBB").expect("same-size replacement");
    OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open replacement")
        .set_modified(original_modified)
        .expect("restore mtime");
    // A stale candidate may abort the page or resolve it as anonymous, but it
    // must never expose the original patient association.
    if let Ok(entries) = resolve_library_candidates_cancellable(&directory, &candidates, &|| false)
    {
        assert_eq!(entries[0].patient_id, None);
    }
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn visible_page_resolution_honors_cancellation() {
    let directory = fingerprint_test_directory("metadata-cancel");
    let path = directory.join("custom.jpg");
    fs::write(&path, vec![42_u8; 256 * 1024]).expect("capture");
    let candidates = try_scan_library_directory_metadata(&directory).expect("cheap scan");
    let checks = std::cell::Cell::new(0);
    let result = resolve_library_candidates_cancellable(&directory, &candidates, &|| {
        checks.set(checks.get() + 1);
        checks.get() >= 4
    });
    assert_eq!(
        result.expect_err("cancelled hash").kind(),
        std::io::ErrorKind::Interrupted
    );
    assert!(checks.get() >= 4);
    fs::remove_dir_all(directory).expect("cleanup");
}

fn fingerprint_test_directory(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("iriscope-fingerprint-{label}-{unique}"));
    fs::create_dir_all(&directory).expect("directory");
    directory
}

fn index_test_capture(directory: &Path, path: &Path) -> u64 {
    let patient = create_patient(directory, "Jean", "Dupont").expect("patient");
    let mut session = CaptureSession::new("Jean", "Dupont", Eye::Right);
    session.set_patient_id(Some(patient.id));
    record_capture_metadata(
        directory,
        path,
        &session,
        CaptureKind::Photo,
        CaptureTimestamp {
            year: 2020,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        },
    )
    .expect("index with content digest");
    patient.id
}

#[test]
fn same_size_same_mtime_edit_cannot_keep_the_previous_dossier() {
    let directory = fingerprint_test_directory("equal-metadata-edit");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"original capture").expect("original");
    let patient_id = index_test_capture(&directory, &path);
    let before = capture_file_version(&path).expect("original version");
    let original_modified = fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(
        try_scan_library_directory(&directory).unwrap()[0].patient_id,
        Some(patient_id)
    );
    fs::write(&path, b"another! capture").expect("same length replacement");
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_modified)
        .expect("restore mtime");
    let entries = try_scan_library_directory(&directory).expect("scan replacement");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file_size, 16);
    assert_eq!(entries[0].modified_time, original_modified);
    assert_eq!(entries[0].patient_id, None);
    assert_eq!(entries[0].first_name, None);
    assert_eq!(entries[0].eye, Eye::Unspecified);
    if before.is_strong() {
        assert_ne!(entries[0].file_version.as_ref(), Some(&before));
    }
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn identical_backup_copy_preserves_its_dossier_and_capture_date() {
    let directory = fingerprint_test_directory("backup-copy");
    let path = directory.join("custom.jpg");
    let backup = directory.join("restored.data");
    fs::write(&path, b"original capture").expect("original");
    let patient_id = index_test_capture(&directory, &path);
    let original_version = capture_file_version(&path).expect("original version");
    fs::copy(&path, &backup).expect("backup byte copy");
    fs::remove_file(&path).expect("remove original");
    fs::rename(&backup, &path).expect("restore backup with different inode");
    let entries = try_scan_library_directory(&directory).expect("verify backup digest");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].patient_id, Some(patient_id));
    assert_eq!(entries[0].eye, Eye::Right);
    assert_eq!(entries[0].date_str, "2020-01-02");
    if original_version.is_strong() {
        assert_ne!(entries[0].file_version.as_ref(), Some(&original_version));
    }
    assert_eq!(
        try_scan_library_directory(&directory).unwrap()[0].patient_id,
        Some(patient_id)
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn assignment_rejects_a_replacement_since_the_capture_was_displayed() {
    let directory = fingerprint_test_directory("stale-assignment");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"original capture").expect("original");
    let patient = create_patient(&directory, "Jean", "Dupont").expect("patient");
    let displayed_version = capture_file_version(&path).expect("displayed version");
    assert_eq!(
        CaptureFileVersion::from_token(&displayed_version.token()).unwrap(),
        displayed_version
    );
    fs::write(&path, b"another! capture").expect("replacement");
    let result =
        assign_capture_to_patient_if_unchanged(&directory, &path, patient.id, &displayed_version);
    assert!(result.is_err());
    assert_eq!(
        try_scan_library_directory(&directory).unwrap()[0].patient_id,
        None
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn reassigned_replacement_does_not_inherit_historical_eye_or_date() {
    let directory = fingerprint_test_directory("stale-anonymous-fields");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"original capture").expect("original");
    record_capture_metadata(
        &directory,
        &path,
        &CaptureSession::new("", "", Eye::Right),
        CaptureKind::Photo,
        CaptureTimestamp {
            year: 2020,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        },
    )
    .expect("anonymous indexed capture");
    fs::write(&path, b"another! capture").expect("replacement");
    let patient = create_patient(&directory, "Jean", "Dupont").expect("patient");
    let version = capture_file_version(&path).expect("current version");
    assign_capture_to_patient_if_unchanged(&directory, &path, patient.id, &version)
        .expect("assign reviewed replacement");
    let entries = try_scan_library_directory(&directory).expect("scan assigned capture");
    assert_eq!(entries[0].patient_id, Some(patient.id));
    assert_eq!(entries[0].eye, Eye::Unspecified);
    assert_ne!(entries[0].date_str, "2020-01-02");
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn legacy_dossier_fingerprint_upgrade_protects_future_equal_metadata_edits() {
    let directory = fingerprint_test_directory("legacy-upgrade");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"original capture").expect("original");
    let patient_id = index_test_capture(&directory, &path);
    let mut index = load_library_index(&directory).expect("index");
    let legacy = index.entries.get_mut("custom.jpg").expect("capture");
    legacy.content_sha256 = None;
    legacy.file_version = None;
    save_library_index(&directory, &index).expect("legacy index");
    assert!(try_scan_library_directory(&directory).is_err());
    assert_eq!(
        upgrade_capture_fingerprints(&directory).expect("upgrade"),
        1
    );
    assert_eq!(
        try_scan_library_directory(&directory).unwrap()[0].patient_id,
        Some(patient_id)
    );
    assert_eq!(
        upgrade_capture_fingerprints(&directory).expect("idempotent upgrade"),
        0
    );
    let original_modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, b"another! capture").expect("same length replacement");
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_modified)
        .expect("restore mtime");
    assert_eq!(
        try_scan_library_directory(&directory).unwrap()[0].patient_id,
        None
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn inconsistent_stored_digest_cannot_use_matching_version_shortcut() {
    let directory = fingerprint_test_directory("digest-shortcut");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"thirteen bytes").expect("capture");
    index_test_capture(&directory, &path);
    let mut index = load_library_index(&directory).expect("index");
    let stored = index
        .entries
        .get_mut("custom.jpg")
        .expect("capture metadata");
    let actual = stored.content_sha256.expect("recorded digest");
    let mut wrong = actual;
    wrong[0] ^= 0xff;
    stored.content_sha256 = Some(wrong);
    save_library_index(&directory, &index).expect("inconsistent index");
    let entries = try_scan_library_directory(&directory).expect("scan");
    assert_eq!(entries[0].patient_id, None);
    assert_eq!(entries[0].first_name, None);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn transitional_index_does_not_rebaseline_a_changed_capture() {
    let directory = fingerprint_test_directory("transitional-digest");
    let path = directory.join("custom.jpg");
    fs::write(&path, b"AAAAAAAAAAAAA").expect("capture");
    index_test_capture(&directory, &path);
    let original_modified = fs::metadata(&path)
        .expect("metadata")
        .modified()
        .expect("mtime");
    let mut index = load_library_index(&directory).expect("index");
    index
        .entries
        .get_mut("custom.jpg")
        .expect("capture")
        .content_sha256 = None;
    save_library_index(&directory, &index).expect("transitional index");
    fs::write(&path, b"BBBBBBBBBBBBB").expect("same-size edit");
    OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open")
        .set_modified(original_modified)
        .expect("restore mtime");
    assert_eq!(
        upgrade_capture_fingerprints(&directory).expect("upgrade"),
        0
    );
    assert!(try_scan_library_directory(&directory).is_err());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn version_one_index_without_digest_upgrades_without_guessing_dossier() {
    let directory = fingerprint_test_directory("version-one-upgrade");
    let path = directory.join("Jean_Dupont_Droit_2020-01-02_03-04-05.jpg");
    fs::write(&path, b"legacy bytes!").expect("legacy capture");
    record_capture_metadata(
        &directory,
        &path,
        &CaptureSession::new("Jean", "Dupont", Eye::Right),
        CaptureKind::Photo,
        CaptureTimestamp {
            year: 2020,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        },
    )
    .expect("initial index");
    let mut old: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.join(super::LIBRARY_INDEX_FILE)).expect("index bytes"),
    )
    .expect("index JSON");
    old["version"] = 1.into();
    let entry = old["entries"]
        .as_object_mut()
        .expect("entries")
        .get_mut("Jean_Dupont_Droit_2020-01-02_03-04-05.jpg")
        .expect("entry")
        .as_object_mut()
        .expect("entry object");
    entry.remove("content_sha256");
    entry.remove("file_version");
    entry.remove("patient_id");
    old.as_object_mut()
        .expect("index object")
        .remove("patients");
    old.as_object_mut()
        .expect("index object")
        .remove("next_patient_id");
    fs::write(
        directory.join(super::LIBRARY_INDEX_FILE),
        serde_json::to_vec(&old).expect("serialize old index"),
    )
    .expect("write old index");
    assert_eq!(
        upgrade_capture_fingerprints(&directory).expect("upgrade"),
        1
    );
    let upgraded = load_library_index(&directory).expect("upgraded index");
    let stored = upgraded
        .entries
        .get("Jean_Dupont_Droit_2020-01-02_03-04-05.jpg")
        .expect("entry");
    assert!(stored.content_sha256.is_some());
    assert!(
        stored
            .file_version
            .as_ref()
            .is_some_and(|version| version.content_digest().is_some())
    );
    let patient = create_patient(&directory, "Jean", "Dupont").expect("same-name dossier");
    let entry = &try_scan_library_directory(&directory).expect("scan")[0];
    assert_ne!(entry.patient_id, Some(patient.id));
    assert_eq!(entry.patient_id, None);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn scan_of_read_only_library_does_not_create_lock_file() {
    use std::os::unix::fs::PermissionsExt;

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("iriscope-readonly-library-{unique}"));
    std::fs::create_dir_all(&dir).expect("create directory");
    std::fs::write(dir.join("Iris_Droit_2026-09-20_14-30-00.jpg"), b"capture")
        .expect("create capture");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).expect("make read only");
    let entries = try_scan_library_directory(&dir).expect("scan read-only library");
    assert_eq!(entries.len(), 1);
    assert!(!dir.join(super::LIBRARY_INDEX_LOCK_FILE).exists());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore permission");
    std::fs::remove_dir_all(dir).expect("remove directory");
}

#[test]
fn patient_search_cache_tracks_creations_and_directory_switches() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let first_dir = std::env::temp_dir().join(format!("iriscope-search-cache-{unique}"));
    let second_dir = first_dir.join("other");
    let first = create_patient(&first_dir, "Émilie", "Martin").unwrap();
    let mut cache = super::PatientSearchCache::default();
    assert_eq!(
        cache.search(&first_dir, "e\u{301}mi", "MAR", 25).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        cache.search(&first_dir, "ÉMILIE", "martin", 25).unwrap(),
        vec![first.clone()]
    );
    let second = create_patient(&first_dir, "Émilie", "Martin").unwrap();
    assert_eq!(
        cache.search(&first_dir, "Émi", "Mar", 25).unwrap(),
        vec![first.clone(), second]
    );
    assert_eq!(
        cache.search(&first_dir, "Émi", "Mar", 1).unwrap(),
        vec![first]
    );
    let other = create_patient(&second_dir, "Émilie", "Moreau").unwrap();
    assert_eq!(
        cache.search(&second_dir, "Émi", "Mo", 25).unwrap(),
        vec![other]
    );
    assert_eq!(cache.search(&first_dir, "Émi", "Mar", 25).unwrap().len(), 2);
    let backup = first_dir.join(super::LIBRARY_INDEX_BACKUP_FILE);
    std::fs::remove_file(&backup).unwrap();
    std::fs::create_dir(&backup).unwrap();
    assert_eq!(
        cache.search(&first_dir, "Émi", "Mar", 25).unwrap().len(),
        2,
        "an unavailable backup must not hide dossiers from a readable primary index"
    );
    std::fs::remove_dir_all(first_dir).unwrap();
}

#[test]
fn correcting_names_preserves_identity_captures_and_rejects_concurrent_edits() {
    let directory = fingerprint_test_directory("correct-dossier");
    let patient = create_patient(&directory, "Jeam", "Dupont").unwrap();
    let mut session = CaptureSession::new("Jeam", "Dupont", Eye::Right);
    session.set_patient_id(Some(patient.id));
    let capture = save_indexed_capture(
        &directory,
        "unchanged.jpg",
        b"captured bytes",
        &session,
        CaptureKind::Photo,
        CaptureTimestamp {
            year: 2026,
            month: 10,
            day: 5,
            hour: 12,
            minute: 0,
            second: 0,
        },
    )
    .unwrap();
    let corrected =
        super::update_patient(&directory, patient.id, "Jeam", "Dupont", "Jean", "Dupont").unwrap();
    assert_eq!(corrected.id, patient.id);
    assert_eq!(corrected.dossier_number, patient.dossier_number);
    assert_eq!(
        corrected.last_capture,
        get_patient(&directory, patient.id)
            .unwrap()
            .unwrap()
            .last_capture
    );
    let entries = try_scan_library_directory(&directory).unwrap();
    assert_eq!(entries[0].patient_id, Some(patient.id));
    assert_eq!(entries[0].first_name.as_deref(), Some("Jean"));
    assert!(capture.file_path.exists());
    assert!(
        super::update_patient(&directory, patient.id, "Jeam", "Dupont", "Other", "Name").is_err()
    );
    assert!(
        super::update_patient(&directory, patient.id, "Jean", "Dupont", " ", "Dupont").is_err()
    );
    assert_eq!(
        get_patient(&directory, patient.id)
            .unwrap()
            .unwrap()
            .first_name,
        "Jean"
    );
    fs::remove_dir_all(directory).unwrap();
}
