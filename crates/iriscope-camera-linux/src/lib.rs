//! Linux camera backend built around `V4L2`.

/// Human-readable name of the native Linux camera API.
pub const BACKEND_NAME: &str = "V4L2";

#[cfg(target_os = "linux")]
mod platform;

#[cfg(target_os = "linux")]
pub use platform::{LinuxV4l2Backend, create_backend};

#[cfg(not(target_os = "linux"))]
mod unavailable {
    use std::time::Duration;

    use iriscope_core::camera::{
        CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
        CameraDeviceId, CameraError, CameraErrorKind, CameraResult,
    };

    /// Placeholder available when this crate is compiled on another operating system.
    #[derive(Debug, Default)]
    pub struct LinuxV4l2Backend;

    impl CameraBackend for LinuxV4l2Backend {
        fn kind(&self) -> CameraBackendKind {
            CameraBackendKind::V4l2
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

    /// Creates a backend value that reports that `V4L2` is unavailable.
    #[must_use]
    pub fn create_backend() -> Box<dyn CameraBackend> {
        Box::<LinuxV4l2Backend>::default()
    }

    fn unavailable() -> CameraError {
        CameraError::new(
            CameraErrorKind::BackendUnavailable,
            "V4L2 is only available on Linux",
        )
    }
}

#[cfg(not(target_os = "linux"))]
pub use unavailable::{LinuxV4l2Backend, create_backend};
