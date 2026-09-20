//! Windows camera backend built around Media Foundation.

/// Human-readable name of the native Windows camera API.
pub const BACKEND_NAME: &str = "Media Foundation";

#[cfg(target_os = "windows")]
mod platform;

#[cfg(target_os = "windows")]
pub use platform::{WindowsMediaFoundationBackend, create_backend};

#[cfg(not(target_os = "windows"))]
mod unavailable {
    use std::time::Duration;

    use iriscope_core::camera::{
        CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
        CameraDeviceId, CameraError, CameraErrorKind, CameraResult,
    };

    /// Placeholder available when this crate is compiled on another operating system.
    #[derive(Debug, Default)]
    pub struct WindowsMediaFoundationBackend;

    impl CameraBackend for WindowsMediaFoundationBackend {
        fn kind(&self) -> CameraBackendKind {
            CameraBackendKind::MediaFoundation
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

    /// Creates a backend value that reports that Media Foundation is unavailable.
    #[must_use]
    pub fn create_backend() -> Box<dyn CameraBackend> {
        Box::<WindowsMediaFoundationBackend>::default()
    }

    fn unavailable() -> CameraError {
        CameraError::new(
            CameraErrorKind::BackendUnavailable,
            "Media Foundation is only available on Windows",
        )
    }
}

#[cfg(not(target_os = "windows"))]
pub use unavailable::{WindowsMediaFoundationBackend, create_backend};
