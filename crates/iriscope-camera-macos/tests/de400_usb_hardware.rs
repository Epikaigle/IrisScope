#![cfg(target_os = "macos")]
//! Opt-in integration test against a manually authorized, connected DE400 helper.
use iriscope_core::{
    camera::{CameraDevice, CameraEvent, StreamConfiguration},
    capabilities::{CameraControlKind, CameraControlValue},
};
use std::time::Duration;

fn frames(device: &mut dyn CameraDevice) {
    for _ in 0..3 {
        loop {
            let event = device
                .next_event(Duration::from_secs(5))
                .expect("USB event");
            if let CameraEvent::Frame(frame) = event {
                assert_eq!(frame.resolution.width, 1280);
                assert_eq!(frame.resolution.height, 1024);
                assert_eq!(frame.data.len(), 1280 * 1024 * 2);
                break;
            }
        }
    }
}

#[test]
#[ignore = "requires authorized DE400 helper and IRISCOPE_DE400_SOCKET"]
fn video_controls_restore_and_reopen_without_reauthenticating() {
    assert!(std::env::var_os("IRISCOPE_DE400_SOCKET").is_some());
    let mut backend = iriscope_camera_macos::create_backend();
    for pass in 0..2 {
        let id = backend.enumerate_devices().unwrap()[0].id.clone();
        let mut device = backend.open(&id).expect("open USB helper");
        let mode = &device.capabilities().modes[0];
        let configuration = StreamConfiguration {
            pixel_format: mode.pixel_format.clone(),
            resolution: mode.resolution,
            frame_rate: mode.frame_rates[0],
        };
        let controls = device.capabilities().controls.clone();
        assert!(!controls.is_empty(), "camera controls must be exposed");
        device.start_stream(&configuration).unwrap();
        frames(device.as_mut());
        for control in controls {
            let original = device
                .control_value(&control.id)
                .expect("read camera control");
            println!(
                "CONTROL pass={pass} {} {:?} {:?}",
                control.name, control.kind, original
            );
            if control.read_only {
                continue;
            }
            let proposed = match (&control.kind, &original) {
                (
                    CameraControlKind::Integer { maximum, step, .. },
                    CameraControlValue::Integer(current),
                ) => CameraControlValue::Integer(if current + step <= *maximum {
                    current + step
                } else {
                    current - step
                }),
                (CameraControlKind::Boolean { .. }, CameraControlValue::Boolean(current)) => {
                    CameraControlValue::Boolean(!current)
                }
                (CameraControlKind::Menu { items, .. }, CameraControlValue::Menu(current)) => {
                    CameraControlValue::Menu(
                        items
                            .iter()
                            .find(|item| item.value != *current)
                            .unwrap()
                            .value,
                    )
                }
                _ => continue,
            };
            let result = device
                .set_control_value(&control.id, &proposed)
                .and_then(|()| device.control_value(&control.id));
            // Restore before asserting so a failed round trip preserves settings.
            device
                .set_control_value(&control.id, &original)
                .expect("restore camera control");
            assert_eq!(device.control_value(&control.id).unwrap(), original);
            assert_eq!(result.unwrap(), proposed);
            if let CameraControlKind::Integer { maximum, .. } = control.kind {
                assert!(
                    device
                        .set_control_value(&control.id, &CameraControlValue::Integer(maximum + 1))
                        .is_err()
                );
                assert_eq!(device.control_value(&control.id).unwrap(), original);
            }
        }
        frames(device.as_mut());
        let saved = device
            .capabilities()
            .controls
            .iter()
            .map(|control| (control.clone(), device.control_value(&control.id).unwrap()))
            .collect::<Vec<_>>();
        let reset_result = device.reset_controls();
        let defaults = saved
            .iter()
            .map(|(control, _)| device.control_value(&control.id))
            .collect::<Vec<_>>();
        for (control, original) in &saved {
            if !control.read_only {
                device
                    .set_control_value(&control.id, original)
                    .expect("restore settings after reset test");
            }
        }
        reset_result.expect("reset camera controls");
        for ((control, _), value) in saved.iter().zip(defaults) {
            let expected = match control.kind {
                CameraControlKind::Integer { default, .. } => CameraControlValue::Integer(default),
                CameraControlKind::Boolean { default } => CameraControlValue::Boolean(default),
                CameraControlKind::Menu { default, .. } => CameraControlValue::Menu(default),
                _ => continue,
            };
            assert_eq!(value.unwrap(), expected);
        }
        device.stop_stream().unwrap();
        assert!(device.active_configuration().is_none());
        drop(device);
    }
}
