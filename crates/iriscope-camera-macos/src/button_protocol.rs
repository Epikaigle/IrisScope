//! DE400 interrupt status packets measured on the physical camera.

#[derive(Default)]
pub(crate) struct ButtonProtocol {
    held: bool,
}
impl ButtonProtocol {
    /// Only a release followed by a new press rearms an already held button.
    pub(crate) fn pressed(&mut self, packet: &[u8]) -> bool {
        match packet {
            [2, 1, 0, 1] => {
                let new_press = !self.held;
                self.held = true;
                new_press
            }
            [2, 1, 0, 0] => {
                self.held = false;
                false
            }
            _ => false,
        }
    }
}

/// `AVFoundation` UVC IDs include location ID, vendor ID and product ID.
pub(crate) fn de400_location(unique_id: &str) -> Option<u32> {
    let hex = unique_id
        .strip_prefix("0x")
        .or_else(|| unique_id.strip_prefix("0X"))
        .unwrap_or(unique_id);
    let id = u64::from_str_radix(hex, 16).ok()?;
    if (id >> 16) & 0xffff != 0x21cd || id & 0xffff != 0x603b {
        return None;
    }
    u32::try_from(id >> 32)
        .ok()
        .filter(|location| *location != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_the_selected_camera_location_instead_of_the_first_usb_camera() {
        assert_eq!(de400_location("0x1420000021cd603b"), Some(0x1420_0000));
        assert_eq!(de400_location("0x1430000021cd603b"), Some(0x1430_0000));
        for id in [
            "FaceTime HD Camera",
            "0x1420000021cd1234",
            "0x0000000021cd603b",
        ] {
            assert!(de400_location(id).is_none());
        }
    }
    #[test]
    fn a_hold_produces_one_capture_and_release_rearms() {
        let mut protocol = ButtonProtocol::default();
        assert!(protocol.pressed(&[2, 1, 0, 1]));
        for _ in 0..20 {
            assert!(!protocol.pressed(&[2, 1, 0, 1]));
        }
        assert!(!protocol.pressed(&[2, 1, 0, 0]));
        assert!(protocol.pressed(&[2, 1, 0, 1]));
    }
    #[test]
    fn unrelated_truncated_and_extended_packets_do_not_capture_or_rearm() {
        let mut protocol = ButtonProtocol::default();
        assert!(protocol.pressed(&[2, 1, 0, 1]));
        for packet in [
            &[1, 1, 0, 0][..],
            &[2, 1, 0][..],
            &[2, 1, 0, 0, 1][..],
            &[][..],
        ] {
            assert!(!protocol.pressed(packet));
        }
        assert!(!protocol.pressed(&[2, 1, 0, 1]));
    }
}
