//! Common camera abstraction implemented by every native backend.

use std::{error::Error, fmt, sync::Arc, time::Duration};

use crate::capabilities::{
    CameraCapabilities, CameraControlId, CameraControlValue, FrameRate, PixelFormat, Resolution,
};

/// Native camera API used by a backend.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CameraBackendKind {
    /// `Video4Linux2` on Linux.
    V4l2,
    /// Media Foundation on Windows.
    MediaFoundation,
    /// `AVFoundation` on macOS.
    AvFoundation,
}

impl fmt::Display for CameraBackendKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::V4l2 => "V4L2",
            Self::MediaFoundation => "Media Foundation",
            Self::AvFoundation => "AVFoundation",
        })
    }
}

/// Opaque identifier that is meaningful to the backend that created it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CameraDeviceId(String);

impl CameraDeviceId {
    /// Creates a backend-specific device identifier.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CameraDeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// USB identity when the operating system makes it available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsbDeviceIdentity {
    /// USB vendor identifier.
    pub vendor_id: u16,
    /// USB product identifier.
    pub product_id: u16,
    /// Device serial number, if exposed.
    pub serial_number: Option<String>,
    /// Hardware revision, if exposed.
    pub hardware_revision: Option<String>,
}

/// A camera found by a native backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraDescriptor {
    /// Opaque identifier passed back to the same backend when opening the camera.
    pub id: CameraDeviceId,
    /// Human-readable device name.
    pub display_name: String,
    /// Native API that discovered the camera.
    pub backend: CameraBackendKind,
    /// USB identity, when available.
    pub usb: Option<UsbDeviceIdentity>,
}

/// An exact mode requested when starting the stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamConfiguration {
    /// Frame format.
    pub pixel_format: PixelFormat,
    /// Frame dimensions.
    pub resolution: Resolution,
    /// Requested frame rate.
    pub frame_rate: FrameRate,
}

/// A camera connection change reported by a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CameraDeviceEvent {
    /// A camera became available.
    Connected(CameraDescriptor),
    /// A camera is no longer available.
    Disconnected(CameraDeviceId),
}

/// One frame delivered by the camera.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedFrame {
    /// Monotonically increasing sequence number supplied by the backend.
    pub sequence_number: u64,
    /// Time elapsed since the stream started.
    pub timestamp: Duration,
    /// Frame format before any display conversion.
    pub pixel_format: PixelFormat,
    /// Frame dimensions.
    pub resolution: Resolution,
    /// Native frame bytes shared without forcing an additional copy.
    pub data: Arc<[u8]>,
}

/// An event produced while a camera is open.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CameraEvent {
    /// A native frame, including the original MJPEG bytes when applicable.
    Frame(CapturedFrame),
    /// The physical camera button was pressed.
    HardwareButtonPressed,
    /// The physical camera button was released.
    HardwareButtonReleased,
    /// The open camera was disconnected.
    Disconnected,
}

/// Broad error category shared by all camera APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CameraErrorKind {
    /// The native camera service is unavailable.
    BackendUnavailable,
    /// The operating system denied access to the camera.
    PermissionDenied,
    /// The selected device no longer exists.
    DeviceNotFound,
    /// Another process or session is using the device exclusively.
    DeviceBusy,
    /// The requested operation or setting is unsupported.
    Unsupported,
    /// A stream mode or control value is invalid.
    InvalidConfiguration,
    /// An operation did not finish within its allowed time.
    TimedOut,
    /// The open device was disconnected.
    Disconnected,
    /// The native API returned another failure.
    Backend,
}

/// Error returned through the common camera API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraError {
    kind: CameraErrorKind,
    message: String,
    platform_code: Option<i64>,
}

impl CameraError {
    /// Creates a camera error without a native error code.
    #[must_use]
    pub fn new(kind: CameraErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            platform_code: None,
        }
    }

    /// Adds an operating-system or native API error code.
    #[must_use]
    pub const fn with_platform_code(mut self, platform_code: i64) -> Self {
        self.platform_code = Some(platform_code);
        self
    }

    /// Returns the shared error category.
    #[must_use]
    pub const fn kind(&self) -> CameraErrorKind {
        self.kind
    }

    /// Returns the native error code when one was provided.
    #[must_use]
    pub const fn platform_code(&self) -> Option<i64> {
        self.platform_code
    }
}

