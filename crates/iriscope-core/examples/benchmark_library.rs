//! Reproducible metadata and verification workload; never reads the user's library.
use iriscope_core::{
    library::{
        CaptureKind, LibraryQuery, create_patient, resolve_library_candidates_cancellable,
        save_indexed_capture, try_scan_library_directory_metadata,
    },
    session::{CaptureSession, Eye},
    storage::CaptureTimestamp,
};
use std::{
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = std::env::args()
        .nth(1)
        .map_or(Ok(10_000), |arg| arg.parse::<usize>())?;
    if !(100..=100_000).contains(&count) {
        return Err("Choose 100–100000 captures".into());
    }
    let directory = Temporary(std::env::temp_dir().join(format!(
        "iriscope-benchmark-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    )));
    fs::create_dir(&directory.0)?;
    let patient = create_patient(&directory.0, "Synthetic", "Benchmark")?;
    let mut session = CaptureSession::new("Synthetic", "Benchmark", Eye::Left);
    session.set_patient_id(Some(patient.id));
    // Synthetic bytes exercise metadata and SHA validation; image decoding is measured separately.
    let media = vec![0x42_u8; 64 * 1024];
    save_indexed_capture(
        &directory.0,
        "seed.jpg",
        &media,
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
    )?;
    let index_path = directory.0.join(".iriscope-index.json");
    let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&index_path)?)?;
    let seed = index["entries"]["seed.jpg"].clone();
    for number in 1..count {
        let name = format!("capture-{number:06}.jpg");
        fs::write(directory.0.join(&name), &media)?;
        let mut metadata = seed.clone();
        metadata["date_str"] = format!("2026-09-{:02}", number % 30 + 1).into();
        index["entries"][name] = metadata;
    }
    fs::write(&index_path, serde_json::to_vec(&index)?)?;
    let start = Instant::now();
    let candidates = try_scan_library_directory_metadata(&directory.0)?;
    let scan = start.elapsed();
    let query = LibraryQuery::parse("D-000001", "2026-09-01", "2026-10-05", 1, 0)?;
    let start = Instant::now();
    let selected: Vec<_> = candidates
        .into_iter()
        .filter(|entry| query.matches(entry))
        .take(100)
        .collect();
    let filtered = start.elapsed();
    let start = Instant::now();
    let verified = resolve_library_candidates_cancellable(&directory.0, &selected, &|| false)?;
    let verification = start.elapsed();
    if verified.len() != 100
        || verified
            .iter()
            .any(|entry| entry.patient_id != Some(patient.id))
    {
        return Err("Workload validation failed".into());
    }
    println!(
        "captures={count} media_bytes=65536 metadata_scan_ms={:.2} select_100_ms={:.2} verify_100_ms={:.2}",
        scan.as_secs_f64() * 1000.0,
        filtered.as_secs_f64() * 1000.0,
        verification.as_secs_f64() * 1000.0
    );
    Ok(())
}
