//! A one-hour, 8 FPS synthetic AVI: indexing and verified random seeks, without a camera.
use iriscope_core::{
    capabilities::FrameRate,
    disk_space::ensure_available_space,
    video::{AviMjpegReader, AviMjpegWriter},
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
    let jpeg = fs::read(
        std::env::args()
            .nth(1)
            .ok_or("Pass a synthetic 1280x1024 JPEG path")?,
    )?;
    if jpeg.len() > 128 * 1024 || !jpeg.starts_with(&[0xff, 0xd8]) {
        return Err("Use a JPEG below 128 KiB, to stay within the classic AVI limit".into());
    }
    let directory = Temporary(std::env::temp_dir().join(format!(
        "iriscope-video-benchmark-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    )));
    fs::create_dir(&directory.0)?;
    // A valid JPEG comment identifies each frame, allowing seeks to be checked.
    let mut frame = vec![0xff, 0xd8, 0xff, 0xfe, 0, 6, 0, 0, 0, 0];
    frame.extend_from_slice(&jpeg[2..]);
    ensure_available_space(&directory.0, u64::try_from(frame.len() + 32)? * 28_800)?;
    let (mut writer, path) = AviMjpegWriter::create_unique(
        &directory.0,
        "one-hour.avi",
        1280,
        1024,
        FrameRate::new(8, 1).unwrap(),
    )?;
    for index in 0..28_800_u32 {
        frame[6..10].copy_from_slice(&index.to_le_bytes());
        writer.write_frame(&frame)?;
    }
    writer.finish()?;
    drop(writer);
    let start = Instant::now();
    let mut reader = AviMjpegReader::open(&path)?;
    let indexing = start.elapsed();
    if reader.frame_count() != 28_800 {
        return Err("Frame-count validation failed".into());
    }
    let mut times = Vec::new();
    for seek in 0..100_usize {
        let index = (seek * 7919) % 28_800;
        let start = Instant::now();
        let bytes = std::hint::black_box(reader.read_frame(index)?);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        if bytes[6..10] != u32::try_from(index)?.to_le_bytes() {
            return Err("Seek returned the wrong frame".into());
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "duration_seconds=3600 frames={} avi_bytes={} indexing_ms={:.2} seek_median_ms={:.2} seek_p95_ms={:.2}",
        reader.frame_count(),
        fs::metadata(&path)?.len(),
        indexing.as_secs_f64() * 1000.0,
        times[50],
        times[95]
    );
    Ok(())
}
