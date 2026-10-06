//! Day-based consultation notes, saved with the existing library transaction.
use super::{LIBRARY_INDEX_WRITE_LOCK, PatientRecord, index, locks};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

/// General observations for one dossier and one capture day.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsultationNotes {
    /// Free-form notes distinct from annotations on individual photographs.
    pub notes: String,
    /// UTC timestamp of the last successful save.
    pub updated_at: String,
}
fn key(id: u64, day: &str) -> io::Result<String> {
    if id == 0 || day.len() != 10 || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Dossier ou date invalide.",
        ));
    }
    Ok(format!("{id}:{day}"))
}
/// Searches by explicit dossier number or name tokens, without selecting homonyms.
/// # Errors
/// Returns an error when the private index cannot be read.
pub fn search_dossiers(directory: &Path, query: &str) -> io::Result<Vec<PatientRecord>> {
    let query = super::normalized_patient_name(query);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let numeric = query
        .strip_prefix("d-")
        .unwrap_or(&query)
        .parse::<u64>()
        .ok();
    let library = index::load_library_index(directory)?;
    let mut rows: Vec<_> = library
        .patients
        .iter()
        .filter(|(id, p)| {
            numeric.map_or_else(
                || {
                    let name = super::normalized_patient_name(&format!(
                        "{} {}",
                        p.first_name, p.last_name
                    ));
                    query.split_whitespace().all(|token| name.contains(token))
                },
                |n| **id == n,
            )
        })
        .map(|(&id, p)| PatientRecord::from_stored(id, p))
        .collect();
    rows.sort_by_key(|p| p.id);
    rows.truncate(50);
    Ok(rows)
}
/// Reads the observations for an existing dossier and capture day.
/// # Errors
/// Rejects invalid identities and index read failures.
pub fn load_consultation_notes(
    directory: &Path,
    id: u64,
    day: &str,
) -> io::Result<ConsultationNotes> {
    let key = key(id, day)?;
    let library = index::load_library_index(directory)?;
    if !library.patients.contains_key(&id) {
        return Err(io::Error::other("Dossier introuvable."));
    }
    let notes = library.consultations.get(&key).cloned().unwrap_or_default();
    if notes.notes.len() > 32_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Notes enregistrées trop longues.",
        ));
    }
    Ok(notes)
}
/// Saves notes atomically without rewriting any capture.
/// # Errors
/// Rejects notes above 32,000 bytes, missing dossiers and cancelled writes.
pub fn save_consultation_notes(
    directory: &Path,
    id: u64,
    day: &str,
    notes: &str,
    cancel: &dyn Fn() -> bool,
) -> io::Result<()> {
    let key = key(id, day)?;
    if notes.len() > 32_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Notes limitées à 32 000 octets.",
        ));
    }
    let _guard = locks::mutex(&LIBRARY_INDEX_WRITE_LOCK, cancel)?;
    let _lock = index::lock_library_index_cancellable(directory, cancel)?;
    let mut library = index::load_library_index_for_write(directory)?;
    if !library.patients.contains_key(&id) {
        return Err(io::Error::other("Dossier introuvable."));
    }
    if cancel() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Enregistrement annulé.",
        ));
    }
    library.consultations.insert(
        key,
        ConsultationNotes {
            notes: notes.to_owned(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    index::save_library_index(directory, &library)
}
/// Returns the dates of saved general notes for an existing dossier.
/// # Errors
/// Returns an index read error.
pub fn consultation_dates(directory: &Path, id: u64) -> io::Result<Vec<String>> {
    let index = index::load_library_index(directory)?;
    let prefix = format!("{id}:");
    Ok(index
        .consultations
        .keys()
        .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consultation_notes_are_isolated_validated_and_recoverable() {
        let root = std::env::temp_dir().join(format!(
            "iriscope-consultation-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let a = super::super::create_patient(&root, "Anne", "Martin").unwrap();
        let b = super::super::create_patient(&root, "Anne", "Martin").unwrap();
        assert_eq!(search_dossiers(&root, "martin anne").unwrap().len(), 2);
        assert_eq!(
            search_dossiers(&root, &b.dossier_number).unwrap()[0].id,
            b.id
        );
        save_consultation_notes(&root, a.id, "2026-10-06", "Éclairage constant", &|| false)
            .unwrap();
        assert!(
            load_consultation_notes(&root, b.id, "2026-10-06")
                .unwrap()
                .notes
                .is_empty()
        );
        assert!(
            load_consultation_notes(&root, a.id, "2026-10-07")
                .unwrap()
                .notes
                .is_empty()
        );
        assert!(save_consultation_notes(&root, a.id, "2026-10-06", "perdu", &|| true).is_err());
        assert!(save_consultation_notes(&root, a.id, "2026-02-30", "", &|| false).is_err());
        assert!(save_consultation_notes(&root, 999, "2026-10-06", "", &|| false).is_err());
        assert!(
            save_consultation_notes(&root, a.id, "2026-10-06", &"a".repeat(32_001), &|| false)
                .is_err()
        );
        let backup_parent = root.with_extension("backups");
        std::fs::create_dir(&backup_parent).unwrap();
        let backup = super::super::backup_library(&root, &backup_parent, &|| false).unwrap();
        let restore_parent = root.with_extension("restore");
        std::fs::create_dir(&restore_parent).unwrap();
        let restored =
            super::super::restore_library(&backup.directory, &restore_parent, &|| false).unwrap();
        assert_eq!(
            load_consultation_notes(&restored.directory, a.id, "2026-10-06")
                .unwrap()
                .notes,
            "Éclairage constant"
        );
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(backup_parent).unwrap();
        std::fs::remove_dir_all(restore_parent).unwrap();
    }
}
