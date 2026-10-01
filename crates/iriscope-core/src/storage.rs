//! Capture naming and storage policies.

use chrono::{Datelike, Local, Timelike};
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

use crate::session::CaptureSession;

/// Default filename template described by the `IrisScope` product specification.
pub const DEFAULT_FILENAME_TEMPLATE: &str = "{prenom}_{nom}_{oeil}_{date}_{heure}";
/// Leaves room for collision suffixes on common 255-byte filesystems.
pub const MAX_CAPTURE_FILENAME_BYTES: usize = 220;
static NEXT_CAPTURE_WRITE_ID: AtomicU64 = AtomicU64::new(0);

/// Calendar and clock fields used to render a capture filename.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureTimestamp {
    /// Four-digit year.
    pub year: u16,
    /// Month from 1 to 12.
    pub month: u8,
    /// Day from 1 to 31.
    pub day: u8,
    /// Hour from 0 to 23.
    pub hour: u8,
    /// Minute from 0 to 59.
    pub minute: u8,
    /// Second from 0 to 59.
    pub second: u8,
}

impl CaptureTimestamp {
    /// Obtains the current local timestamp.
    #[must_use]
    pub fn now() -> Self {
        let now = Local::now();

        Self {
            year: u16::try_from(now.year()).unwrap_or(1970),
            month: u8::try_from(now.month()).unwrap_or(1),
            day: u8::try_from(now.day()).unwrap_or(1),
            hour: u8::try_from(now.hour()).unwrap_or_default(),
            minute: u8::try_from(now.minute()).unwrap_or_default(),
            second: u8::try_from(now.second()).unwrap_or_default(),
        }
    }

    fn date(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    fn time(self) -> String {
        format!("{:02}-{:02}-{:02}", self.hour, self.minute, self.second)
    }
}

/// Configurable policy used to name captures without changing their pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureNamingPolicy {
    template: String,
}

impl Default for CaptureNamingPolicy {
    fn default() -> Self {
        Self::new(DEFAULT_FILENAME_TEMPLATE)
    }
}

/// Reports whether a filename template preserves the required patient identity.
///
/// `IrisScope` requires every new capture filename to contain the patient's first name,
/// last name, and selected eye. Date and time remain optional.
#[must_use]
pub fn filename_template_preserves_identity(template: &str) -> bool {
    ["{prenom}", "{nom}", "{oeil}"]
        .iter()
        .all(|token| template.contains(token))
}

/// Reports whether every brace-delimited token is supported by the naming policy.
#[must_use]
pub fn filename_template_uses_supported_tokens(template: &str) -> bool {
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        if before.contains('}') {
            return false;
        }
        let Some((token, after_close)) = after_open.split_once('}') else {
            return false;
        };
        if !matches!(token, "prenom" | "nom" | "oeil" | "date" | "heure") {
            return false;
        }
        rest = after_close;
    }
    !rest.contains('}')
}

impl CaptureNamingPolicy {
    /// Creates a policy from a user-visible filename template.
    #[must_use]
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }

    /// Returns the configured template.
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Renders a cross-platform filename for one capture.
    ///
    /// Supported tokens are `{prenom}`, `{nom}`, `{oeil}`, `{date}` and `{heure}`.
    #[must_use]
    pub fn filename(
        &self,
        session: &CaptureSession,
        timestamp: CaptureTimestamp,
        extension: &str,
    ) -> String {
        // Substitute only tokens in the template. A patient's literal name may
        // contain text such as "{nom}" and must not be interpreted a second time.
        let mut rendered = String::new();
        let mut remainder = self.template.as_str();
        while let Some(start) = remainder.find('{') {
            rendered.push_str(&remainder[..start]);
            let after_open = &remainder[start + 1..];
            if let Some(end) = after_open.find('}') {
                let token = &after_open[..end];
                match token {
                    "prenom" => rendered.push_str(&sanitize_component(session.first_name())),
                    "nom" => rendered.push_str(&sanitize_component(session.last_name())),
                    "oeil" => rendered.push_str(session.eye().filename_label()),
                    "date" => rendered.push_str(&timestamp.date()),
                    "heure" => rendered.push_str(&timestamp.time()),
                    _ => rendered.push_str(&remainder[start..start + end + 2]),
                }
                remainder = &after_open[end + 1..];
            } else {
                rendered.push_str(&remainder[start..]);
                remainder = "";
                break;
            }
        }
        rendered.push_str(remainder);
        let mut stem = sanitize_component(&rendered);
        if stem.is_empty() {
            stem.push_str("Iris");
        } else if !session.has_identity() && !stem.starts_with("Iris") {
            stem = format!("Iris_{stem}");
        }

        let extension = sanitize_extension(extension);
        if extension.is_empty() {
            stem
        } else {
            format!("{stem}.{extension}")
        }
    }
}

