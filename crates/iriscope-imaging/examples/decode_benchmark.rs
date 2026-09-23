use std::{env, fs, hint::black_box, process::ExitCode, time::Instant};

use iriscope_imaging::decode_mjpeg_to_rgb8;

fn main() -> ExitCode {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!("usage: decode_benchmark <frame.jpg> [iterations]");
        return ExitCode::FAILURE;
    };
    let iterations = env::args()
        .nth(2)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(100);
    if iterations == 0 {
        eprintln!("iterations must be greater than zero");
        return ExitCode::FAILURE;
    }
    let jpeg = match fs::read(&path) {
        Ok(jpeg) => jpeg,
        Err(error) => {
            eprintln!("failed to read {}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };

    let Ok((width, height, warmup)) = decode_mjpeg_to_rgb8(black_box(&jpeg)) else {
        eprintln!("failed to decode {}", path.to_string_lossy());
        return ExitCode::FAILURE;
    };
    black_box(warmup);

    let started = Instant::now();
    for _ in 0..iterations {
        let Ok((_, _, rgb)) = decode_mjpeg_to_rgb8(black_box(&jpeg)) else {
            eprintln!("decode failed during benchmark");
            return ExitCode::FAILURE;
        };
        black_box(rgb);
    }
    let elapsed = started.elapsed();
    let per_frame = elapsed / iterations;
    let frames_per_second = f64::from(iterations) / elapsed.as_secs_f64();

    println!(
        "{width}x{height}, {iterations} frames in {elapsed:.2?}: {per_frame:.2?}/frame ({frames_per_second:.1} fps)"
    );
    ExitCode::SUCCESS
}
