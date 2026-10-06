//! Non-destructive, content-bound observations, included in the library backup.
use super::{CaptureFileVersion, LIBRARY_INDEX_WRITE_LOCK, index, locks};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

/// A manually placed visual annotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationKind {
    /// Ellipse drawn between two points.
    Circle,
    /// Arrow from the first point to the second.
    Arrow,
    /// Point marker.
    Point,
    /// Text positioned at the first point.
    Text,
}

/// Coordinates are fractions of the original, unrotated photograph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// Kind of annotation.
    pub kind: AnnotationKind,
    /// First horizontal coordinate, in [0, 1].
    pub x: f32,
    /// First vertical coordinate, in [0, 1].
    pub y: f32,
    /// Second horizontal coordinate, in [0, 1].
    pub end_x: f32,
    /// Second vertical coordinate, in [0, 1].
    pub end_y: f32,
    /// Optional label; required for text annotations.
    pub text: String,
}

/// Observations belonging to one photograph, never applied to its pixels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhotoReview {
    /// Operator-selected useful take; never an automatic quality assessment.
    #[serde(default)]
    pub retained: bool,
    /// Free-form consultation notes.
    pub notes: String,
    /// Manually drawn markers.
    pub annotations: Vec<Annotation>,
    /// UTC timestamp of the last successful save.
    #[serde(default)]
    pub updated_at: String,
}

impl PhotoReview {
    /// Rejects excessive text, invalid coordinates, and oversized models.
    /// # Errors
    /// Returns invalid input for an invalid observation.
    pub fn validate(&self) -> io::Result<()> {
        let valid = self.notes.len() <= 32_000
            && self.annotations.len() <= 256
            && self.annotations.iter().all(|a| {
                [a.x, a.y, a.end_x, a.end_y]
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    && a.text.len() <= 512
                    && (a.kind != AnnotationKind::Text || !a.text.trim().is_empty())
            });
        if valid {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Notes limitées à 32 000 octets, 256 annotations et 512 octets par libellé.",
            ))
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct StoredReview {
    sha256: [u8; 32],
    review: PhotoReview,
}

fn identity(
    path: &Path,
    expected: &CaptureFileVersion,
    cancel: &dyn Fn() -> bool,
) -> io::Result<[u8; 32]> {
    let current = super::capture_file_version_cancellable(path, cancel)?;
    // A content digest binds observations to the photo across backup/restore,
    // while native identity protects against replacement before this action.
    if expected.supports_fast_validation() {
        if !current.same_native_metadata(expected)
            || expected
                .content_digest()
                .is_some_and(|digest| current.content_digest() != Some(digest))
        {
            return Err(io::Error::other(
                "La photo a changé. Actualisez la bibliothèque.",
            ));
        }
    } else if current != *expected {
        return Err(io::Error::other(
            "La photo a changé. Actualisez la bibliothèque.",
        ));
    }
    current
        .content_digest()
        .ok_or_else(|| io::Error::other("Empreinte photo indisponible."))
}

fn file_name(path: &Path) -> io::Result<&str> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io::Error::other("Nom de photo invalide."))?;
    crate::storage::validate_capture_file_name(name)?;
    Ok(name)
}

/// Reads observations only if they still belong to the same photo contents.
/// # Errors
/// Returns an I/O or validation error, preserving existing observations.
pub fn load_photo_review(
    path: &Path,
    expected: &CaptureFileVersion,
    cancel: &dyn Fn() -> bool,
) -> io::Result<PhotoReview> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Dossier photo absent."))?;
    index::ensure_local_regular_capture(directory, path)?;
    let name = file_name(path)?;
    let digest = identity(path, expected, cancel)?;
    let _guard = locks::mutex(&LIBRARY_INDEX_WRITE_LOCK, cancel)?;
    let library = index::load_library_index(directory)?;
    let review = library
        .reviews
        .get(name)
        .filter(|s| s.sha256 == digest)
        .map_or_else(PhotoReview::default, |s| s.review.clone());
    review.validate()?;
    Ok(review)
}