/// Saves bytes to a new capture file without overwriting an existing capture.
///
/// A collision adds `_2`, `_3`, and so on before the extension. File creation is
/// atomic, so concurrent writers cannot select the same destination.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be created or the capture cannot
/// be written. `file_name` must contain a single filename rather than a path.
pub fn save_new_capture(directory: &Path, file_name: &str, data: &[u8]) -> io::Result<PathBuf> {
    validate_capture_file_name(file_name)?;

    create_private_directory(directory)?;
    let temporary_path = directory.join(format!(
        ".iriscope-capture-{}-{}.tmp",
        std::process::id(),
        NEXT_CAPTURE_WRITE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut temporary = options.open(&temporary_path)?;
        temporary.write_all(data)?;
        temporary.sync_all()?;
        drop(temporary);

        publish_completed_capture(&temporary_path, directory, file_name)
    })();
    let _ = fs::remove_file(temporary_path);
    result
}

/// Publishes an already completed capture under a collision-safe filename.
/// The source is preserved on failure and removed after successful publication.
/// Publication requires an atomic hard link. Filesystems without hard-link
/// support return an error while retaining the completed source.
///
/// # Errors
///
/// Returns an I/O error if the name is invalid or publication fails.
pub fn publish_completed_capture(
    source: &Path,
    directory: &Path,
    file_name: &str,
) -> io::Result<PathBuf> {
    validate_capture_file_name(file_name)?;
    create_private_directory(directory)?;
    let requested_path = Path::new(file_name);
    let stem = requested_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("Iris");
    let extension = requested_path.extension().and_then(OsStr::to_str);
    for collision_index in 1_u32.. {
        let candidate_name = collision_name(stem, extension, file_name, collision_index)?;
        validate_capture_file_name(&candidate_name)?;
        let candidate = directory.join(candidate_name);
        match fs::hard_link(source, &candidate) {
            Ok(()) => {
                // The file already exists at this point. A directory sync failure
                // cannot be reported as a failed capture without misleading callers.
                let _ = sync_directory(directory);
                let _ = fs::remove_file(source);
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "capture filename suffix exhausted",
    ))
}

pub(crate) fn collision_name(
    stem: &str,
    extension: Option<&str>,
    original: &str,
    index: u32,
) -> io::Result<String> {
    if index == 1 {
        return Ok(original.to_owned());
    }
    let suffix = format!("_{index}");
    let extension = extension.map_or(String::new(), |extension| format!(".{extension}"));
    let stem_budget = MAX_CAPTURE_FILENAME_BYTES
        .checked_sub(suffix.len() + extension.len())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "capture extension leaves no room for a collision suffix",
            )
        })?;
    let safe_end = stem
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end <= stem_budget)
        .last()
        .unwrap_or(0);
    Ok(format!("{}{}{}", &stem[..safe_end], suffix, extension))
}