impl fmt::Display for CameraError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = self.platform_code {
            write!(formatter, "{} (platform code {code})", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl Error for CameraError {}

/// Result type returned by camera operations.
pub type CameraResult<T> = Result<T, CameraError>;

/// Largest accepted width or height of a native camera frame.
pub const MAX_CAMERA_FRAME_DIMENSION: u32 = 8_192;
/// Largest accepted decoded camera image.
pub const MAX_CAMERA_FRAME_PIXELS: u64 = 16_000_000;
/// Largest accepted native MJPEG sample.
pub const MAX_CAMERA_MJPEG_BYTES: usize = 32 * 1024 * 1024;
/// Largest accepted native uncompressed sample or packed image.
pub const MAX_CAMERA_RAW_BYTES: usize = 64 * 1024 * 1024;

/// Returns the byte limit for a supported frame format after validating its dimensions.
///
/// # Errors
///
/// Rejects zero, excessive or incompatible dimensions and unknown formats.
pub fn camera_frame_byte_limit(
    pixel_format: &PixelFormat,
    resolution: Resolution,
) -> CameraResult<usize> {
    if resolution.width == 0
        || resolution.height == 0
        || resolution.width > MAX_CAMERA_FRAME_DIMENSION
        || resolution.height > MAX_CAMERA_FRAME_DIMENSION
        || resolution.pixel_count() > MAX_CAMERA_FRAME_PIXELS
    {
        return Err(CameraError::new(
            CameraErrorKind::InvalidConfiguration,
            format!("camera frame dimensions {resolution} exceed safe limits"),
        ));
    }
    if (matches!(pixel_format, PixelFormat::Yuyv | PixelFormat::Nv12) && resolution.width % 2 != 0)
        || (matches!(pixel_format, PixelFormat::Nv12) && resolution.height % 2 != 0)
    {
        return Err(CameraError::new(
            CameraErrorKind::InvalidConfiguration,
            format!("camera frame dimensions {resolution} are incompatible with {pixel_format}"),
        ));
    }
    match pixel_format {
        PixelFormat::Mjpeg => Ok(MAX_CAMERA_MJPEG_BYTES),
        PixelFormat::Yuyv | PixelFormat::Nv12 | PixelFormat::Bgra8 => Ok(MAX_CAMERA_RAW_BYTES),
        PixelFormat::Other(_) => Err(CameraError::new(
            CameraErrorKind::Unsupported,
            format!("camera frame format {pixel_format} is unsupported"),
        )),
    }
}

/// Checks a native sample size before copying or allocating its bytes.
///
/// # Errors
///
/// Rejects an empty or oversized sample, or an invalid mode.
pub fn validate_camera_frame_bytes(
    pixel_format: &PixelFormat,
    resolution: Resolution,
    bytes: usize,
) -> CameraResult<()> {
    let limit = camera_frame_byte_limit(pixel_format, resolution)?;
    if bytes == 0 || bytes > limit {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            format!("camera frame contains {bytes} bytes; allowed range is 1..={limit}"),
        ));
    }
    Ok(())
}

/// Platform-independent entry point implemented by each native camera backend.
pub trait CameraBackend: Send {
    /// Identifies the native API used by this backend.
    fn kind(&self) -> CameraBackendKind;

    /// Lists the cameras currently exposed by the native API.
    ///
    /// # Errors
    ///
    /// Returns an error when the native API cannot enumerate its devices.
    fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>>;

