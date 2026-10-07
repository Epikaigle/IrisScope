//! Opt-in regression check against a connected DE400; no user captures or settings.
#![cfg(target_os = "macos")]

use iriscope_core::{
    camera::{CameraErrorKind, CameraEvent, StreamConfiguration, validate_camera_frame_bytes},
    capabilities::{FrameRate, PixelFormat, Resolution},
};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires a connected DE400 and macOS camera permission"]
fn connected_de400_preserves_resolution_across_stream_restarts() {
    let mut backend = iriscope_camera_macos::create_backend();
    let descriptor = backend
        .enumerate_devices()
        .expect("enumerate cameras")
        .into_iter()
        .find(|camera| {
            camera
                .usb
                .as_ref()
                .is_some_and(|usb| usb.vendor_id == 0x21cd && usb.product_id == 0x603b)
        })
        .expect("connect the DE400 before running this test");
    let mut device = backend.open(&descriptor.id).expect("open DE400");
    let unsupported = StreamConfiguration {
        pixel_format: PixelFormat::Yuyv,
        resolution: Resolution::new(800, 600),
        frame_rate: FrameRate::new(15, 1).unwrap(),
    };
    let mode = device
        .capabilities()
        .find_mode(&unsupported.pixel_format, unsupported.resolution)
        .expect("800x600 mode");
    if !mode.frame_rates.contains(&unsupported.frame_rate) {
        assert_eq!(
            device
                .start_stream(&unsupported)
                .expect_err(
                    "a rounded rate outside the native range must be rejected without aborting"
                )
                .kind(),
            CameraErrorKind::InvalidConfiguration
        );
    }
    for (width, height) in [(1280, 1024), (800, 600), (640, 480), (1280, 1024)] {
        let mode = device
            .capabilities()
            .find_mode(&PixelFormat::Yuyv, Resolution::new(width, height))
            .expect("advertised mode");
        let configuration = StreamConfiguration {
            pixel_format: PixelFormat::Yuyv,
            resolution: Resolution::new(width, height),
            frame_rate: *mode
                .frame_rates
                .first()
                .expect("advertised exact frame rate"),
        };
        device
            .start_stream(&configuration)
            .expect("start selected mode");
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut received = 0;
        while received < 3 && Instant::now() < deadline {
            match device.next_event(Duration::from_millis(500)) {
                Ok(CameraEvent::Frame(frame)) => {
                    assert_eq!(
                        frame.resolution, configuration.resolution,
                        "output must not downscale the selected camera mode"
                    );
                    validate_camera_frame_bytes(
                        &frame.pixel_format,
                        frame.resolution,
                        frame.data.len(),
                    )
                    .expect("valid frame buffer");
                    received += 1;
                }
                Ok(CameraEvent::HardwareButtonPressed) => {}
                Err(error) if error.kind() == CameraErrorKind::TimedOut => {}
                other => panic!("unexpected camera event: {other:?}"),
            }
        }
        assert_eq!(received, 3, "three frames must arrive before the deadline");
        println!(
            "PASS {width}x{height} at requested {:.6} fps, three valid frames; {:?}",
            configuration.frame_rate.frames_per_second(),
            device.hardware_button_status()
        );
        device
            .stop_stream()
            .expect("stop the camera before restarting");
    }
}
