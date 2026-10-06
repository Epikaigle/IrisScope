use crate::app_helpers::{decode_camera_frame_to_rgb8, ranked_stream_configurations};
use crate::platform_camera;
use crate::playback::is_de400;
use iriscope_core::camera::CameraEvent;
use std::time::{Duration, Instant};

#[allow(clippy::too_many_lines)]
pub fn run_diagnose() {
    let mut backend = platform_camera::create_backend();
    println!("IrisScope Camera Diagnostic ({})", backend.kind());

    let devices = match backend.enumerate_devices() {
        Ok(devs) => devs,
        Err(err) => {
            eprintln!("Failed to enumerate cameras: {err}");
            std::process::exit(1);
        }
    };

    if devices.is_empty() {
        eprintln!("No video capture device detected.");
        std::process::exit(1);
    }

    for (index, device) in devices.iter().enumerate() {
        println!("\nDevice #{index}: {}", device.display_name);
        println!("  ID: {}", device.id);
        if let Some(usb) = &device.usb {
            println!(
                "  USB: VID {:04x}, PID {:04x}",
                usb.vendor_id, usb.product_id
            );
            if let Some(serial) = &usb.serial_number {
                println!("  Serial: {serial}");
            }
            if let Some(rev) = &usb.hardware_revision {
                println!("  Revision: {rev}");
            }
        }
    }

    let Some(target) = devices.iter().find(|device| is_de400(device)) else {
        eprintln!("Firefly DE400 not detected.");
        std::process::exit(1);
    };

    println!("\nOpening device: {} [{}]", target.display_name, target.id);
    println!("{}", crate::app_helpers::hardware_button_status());
    let mut dev = match backend.open(&target.id) {
        Ok(dev) => dev,
        Err(err) => {
            eprintln!("Failed to open camera: {err}");
            std::process::exit(1);
        }
    };

    let caps = dev.capabilities();
    println!("\nModes available:");
    for mode in &caps.modes {
        let fps_list: Vec<String> = mode
            .frame_rates
            .iter()
            .map(|f| format!("{:.2} fps", f.frames_per_second()))
            .collect();
        println!(
            "  Format: {}, Resolution: {}, Rates: [{}]",
            mode.pixel_format,
            mode.resolution,
            fps_list.join(", ")
        );
    }

    println!("\nControls available:");
    for ctrl in &caps.controls {
        println!(
            "  Control: {} ({})",
            ctrl.name,
            if ctrl.read_only {
                "Read-only"
            } else {
                "Read-Write"
            }
        );
    }

    let candidates = ranked_stream_configurations(caps);

    if candidates.is_empty() {
        eprintln!("No usable mode found.");
        std::process::exit(1);
    }

    let mut active_configuration = None;
    println!("\nTrying camera modes in preferred order...");
    for candidate in candidates {
        print!(
            "  {} at {} with {:.2} fps... ",
            candidate.pixel_format,
            candidate.resolution,
            candidate.frame_rate.frames_per_second()
        );

        match dev.start_stream(&candidate) {
            Ok(()) => {
                println!("OK");
                active_configuration = Some(dev.active_configuration().unwrap_or(candidate));
                break;
            }
            Err(error) => {
                println!("failed ({error})");
            }
        }
    }

    let Some(config) = active_configuration else {
        eprintln!("No advertised camera mode could be started.");
        std::process::exit(1);
    };

    println!(
        "Stream started successfully with {} at {} / {:.2} fps. Capturing 3 test frames...",
        config.pixel_format,
        config.resolution,
        config.frame_rate.frames_per_second()
    );
    let start_time = Instant::now();
    let mut captured = 0;
    let mut decoded = 0;
    while decoded < 3 && start_time.elapsed() < Duration::from_secs(5) {
        match dev.next_event(Duration::from_millis(1500)) {
            Ok(CameraEvent::Frame(frame)) => {
                captured += 1;
                println!(
                    "  Frame #{}: {} bytes (seq {}, timestamp {:?})",
                    captured,
                    frame.data.len(),
                    frame.sequence_number,
                    frame.timestamp
                );
                if let Some((width, height, rgb)) = decode_camera_frame_to_rgb8(&frame) {
                    decoded += 1;
                    println!(
                        "  ✓ Successfully decoded {} to RGB8: {width}×{height} ({} bytes)",
                        frame.pixel_format,
                        rgb.len()
                    );
                } else {
                    eprintln!(
                        "  ✗ No RGB8 diagnostic decoder is available for {}",
                        frame.pixel_format
                    );
                }
            }
            Ok(other) => {
                println!("  Event: {other:?}");
            }
            Err(err) => {
                eprintln!("  Capture error: {err}");
                break;
            }
        }
    }

    if let Some(status) = dev.hardware_button_status() {
        println!("{status}");
    }
    let elapsed = start_time.elapsed();
    let _ = dev.stop_stream();
    println!(
        "Stopped stream. Captured {captured} frames, decoded {decoded} in {:.2}s",
        elapsed.as_secs_f64()
    );
    if decoded < 3 {
        std::process::exit(1);
    }
}
