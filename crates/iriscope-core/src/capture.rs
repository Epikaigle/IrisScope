//! Still image capture coordination.

use std::sync::Mutex;

use crate::camera::CapturedFrame;

/// A single-slot frame handoff where publishing a new frame replaces the old one.
///
/// The renderer can therefore fall behind without accumulating display latency.
#[derive(Debug, Default)]
pub struct LatestFrame {
    frame: Mutex<Option<CapturedFrame>>,
}

impl LatestFrame {
    /// Creates an empty latest-frame slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            frame: Mutex::new(None),
        }
    }

    /// Publishes a frame and returns the frame that was superseded, if any.
    pub fn publish(&self, frame: CapturedFrame) -> Option<CapturedFrame> {
        self.frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(frame)
    }

    /// Takes the newest frame, leaving the slot empty.
    pub fn take(&self) -> Option<CapturedFrame> {
        self.frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Copies the newest frame handle without consuming it.
    #[must_use]
    pub fn snapshot(&self) -> Option<CapturedFrame> {
        self.frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use crate::{
        camera::CapturedFrame,
        capabilities::{PixelFormat, Resolution},
    };

    use super::LatestFrame;

    fn frame(sequence_number: u64) -> CapturedFrame {
        CapturedFrame {
            sequence_number,
            timestamp: Duration::from_millis(sequence_number),
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(1280, 1024),
            data: Arc::from([
                0xff,
                0xd8,
                u8::try_from(sequence_number).expect("test sequence fits in a byte"),
                0xff,
                0xd9,
            ]),
        }
    }

    #[test]
    fn newest_frame_replaces_an_unconsumed_frame() {
        let latest = LatestFrame::new();

        assert!(latest.publish(frame(1)).is_none());
        assert_eq!(
            latest.publish(frame(2)).map(|frame| frame.sequence_number),
            Some(1)
        );
        assert_eq!(
            latest.snapshot().map(|frame| frame.sequence_number),
            Some(2)
        );
        assert_eq!(latest.take().map(|frame| frame.sequence_number), Some(2));
        assert!(latest.take().is_none());
    }
}
