use super::*;
use ed25519_dalek::{Signer, SigningKey};

fn manifest() -> Manifest {
    Manifest {
        schema: 1,
        version: "0.4.2".into(),
        assets: vec![Asset {
            platform: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            kind: "macos-bundle".into(),
            name: "IrisScope-0.4.2-macos-x86_64-update.tar.gz".into(),
            size: 3,
            sha256: format!("{:x}", Sha256::digest(b"abc")),
        }],
    }
}

#[test]
fn only_the_trusted_signature_can_authorize_update_metadata() {
    let key = SigningKey::from_bytes(&[37; 32]);
    let bytes = serde_json::to_vec(&manifest()).unwrap();
    let signature = key.sign(&bytes).to_bytes();
    assert!(verify_manifest_with_key(&bytes, &signature, &key.verifying_key().to_bytes()).is_ok());
    assert!(
        verify_manifest_with_key(
            &bytes,
            &signature,
            &SigningKey::from_bytes(&[39; 32]).verifying_key().to_bytes()
        )
        .is_err()
    );
    let mut tampered = bytes;
    tampered[20] ^= 1;
    assert!(
        verify_manifest_with_key(&tampered, &signature, &key.verifying_key().to_bytes()).is_err()
    );
    assert!(
        verify_manifest_with_key(&tampered, &[0; 63], &key.verifying_key().to_bytes()).is_err()
    );
}

#[test]
fn signed_manifest_rejects_unsafe_names_and_unbounded_downloads() {
    let key = SigningKey::from_bytes(&[37; 32]);
    for name in ["../archive.tar.gz", "/archive", "archive?token=x", "..", ""] {
        let mut manifest = manifest();
        manifest.assets[0].name = name.into();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(
            verify_manifest_with_key(
                &bytes,
                &key.sign(&bytes).to_bytes(),
                &key.verifying_key().to_bytes()
            )
            .is_err()
        );
    }
    let mut manifest = manifest();
    manifest.assets[0].size = MAX_ARCHIVE_BYTES + 1;
    let bytes = serde_json::to_vec(&manifest).unwrap();
    assert!(
        verify_manifest_with_key(
            &bytes,
            &key.sign(&bytes).to_bytes(),
            &key.verifying_key().to_bytes()
        )
        .is_err()
    );
}

#[test]
fn archive_requires_exact_length_and_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive");
    std::fs::write(&path, b"abc").unwrap();
    assert!(verify_archive(&path, &manifest().assets[0]).is_ok());
    std::fs::write(&path, b"abd").unwrap();
    assert!(verify_archive(&path, &manifest().assets[0]).is_err());
    std::fs::write(&path, b"ab").unwrap();
    assert!(verify_archive(&path, &manifest().assets[0]).is_err());
}

#[test]
fn extraction_refuses_symlinks_before_they_can_escape_staging() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.gz");
    let gzip =
        flate2::write::GzEncoder::new(File::create(&path).unwrap(), flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_link_name("/tmp").unwrap();
    header.set_cksum();
    archive
        .append_data(&mut header, "IrisScope.app/link", std::io::empty())
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let destination = dir.path().join("stage");
    std::fs::create_dir(&destination).unwrap();
    assert!(install::unpack(&path, &destination).is_err());
    assert!(!destination.join("IrisScope.app/link").exists());
}

#[test]
fn verified_updates_are_strictly_newer_and_match_the_release_tag() {
    // Selection follows signature verification; test its platform/version rules separately.
    let target: InstallTarget =
        serde_json::from_str(r#"{"path":"/tmp/IrisScope.app","kind":"macos-bundle"}"#).unwrap();
    for current in ["0.4.2", "0.4.3", "0.5.0"] {
        assert!(
            select_verified_manifest(manifest(), vec![], vec![], current, "v0.4.2", &target)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        select_verified_manifest(manifest(), vec![], vec![], "0.4.1", "v0.4.2", &target)
            .unwrap()
            .is_some()
    );
    assert!(
        select_verified_manifest(manifest(), vec![], vec![], "0.4.1", "v9.0.0", &target).is_err()
    );
    let mut missing = manifest();
    missing.assets[0].architecture = "other".into();
    assert!(select_verified_manifest(missing, vec![], vec![], "0.4.1", "v0.4.2", &target).is_err());
}

#[test]
fn cancellation_does_not_wait_for_network_timeout() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        transport::fetch(
            "https://api.github.com/",
            &dir.path().join("data"),
            1024,
            30,
            &AtomicBool::new(true),
            &|_| {}
        )
        .is_err()
    );
}
