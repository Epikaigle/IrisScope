//! Storage preflight independent of camera and UI backends.
use std::{io, path::Path};
/// Free-space reserve below which new recordings are refused.
pub const MIN_CAPTURE_RESERVE: u64 = 64 * 1024 * 1024;
/// Threshold at which the interface warns about remaining storage.
pub const LOW_SPACE_THRESHOLD: u64 = 1024 * 1024 * 1024;
/// Finds available space on the target volume, including for a not-yet-created directory.
/// # Errors
/// Returns filesystem errors rather than treating an unreadable volume as empty.
pub fn available_space(path: &Path) -> io::Result<u64> {
    let absolute = std::path::absolute(path)?;
    let mut existing = absolute.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Volume de stockage introuvable.")
        })?;
    }
    fs2::available_space(existing)
}
/// Checks a byte requirement plus the application's capture reserve.
/// # Errors
/// Returns a descriptive error if space cannot be checked or is insufficient.
pub fn ensure_available_space(path: &Path, bytes: u64) -> io::Result<()> {
    if available_space(path)? < bytes.saturating_add(MIN_CAPTURE_RESERVE) {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "Espace disque insuffisant. Libérez de l’espace ou changez le dossier des captures.",
        ));
    }
    Ok(())
}
