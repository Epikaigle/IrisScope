use std::time::Duration;

use av_foundation::{capture_device::AVCaptureDevice, media_format::AVMediaTypeVideo};
use iriscope_core::camera::{
    CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
    CameraDeviceId, CameraError, CameraErrorKind, CameraResult,
};

/// Native macOS camera backend using `AVFoundation`.
#[derive(Debug, Default)]
pub struct MacAvFoundationBackend;

impl MacAvFoundationBackend {
    /// Creates a backend. Device discovery happens during enumeration.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CameraBackend for MacAvFoundationBackend {
    fn kind(&self) -> CameraBackendKind {
        CameraBackendKind::AvFoundation
    }

    fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>> {
        // SAFETY: `AVMediaTypeVideo` is a process-lifetime `AVFoundation` constant.
        let media_type = unsafe { AVMediaTypeVideo };
        let devices = AVCaptureDevice::devices_with_media_type(media_type);
        let mut descriptors = devices
            .iter()
            .map(|device| CameraDescriptor {
                id: CameraDeviceId::new(device.unique_id().to_string()),
                display_name: device.localized_name().to_string(),
                backend: CameraBackendKind::AvFoundation,
                usb: None,
            })
            .collect::<Vec<_>>();

        descriptors.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        Ok(descriptors)
    }

    fn wait_for_device_event(
        &mut self,
        _timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>> {
        Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "AVFoundation hotplug monitoring is not implemented yet",
        ))
    }

    fn open(&mut self, _device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "AVFoundation streaming is not implemented yet",
        ))
    }
}

/// Creates the native macOS camera backend.
#[must_use]
pub fn create_backend() -> Box<dyn CameraBackend> {
    Box::new(MacAvFoundationBackend::new())
}