/// Checks whether a generated capture name fits a portable file component.
///
/// # Errors
///
/// Returns `InvalidInput` for a path or a name too long for routine filesystems.
pub fn validate_capture_file_name(file_name: &str) -> io::Result<()> {
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || Path::new(file_name).file_name() != Some(OsStr::new(file_name))
        || file_name.ends_with('.')
        || file_name.ends_with(' ')
        || file_name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
        || Path::new(file_name)
            .file_stem()
            .and_then(OsStr::to_str)
            .is_some_and(is_windows_reserved_name)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture name must be a portable single filename",
        ));
    }
    if file_name.len() > MAX_CAPTURE_FILENAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("capture filename exceeds {MAX_CAPTURE_FILENAME_BYTES} bytes"),
        ));
    }
    Ok(())
}

pub(crate) fn create_private_directory(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(directory)
}

#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn sync_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(directory)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

fn sanitize_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut previous_was_separator = false;

    for character in value.trim().chars() {
        let separator = character.is_whitespace()
            || character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            );
        if separator {
            if !output.is_empty() && !previous_was_separator {
                output.push('_');
            }
            previous_was_separator = true;
        } else {
            output.push(character);
            previous_was_separator = character == '_';
        }
    }

    let output = output.trim_matches(['_', '.', ' ']).to_owned();
    if is_windows_reserved_name(&output) {
        format!("_{output}")
    } else {
        output
    }
}

