//! Identity-safe display titles and legacy filename parsing.

use unicode_normalization::UnicodeNormalization;

use crate::session::{CaptureSession, Eye};

use super::{CaptureKind, LibraryEntry, LibraryFilter, PresentedLibraryItem};

/// Formats indexed entries according to medical confidentiality rules.
///
/// If `active_session` has patient identity and matches the entry's patient,
/// the identity is displayed. All other records are strictly anonymized.
#[must_use]
pub fn present_library_items(
    entries: &[LibraryEntry],
    active_session: &CaptureSession,
    filter: LibraryFilter,
) -> Vec<PresentedLibraryItem> {
    entries
        .iter()
        .filter(|entry| match filter {
            LibraryFilter::All => true,
            LibraryFilter::PhotosOnly => entry.kind == CaptureKind::Photo,
            LibraryFilter::VideosOnly => entry.kind == CaptureKind::Video,
        })
        .map(|entry| {
            let matches_patient = matches_session_identity(entry, active_session);

            let eye_text = match entry.eye {
                Eye::Left => "Œil Gauche",
                Eye::Right => "Œil Droit",
                Eye::Unspecified => "Œil non renseigné",
            };

            let kind_text = match entry.kind {
                CaptureKind::Photo => "Photo",
                CaptureKind::Video => "Vidéo",
            };

            let display_title = if matches_patient {
                let name = format!(
                    "{} {}",
                    active_session.first_name(),
                    active_session.last_name()
                )
                .trim()
                .to_owned();
                format!("{name} ({eye_text})")
            } else {
                format!("{kind_text} {eye_text}")
            };

            let date_time = format!("{} {}", entry.date_str, entry.time_str);

            PresentedLibraryItem {
                file_path: entry.file_path.clone(),
                dossier_number: entry.patient_id.map(|id| format!("D-{id:06}")),
                kind: entry.kind,
                display_title,
                date_time,
                eye_label: eye_text.to_string(),
                is_current_patient: matches_patient,
            }
        })
        .collect()
}

fn matches_session_identity(entry: &LibraryEntry, session: &CaptureSession) -> bool {
    if let Some(id) = session.patient_id() {
        return entry.patient_id == Some(id);
    }
    // A typed name has no authority to reveal captures: an identical name may
    // refer to a different person. Legacy captures require manual assignment.
    false
}

pub(super) fn normalized_patient_name(name: &str) -> String {
    name.trim()
        .nfc()
        .flat_map(char::to_lowercase)
        .nfc()
        .collect()
}

pub(super) fn parse_filename(stem: &str) -> (Option<String>, Option<String>, Eye, String, String) {
    let parts: Vec<&str> = stem.split('_').collect();

    // Standard pattern: {prenom}_{nom}_{oeil}_{date}_{heure} (5 parts)
    // Anonymous pattern: Iris_{oeil}_{date}_{heure} (4 parts)
    if parts.len() == 5 {
        let first_name = parts[0];
        let last_name = parts[1];
        let eye = parse_eye(parts[2]);
        let date_str = parts[3].to_string();
        let time_str = parts[4].replace('-', ":");

        (
            Some(first_name.to_string()),
            Some(last_name.to_string()),
            eye,
            date_str,
            time_str,
        )
    } else if parts.len() == 4 && parts[0] == "Iris" {
        let eye = parse_eye(parts[1]);
        let date_str = parts[2].to_string();
        let time_str = parts[3].replace('-', ":");
        (None, None, eye, date_str, time_str)
    } else if parts.len() > 5 {
        // Underscores in names make the first/last-name boundary unknowable.
        // Retain eye and timestamp for browsing, but do not guess identity.
        let eye = parse_eye(parts[parts.len() - 3]);
        let date_str = parts[parts.len() - 2].to_string();
        let time_str = parts[parts.len() - 1].replace('-', ":");
        (None, None, eye, date_str, time_str)
    } else {
        (None, None, Eye::Unspecified, String::new(), String::new())
    }
}

fn parse_eye(label: &str) -> Eye {
    match label.to_lowercase().as_str() {
        "gauche" | "left" => Eye::Left,
        "droit" | "right" => Eye::Right,
        _ => Eye::Unspecified,
    }
}