/// Atomically saves observations in the existing private library index.
/// # Errors
/// Returns an I/O error without overwriting the original photograph.
pub fn save_photo_review(
    path: &Path,
    expected: &CaptureFileVersion,
    review: &PhotoReview,
    cancel: &dyn Fn() -> bool,
) -> io::Result<()> {
    review.validate()?;
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Dossier photo absent."))?;
    index::ensure_local_regular_capture(directory, path)?;
    let name = file_name(path)?;
    let _guard = locks::mutex(&LIBRARY_INDEX_WRITE_LOCK, cancel)?;
    let _lock = index::lock_library_index_cancellable(directory, cancel)?;
    let digest = identity(path, expected, cancel)?;
    let mut library = index::load_library_index_for_write(directory)?;
    let mut review = review.clone();
    review.updated_at = chrono::Utc::now().to_rfc3339();
    library.reviews.insert(
        name.to_owned(),
        StoredReview {
            sha256: digest,
            review,
        },
    );
    if cancel() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Enregistrement annulé.",
        ));
    }
    // Recheck after index recovery so an external replacement cannot attach the draft to a new photo.
    if identity(path, expected, cancel)? != digest {
        return Err(io::Error::other(
            "La photo a changé pendant l’enregistrement.",
        ));
    }
    index::save_library_index(directory, &library)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "iriscope-review-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn photo(&self) -> std::path::PathBuf {
            self.0.join("photo.jpg")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn review() -> PhotoReview {
        PhotoReview {
            retained: false,
            notes: "Observation datée · œil gauche\nÉclairage à vérifier".into(),
            annotations: vec![Annotation {
                kind: AnnotationKind::Circle,
                x: 0.3,
                y: 0.4,
                end_x: 0.45,
                end_y: 0.5,
                text: String::new(),
            }],
            updated_at: String::new(),
        }
    }
    #[test]
    fn observations_round_trip_without_changing_photo_and_survive_backup_restore() {
        let fixture = Fixture::new();
        fs::write(fixture.photo(), b"original photo bytes").unwrap();
        let version = super::super::capture_file_version(&fixture.photo()).unwrap();
        let before = fs::read(fixture.photo()).unwrap();
        save_photo_review(&fixture.photo(), &version, &review(), &|| false).unwrap();
        let loaded = load_photo_review(&fixture.photo(), &version, &|| false).unwrap();
        assert_eq!(loaded.notes, review().notes);
        assert_eq!(loaded.annotations, review().annotations);
        assert!(!loaded.updated_at.is_empty());
        assert_eq!(fs::read(fixture.photo()).unwrap(), before);
        let backup_parent = Fixture::new();
        let backup = super::super::backup_library(&fixture.0, &backup_parent.0, &|| false).unwrap();
        let restored = backup_parent.0.join("restored");
        fs::create_dir(&restored).unwrap();
        let restored =
            super::super::restore_library(&backup.directory, &restored, &|| false).unwrap();
        let path = restored.directory.join("photo.jpg");
        let version = super::super::capture_file_version(&path).unwrap();
        assert_eq!(
            load_photo_review(&path, &version, &|| false).unwrap(),
            loaded
        );
    }
    #[test]
    fn replaced_photo_does_not_inherit_observations_and_stale_save_is_rejected() {
        let fixture = Fixture::new();
        fs::write(fixture.photo(), b"original bytes").unwrap();
        let expected = super::super::capture_file_version(&fixture.photo()).unwrap();
        save_photo_review(&fixture.photo(), &expected, &review(), &|| false).unwrap();
        fs::write(fixture.photo(), b"different image bytes").unwrap();
        assert!(save_photo_review(&fixture.photo(), &expected, &review(), &|| false).is_err());
        let version = super::super::capture_file_version(&fixture.photo()).unwrap();
        assert_eq!(
            load_photo_review(&fixture.photo(), &version, &|| false).unwrap(),
            PhotoReview::default()
        );
    }
    #[test]
    fn invalid_or_cancelled_edits_preserve_the_previous_observations() {
        let fixture = Fixture::new();
        fs::write(fixture.photo(), b"photo bytes").unwrap();
        let version = super::super::capture_file_version(&fixture.photo()).unwrap();
        save_photo_review(&fixture.photo(), &version, &review(), &|| false).unwrap();
        let before = fs::read(fixture.0.join(".iriscope-index.json")).unwrap();
        let mut edited = review();
        edited.notes = "edited".into();
        assert!(save_photo_review(&fixture.photo(), &version, &edited, &|| true).is_err());
        edited.annotations[0].x = f32::NAN;
        assert!(save_photo_review(&fixture.photo(), &version, &edited, &|| false).is_err());
        assert_eq!(
            fs::read(fixture.0.join(".iriscope-index.json")).unwrap(),
            before
        );
    }
    #[test]
    fn text_annotations_and_model_limits_are_validated() {
        let mut review = review();
        review.annotations[0].kind = AnnotationKind::Text;
        assert!(review.validate().is_err());
        review.annotations[0].text = "Zone observée".into();
        assert!(review.validate().is_ok());
        review.notes = "a".repeat(32_001);
        assert!(review.validate().is_err());
        review.notes.clear();
        review.annotations[0].end_y = 1.1;
        assert!(review.validate().is_err());
    }
}