    /// Waits for a device connection change for at most `timeout`.
    ///
    /// `Ok(None)` means that no event occurred before the timeout.
    ///
    /// # Errors
    ///
    /// Returns an error when native hotplug monitoring fails.
    fn wait_for_device_event(
        &mut self,
        timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>>;

    /// Opens a camera previously returned by [`Self::enumerate_devices`].
    ///
    /// # Errors
    ///
    /// Returns an error when the device disappeared, is busy, or cannot be opened.
    fn open(&mut self, device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>>;
}

/// An open camera controlled by the dedicated capture thread.
pub trait CameraDevice: Send {
    /// Returns the identity recorded when this camera was opened.
    fn descriptor(&self) -> &CameraDescriptor;

    /// Returns the capabilities discovered from this camera.
    fn capabilities(&self) -> &CameraCapabilities;

    /// Starts delivery of camera events with an advertised configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is unsupported or streaming fails.
    fn start_stream(&mut self, configuration: &StreamConfiguration) -> CameraResult<()>;

    /// Returns the mode accepted by the native API after starting a stream.
    /// Backends that cannot query it may return `None`.
    fn active_configuration(&self) -> Option<StreamConfiguration> {
        None
    }

    /// Reports the current optional hardware button receiver state.
    fn hardware_button_status(&self) -> Option<String> {
        None
    }

    /// Stops the active stream. Implementations must also stop safely when dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when the native API cannot stop the stream cleanly.
    fn stop_stream(&mut self) -> CameraResult<()>;

    /// Waits for the next frame, button event, or disconnect for at most `timeout`.
    ///
    /// # Errors
    ///
    /// Returns [`CameraErrorKind::TimedOut`] when no event arrives before the timeout,
    /// or another error if capture fails.
    fn next_event(&mut self, timeout: Duration) -> CameraResult<CameraEvent>;

    /// Reads the current value of an advertised control.
    ///
    /// # Errors
    ///
    /// Returns an error if the control is unavailable or cannot be read.
    fn control_value(&self, control_id: &CameraControlId) -> CameraResult<CameraControlValue>;

    /// Updates an advertised camera control.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is invalid, read-only, or rejected by the camera.
    fn set_control_value(
        &mut self,
        control_id: &CameraControlId,
        value: &CameraControlValue,
    ) -> CameraResult<()>;

    /// Restores every writable control to the default reported by the camera.
    ///
    /// # Errors
    ///
    /// Returns an error if one or more native controls cannot be restored.
    fn reset_controls(&mut self) -> CameraResult<()>;
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use crate::capabilities::{
        CameraCapabilities, CameraControlId, CameraControlValue, CameraMode, FrameRate,
        PixelFormat, Resolution,
    };

    use super::{
        CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
        CameraDeviceId, CameraError, CameraErrorKind, CameraEvent, CameraResult, CapturedFrame,
        MAX_CAMERA_FRAME_DIMENSION, MAX_CAMERA_FRAME_PIXELS, MAX_CAMERA_MJPEG_BYTES,
        MAX_CAMERA_RAW_BYTES, StreamConfiguration, camera_frame_byte_limit,
        validate_camera_frame_bytes,
    };

    #[test]
    fn frame_limits_accept_de400_and_exact_byte_boundaries() {
        let de400 = Resolution::new(1280, 1024);
        assert_eq!(
            camera_frame_byte_limit(&PixelFormat::Mjpeg, de400),
            Ok(MAX_CAMERA_MJPEG_BYTES)
        );
        assert_eq!(
            camera_frame_byte_limit(&PixelFormat::Bgra8, de400),
            Ok(MAX_CAMERA_RAW_BYTES)
        );
        assert_eq!(
            camera_frame_byte_limit(&PixelFormat::Bgra8, Resolution::new(4_000, 4_000)),
            Ok(MAX_CAMERA_RAW_BYTES)
        );
        assert_eq!(
            camera_frame_byte_limit(
                &PixelFormat::Mjpeg,
                Resolution::new(MAX_CAMERA_FRAME_DIMENSION, 1),
            ),
            Ok(MAX_CAMERA_MJPEG_BYTES)
        );
        assert!(
            validate_camera_frame_bytes(&PixelFormat::Mjpeg, de400, MAX_CAMERA_MJPEG_BYTES).is_ok()
        );
        assert!(
            validate_camera_frame_bytes(&PixelFormat::Bgra8, de400, MAX_CAMERA_RAW_BYTES).is_ok()
        );
        assert_eq!(MAX_CAMERA_FRAME_PIXELS, 16_000_000);
    }

    #[test]
    fn frame_limits_reject_empty_oversized_and_incompatible_modes() {
        let de400 = Resolution::new(1280, 1024);
        for bytes in [0, MAX_CAMERA_MJPEG_BYTES + 1] {
            assert_eq!(
                validate_camera_frame_bytes(&PixelFormat::Mjpeg, de400, bytes)
                    .expect_err("invalid MJPEG byte count")
                    .kind(),
                CameraErrorKind::Backend
            );
        }
        assert_eq!(
            validate_camera_frame_bytes(&PixelFormat::Bgra8, de400, MAX_CAMERA_RAW_BYTES + 1)
                .expect_err("oversized raw frame")
                .kind(),
            CameraErrorKind::Backend
        );
        for resolution in [
            Resolution::new(0, 1024),
            Resolution::new(MAX_CAMERA_FRAME_DIMENSION + 1, 1),
            Resolution::new(4_000, 4_001),
            Resolution::new(u32::MAX, u32::MAX),
        ] {
            assert_eq!(
                camera_frame_byte_limit(&PixelFormat::Mjpeg, resolution)
                    .expect_err("unsafe camera dimensions")
                    .kind(),
                CameraErrorKind::InvalidConfiguration
            );
        }
        assert!(camera_frame_byte_limit(&PixelFormat::Yuyv, Resolution::new(1279, 1024)).is_err());
        assert!(camera_frame_byte_limit(&PixelFormat::Nv12, Resolution::new(1280, 1023)).is_err());
        assert_eq!(
            camera_frame_byte_limit(&PixelFormat::Other("unknown".to_owned()), de400)
                .expect_err("unknown camera format")
                .kind(),
            CameraErrorKind::Unsupported
        );
    }

    struct FakeBackend {
        descriptor: CameraDescriptor,
        capabilities: CameraCapabilities,
    }

    impl CameraBackend for FakeBackend {
        fn kind(&self) -> CameraBackendKind {
            CameraBackendKind::V4l2
        }

        fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>> {
            Ok(vec![self.descriptor.clone()])
        }

        fn wait_for_device_event(
            &mut self,
            _timeout: Duration,
        ) -> CameraResult<Option<CameraDeviceEvent>> {
            Ok(None)
        }

        fn open(&mut self, device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
            if device_id != &self.descriptor.id {
                return Err(CameraError::new(
                    CameraErrorKind::DeviceNotFound,
                    "unknown fake camera",
                ));
            }

            Ok(Box::new(FakeDevice {
                descriptor: self.descriptor.clone(),
                capabilities: self.capabilities.clone(),
                streaming: false,
            }))
        }
    }

    struct FakeDevice {
        descriptor: CameraDescriptor,
        capabilities: CameraCapabilities,
        streaming: bool,
    }

    impl CameraDevice for FakeDevice {
        fn descriptor(&self) -> &CameraDescriptor {
            &self.descriptor
        }

        fn capabilities(&self) -> &CameraCapabilities {
            &self.capabilities
        }

        fn start_stream(&mut self, configuration: &StreamConfiguration) -> CameraResult<()> {
            let Some(mode) = self
                .capabilities
                .find_mode(&configuration.pixel_format, configuration.resolution)
            else {
                return Err(CameraError::new(
                    CameraErrorKind::InvalidConfiguration,
                    "mode is unavailable",
                ));
            };

            if !mode.supports_frame_rate(configuration.frame_rate) {
                return Err(CameraError::new(
                    CameraErrorKind::InvalidConfiguration,
                    "frame rate is unavailable",
                ));
            }

            self.streaming = true;
            Ok(())
        }

        fn stop_stream(&mut self) -> CameraResult<()> {
            self.streaming = false;
            Ok(())
        }

        fn next_event(&mut self, _timeout: Duration) -> CameraResult<CameraEvent> {
            if !self.streaming {
                return Err(CameraError::new(
                    CameraErrorKind::Backend,
                    "stream is stopped",
                ));
            }

            Ok(CameraEvent::Frame(CapturedFrame {
                sequence_number: 1,
                timestamp: Duration::ZERO,
                pixel_format: PixelFormat::Mjpeg,
                resolution: Resolution::new(1280, 1024),
                data: Arc::from([0xff, 0xd8, 0xff, 0xd9]),
            }))
        }

        fn control_value(&self, _control_id: &CameraControlId) -> CameraResult<CameraControlValue> {
            Err(CameraError::new(
                CameraErrorKind::Unsupported,
                "fake camera has no controls",
            ))
        }

        fn set_control_value(
            &mut self,
            _control_id: &CameraControlId,
            _value: &CameraControlValue,
        ) -> CameraResult<()> {
            Err(CameraError::new(
                CameraErrorKind::Unsupported,
                "fake camera has no controls",
            ))
        }

        fn reset_controls(&mut self) -> CameraResult<()> {
            Ok(())
        }
    }

    #[test]
    fn backend_and_device_are_usable_through_trait_objects() {
        let frame_rate = FrameRate::new(8, 1).expect("8 fps is valid");
        let resolution = Resolution::new(1280, 1024);
        let descriptor = CameraDescriptor {
            id: CameraDeviceId::new("fake:0"),
            display_name: "Fake DE400".to_owned(),
            backend: CameraBackendKind::V4l2,
            usb: None,
        };
        let capabilities = CameraCapabilities {
            modes: vec![CameraMode {
                pixel_format: PixelFormat::Mjpeg,
                resolution,
                frame_rates: vec![frame_rate],
            }],
            controls: Vec::new(),
        };
        let mut backend: Box<dyn CameraBackend> = Box::new(FakeBackend {
            descriptor: descriptor.clone(),
            capabilities,
        });

        assert_eq!(backend.kind(), CameraBackendKind::V4l2);
        assert_eq!(backend.enumerate_devices(), Ok(vec![descriptor.clone()]));

        let mut device = backend
            .open(&descriptor.id)
            .expect("the fake camera should open");
        device
            .start_stream(&StreamConfiguration {
                pixel_format: PixelFormat::Mjpeg,
                resolution,
                frame_rate,
            })
            .expect("the advertised mode should start");

        let CameraEvent::Frame(frame) = device
            .next_event(Duration::from_millis(100))
            .expect("a frame should be available")
        else {
            panic!("expected a frame event");
        };

        assert_eq!(frame.data.as_ref(), [0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(frame.resolution, resolution);
        device.stop_stream().expect("the stream should stop");
    }
}
