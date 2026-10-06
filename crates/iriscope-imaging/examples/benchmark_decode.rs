//! Synthetic DE400-size JPEG decode timings, including allocation.
use iriscope_imaging::{decode_mjpeg_to_rgb8, encode_rgb8_jpeg};
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280_u32, 1024_u32);
    let mut rgb = vec![0_u8; usize::try_from(width)? * usize::try_from(height)? * 3];
    let mut noise = 0x1234_5678_u32;
    for pixel in rgb.chunks_exact_mut(3) {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        pixel.copy_from_slice(&noise.to_le_bytes()[..3]);
    }
    let jpeg = encode_rgb8_jpeg(&rgb, width, height, 85)?;
    let mut times = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        let decoded = std::hint::black_box(decode_mjpeg_to_rgb8(std::hint::black_box(&jpeg))?);
        if decoded.0 != width || decoded.1 != height {
            return Err("Decode validation failed".into());
        }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "resolution={width}x{height} jpeg_bytes={} runs={} decode_median_ms={:.2} decode_p95_ms={:.2}",
        jpeg.len(),
        times.len(),
        times[50],
        times[95]
    );
    Ok(())
}
