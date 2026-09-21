//! Capture naming and storage policies.

use chrono::{Datelike, Local, Timelike};
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::session::CaptureSession;

/// Default filename template described by the `IrisScope` product specification.
pub const DEFAULT_FILENAME_TEMPLATE: &str = "{prenom}_{nom}_{oeil}_{date}_{heure}";

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
        let rendered = self
            .template
            .replace("{prenom}", &sanitize_component(session.first_name()))
            .replace("{nom}", &sanitize_component(session.last_name()))
            .replace("{oeil}", session.eye().filename_label())
            .replace("{date}", &timestamp.date())
            .replace("{heure}", &timestamp.time());
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
    if Path::new(file_name).file_name() != Some(OsStr::new(file_name)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture name must not contain a directory",
        ));
    }

    fs::create_dir_all(directory)?;
    let requested_path = Path::new(file_name);
    let stem = requested_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("Iris");
    let extension = requested_path.extension().and_then(OsStr::to_str);

    for collision_index in 1_u32.. {
        let candidate_name = if collision_index == 1 {
            file_name.to_owned()
        } else if let Some(extension) = extension {
            format!("{stem}_{collision_index}.{extension}")
        } else {
            format!("{stem}_{collision_index}")
        };
        let candidate = directory.join(candidate_name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate);

        match file {
            Ok(mut file) => {
                if let Err(error) = file.write_all(data) {
                    drop(file);
                    let _ = fs::remove_file(&candidate);
                    return Err(error);
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    unreachable!("the collision counter covers every u32 filename suffix")
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
    use std::{fs, time::SystemTime};

    use crate::session::{CaptureSession, Eye};

    use super::{
        CaptureNamingPolicy, CaptureTimestamp, filename_template_preserves_identity,
        save_new_capture,
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
}
