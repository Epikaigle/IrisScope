//! macOS camera backend built around `AVFoundation`.

/// Human-readable name of the native macOS camera API.
pub const BACKEND_NAME: &str = "AVFoundation";

#[cfg(target_os = "macos")]
mod platform;

#[cfg(target_os = "macos")]
pub use platform::{MacAvFoundationBackend, create_backend};

#[cfg(not(target_os = "macos"))]
mod unavailable {
    use std::time::Duration;

    use iriscope_core::camera::{
        CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
        CameraDeviceId, CameraError, CameraErrorKind, CameraResult,
    };

    /// Placeholder available when this crate is compiled on another operating system.
    #[derive(Debug, Default)]
    pub struct MacAvFoundationBackend;

    impl CameraBackend for MacAvFoundationBackend {
        fn kind(&self) -> CameraBackendKind {
            CameraBackendKind::AvFoundation
        }

        fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>> {
            Err(unavailable())
        }

        fn wait_for_device_event(
            &mut self,
            _timeout: Duration,
        ) -> CameraResult<Option<CameraDeviceEvent>> {
            Err(unavailable())
        }

        fn open(&mut self, _device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
            Err(unavailable())
        }
    }

    /// Creates a backend value that reports that `AVFoundation` is unavailable.
    #[must_use]
    pub fn create_backend() -> Box<dyn CameraBackend> {
        Box::<MacAvFoundationBackend>::default()
    }

    fn unavailable() -> CameraError {
        CameraError::new(
            CameraErrorKind::BackendUnavailable,
            "AVFoundation is only available on macOS",
        )
    }
}

#[cfg(not(target_os = "macos"))]
pub use unavailable::{MacAvFoundationBackend, create_backend};
