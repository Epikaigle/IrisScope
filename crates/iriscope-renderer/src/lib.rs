//! Live camera rendering and viewport management for `IrisScope`.

use iriscope_core::camera::CapturedFrame;

/// Display transformations applied before presentation in the viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportTransform {
    /// Zoom level percentage (100, 125, 150, 200).
    pub zoom_percent: u32,
    /// Rotation in degrees (0, 90, 180, 270).
    pub rotation_degrees: u32,
    /// Horizontal mirroring.
    pub mirror_horizontal: bool,
    /// Vertical mirroring.
    pub mirror_vertical: bool,
    /// Whether display is currently frozen.
    pub is_frozen: bool,
}

impl Default for ViewportTransform {
    fn default() -> Self {
        Self {
            zoom_percent: 100,
            rotation_degrees: 0,
            mirror_horizontal: false,
            mirror_vertical: false,
            is_frozen: false,
        }
    }
}

/// A pipeline that coordinates the latest frame presentation.
#[derive(Debug, Default)]
pub struct RenderPipeline {
    transform: ViewportTransform,
    frozen_frame: Option<CapturedFrame>,
}

impl RenderPipeline {
    /// Creates a new render pipeline with default transforms.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the active viewport transformations.
    #[must_use]
    pub const fn transform(&self) -> ViewportTransform {
        self.transform
    }

    /// Updates viewport transformations.
    pub fn set_transform(&mut self, transform: ViewportTransform) {
        self.transform = transform;
    }

    /// Toggles the frozen state.
    pub fn toggle_freeze(&mut self, current_frame: Option<CapturedFrame>) -> bool {
        self.transform.is_frozen = !self.transform.is_frozen;
        if self.transform.is_frozen {
            self.frozen_frame = current_frame;
        } else {
            self.frozen_frame = None;
        }
        self.transform.is_frozen
    }

    /// Selects either the frozen frame or the incoming live frame.
    #[must_use]
    pub fn active_frame<'a>(
        &'a self,
        incoming: Option<&'a CapturedFrame>,
    ) -> Option<&'a CapturedFrame> {
        if self.transform.is_frozen {
            self.frozen_frame.as_ref().or(incoming)
        } else {
            incoming
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{PixelFormat, Resolution},
    };

    use super::RenderPipeline;

    fn sample_frame() -> CapturedFrame {
        CapturedFrame {
            sequence_number: 1,
            timestamp: Duration::ZERO,
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(1280, 1024),
            data: Arc::from([0xff, 0xd8, 0xff, 0xd9]),
        }
    }

    #[test]
    fn freezing_preserves_frame() {
        let mut pipeline = RenderPipeline::new();
        let frame1 = sample_frame();
        assert!(!pipeline.transform().is_frozen);

        pipeline.toggle_freeze(Some(frame1.clone()));
        assert!(pipeline.transform().is_frozen);

        let active = pipeline.active_frame(None);
        assert!(active.is_some());

        pipeline.toggle_freeze(None);
        assert!(!pipeline.transform().is_frozen);
        assert!(pipeline.active_frame(None).is_none());
    }
}