fn sanitize_extension(extension: &str) -> String {
    extension
        .trim()
        .trim_start_matches('.')
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_windows_reserved_name(value: &str) -> bool {
    let uppercase = value.to_ascii_uppercase();
    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase
            .strip_prefix("COM")
            .or_else(|| uppercase.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes().first(), Some(b'1'..=b'9'))
            })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
        time::SystemTime,
    };

    use crate::session::{CaptureSession, Eye};

    use super::{
        CaptureNamingPolicy, CaptureTimestamp, filename_template_preserves_identity,
        filename_template_uses_supported_tokens, save_new_capture, validate_capture_file_name,
    };

    fn timestamp() -> CaptureTimestamp {
        CaptureTimestamp {
            year: 2026,
            month: 9,
            day: 20,
            hour: 14,
            minute: 32,
            second: 18,
        }
    }

    #[test]
    fn default_template_matches_the_product_filename() {
        let session = CaptureSession::new("Jean", "Dupont", Eye::Right);

        assert_eq!(
            CaptureNamingPolicy::default().filename(&session, timestamp(), "jpg"),
            "Jean_Dupont_Droit_2026-09-20_14-32-18.jpg"
        );
    }

    #[test]
    fn patient_text_is_not_reinterpreted_as_a_template_token() {
        let session = CaptureSession::new("{nom}", "Martin", Eye::Right);
        let filename = CaptureNamingPolicy::default().filename(&session, timestamp(), "jpg");
        assert!(filename.starts_with("{nom}_Martin_Droit"));
    }

    #[test]
    fn collision_suffix_fits_when_requested_filename_uses_full_budget() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-long-collision-{unique}"));
        let name = format!("{}.jpg", "a".repeat(super::MAX_CAPTURE_FILENAME_BYTES - 4));
        let first = save_new_capture(&directory, &name, b"one").expect("first");
        let second = save_new_capture(&directory, &name, b"two").expect("second");
        assert!(
            second
                .file_name()
                .expect("name")
                .to_string_lossy()
                .ends_with("_2.jpg")
        );
        assert!(second.file_name().expect("name").len() <= super::MAX_CAPTURE_FILENAME_BYTES);
        assert_eq!(fs::read(first).expect("read first"), b"one");
        assert_eq!(fs::read(second).expect("read second"), b"two");
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn collision_with_maximum_length_extension_returns_an_error_instead_of_panicking() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-long-extension-{unique}"));
        let name = format!("a.{}", "x".repeat(super::MAX_CAPTURE_FILENAME_BYTES - 2));
        save_new_capture(&directory, &name, b"one").expect("first");
        assert_eq!(
            save_new_capture(&directory, &name, b"two")
                .expect_err("no suffix fits")
                .kind(),
            std::io::ErrorKind::InvalidInput,
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn anonymous_and_invalid_components_are_safe_on_all_platforms() {
        let anonymous = CaptureSession::new("", "", Eye::Left);
        let invalid = CaptureSession::new("CON", "Du/pont:*", Eye::Right);
        let policy = CaptureNamingPolicy::default();

        assert_eq!(
            policy.filename(&anonymous, timestamp(), ".JPG"),
            "Iris_Gauche_2026-09-20_14-32-18.jpg"
        );
        assert_eq!(
            policy.filename(&invalid, timestamp(), "j?p*g"),
            "CON_Du_pont_Droit_2026-09-20_14-32-18.jpg"
        );
    }

    #[test]
    fn saving_never_overwrites_an_existing_capture() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("the test clock should be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-storage-{unique}"));

        let first = save_new_capture(&directory, "Iris.jpg", b"first").expect("first save");
        let second = save_new_capture(&directory, "Iris.jpg", b"second").expect("second save");

        assert_eq!(first.file_name(), Some("Iris.jpg".as_ref()));
        assert_eq!(second.file_name(), Some("Iris_2.jpg".as_ref()));
        assert_eq!(fs::read(first).expect("read first"), b"first");
        assert_eq!(fs::read(second).expect("read second"), b"second");
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn simultaneous_saves_publish_only_complete_distinct_files() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-simultaneous-{unique}"));
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|number| {
                let directory = directory.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let bytes = vec![u8::try_from(number).expect("small number"); 100_000];
                    barrier.wait();
                    let path = save_new_capture(&directory, "Iris.jpg", &bytes).expect("save");
                    (path, bytes)
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .collect::<Vec<_>>();
        assert_ne!(results[0].0, results[1].0);
        for (path, expected) in results {
            assert_eq!(fs::read(path).expect("read complete capture"), expected);
        }
        fs::remove_dir_all(directory).expect("remove test directory");
    }
    #[test]
    fn filename_template_requires_patient_identity_and_eye() {
        assert!(filename_template_preserves_identity(
            "{prenom}_{nom}_{oeil}_{date}_{heure}"
        ));
        assert!(filename_template_preserves_identity(
            "{date}-{nom}-{prenom}-{oeil}"
        ));
        assert!(!filename_template_preserves_identity(
            "{date}_{heure}_{oeil}"
        ));
        assert!(!filename_template_preserves_identity(
            "{prenom}_{nom}_{date}"
        ));
    }

    #[test]
    fn filename_template_rejects_unknown_and_unbalanced_tokens() {
        assert!(filename_template_uses_supported_tokens(
            "{prenom}_{nom}_{oeil}_{date}_{heure}"
        ));
        assert!(!filename_template_uses_supported_tokens(
            "{prenom}_{nom}_{oeil}_{inconnu}"
        ));
        assert!(!filename_template_uses_supported_tokens(
            "{prenom}_{nom}_{oeil"
        ));
        assert!(!filename_template_uses_supported_tokens(
            "{prenom}_{nom}_{oeil}}"
        ));
    }

    #[test]
    fn rejects_oversize_capture_name_before_creating_directory() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-long-name-{unique}"));
        let name = format!("{}.jpg", "é".repeat(110));
        assert_eq!(
            validate_capture_file_name(&name)
                .expect_err("too long")
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert!(save_new_capture(&directory, &name, b"data").is_err());
        assert!(!directory.exists());
    }

    #[test]
    fn rejects_nonportable_capture_names() {
        for name in [
            "CON.jpg",
            "patient\\capture.jpg",
            "bad:name.jpg",
            "capture. ",
        ] {
            assert_eq!(
                validate_capture_file_name(name)
                    .expect_err("invalid name")
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
        validate_capture_file_name("Émilie_Dupont_Gauche.jpg").expect("valid Unicode capture name");
    }

    #[cfg(unix)]
    #[test]
    fn new_capture_and_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-private-{unique}"));
        let file = save_new_capture(&directory, "Iris.jpg", b"image").expect("save");
        assert_eq!(
            fs::metadata(&directory)
                .expect("directory")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&file).expect("file").permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(directory).expect("remove directory");
    }
}
