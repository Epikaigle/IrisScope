use std::time::Duration;

use av_foundation::{capture_device::AVCaptureDevice, media_format::AVMediaTypeVideo};
use core_media::{
    format_description::{CMVideoFormatDescription, kCMMediaType_Video},
    time::{CMTime, kCMTimeFlags_ImpliedValueFlagsMask, kCMTimeFlags_Valid},
};
use iriscope_core::camera::{
    CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
    CameraDeviceId, CameraError, CameraErrorKind, CameraEvent, CameraResult, StreamConfiguration,
};
use iriscope_core::capabilities::{
    CameraCapabilities, CameraControlId, CameraControlValue, CameraMode, FrameRate, Resolution,
};
use objc2_foundation::NSString;

use crate::capabilities::{frame_rate_from_duration_parts, merge_mode, pixel_format_from_ostype};

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

    fn open(&mut self, device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        let unique_id = NSString::from_str(device_id.as_str());
        let device = AVCaptureDevice::device_with_unique_id(&unique_id).ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::DeviceNotFound,
                format!("camera {device_id} is not available through AVFoundation"),
            )
        })?;

        if !device.is_connected().is_true() {
            return Err(CameraError::new(
                CameraErrorKind::DeviceNotFound,
                format!("camera {device_id} was disconnected before it could be opened"),
            ));
        }

        let descriptor = descriptor_from_device(&device);
        let capabilities = capabilities_from_device(&device);
        Ok(Box::new(MacAvFoundationDevice {
            descriptor,
            capabilities,
        }))
    }
}

struct MacAvFoundationDevice {
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
}

impl CameraDevice for MacAvFoundationDevice {
    fn descriptor(&self) -> &CameraDescriptor {
        &self.descriptor
    }

    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }

    fn start_stream(&mut self, _configuration: &StreamConfiguration) -> CameraResult<()> {
        Err(streaming_unavailable())
    }

    fn stop_stream(&mut self) -> CameraResult<()> {
        Err(streaming_unavailable())
    }

    fn next_event(&mut self, _timeout: Duration) -> CameraResult<CameraEvent> {
        Err(streaming_unavailable())
    }

    fn control_value(&self, _control_id: &CameraControlId) -> CameraResult<CameraControlValue> {
        Err(controls_unavailable())
    }

    fn set_control_value(
        &mut self,
        _control_id: &CameraControlId,
        _value: &CameraControlValue,
    ) -> CameraResult<()> {
        Err(controls_unavailable())
    }

    fn reset_controls(&mut self) -> CameraResult<()> {
        Err(controls_unavailable())
    }
}

/// Creates the native macOS camera backend.
#[must_use]
pub fn create_backend() -> Box<dyn CameraBackend> {
    Box::new(MacAvFoundationBackend::new())
}

fn descriptor_from_device(device: &AVCaptureDevice) -> CameraDescriptor {
    CameraDescriptor {
        id: CameraDeviceId::new(device.unique_id().to_string()),
        display_name: device.localized_name().to_string(),
        backend: CameraBackendKind::AvFoundation,
        usb: None,
    }
}

fn capabilities_from_device(device: &AVCaptureDevice) -> CameraCapabilities {
    let mut modes = Vec::new();

    for format in &device.formats() {
        let format_description = format.format_description();
        if format_description.get_media_type() != kCMMediaType_Video {
            continue;
        }

        // `AVCaptureDeviceFormat` guarantees a video format description here. Checking the
        // media type before the Core Foundation downcast keeps the conversion explicit and
        // avoids calling video-only APIs on a different CMFormatDescription payload.
        let Some(video_description) =
            format_description.downcast_into::<CMVideoFormatDescription>()
        else {
            continue;
        };
        let dimensions = video_description.get_dimensions();
        let (Ok(width), Ok(height)) = (
            u32::try_from(dimensions.width),
            u32::try_from(dimensions.height),
        ) else {
            continue;
        };
        if width == 0 || height == 0 {
            continue;
        }

        let mut frame_rates = Vec::new();
        for range in &format.video_supported_frame_rate_ranges() {
            // CameraMode currently represents a native AVFrameRateRange with its exact min/max
            // boundaries. Equal boundaries collapse to one rate; continuous intermediate values
            // remain implicit until the shared capability model gains a range type.
            if let Some(minimum) = frame_rate_from_duration(range.max_frame_duration()) {
                frame_rates.push(minimum);
            }
            if let Some(maximum) = frame_rate_from_duration(range.min_frame_duration()) {
                frame_rates.push(maximum);
            }
        }

        merge_mode(
            &mut modes,
            CameraMode {
                pixel_format: pixel_format_from_ostype(video_description.get_codec_type()),
                resolution: Resolution::new(width, height),
                frame_rates,
            },
        );
    }

    CameraCapabilities {
        modes,
        controls: Vec::new(),
    }
}

fn frame_rate_from_duration(duration: CMTime) -> Option<FrameRate> {
    if duration.flags & kCMTimeFlags_Valid == 0
        || duration.flags & kCMTimeFlags_ImpliedValueFlagsMask != 0
    {
        return None;
    }

    frame_rate_from_duration_parts(duration.value, duration.timescale)
}

fn streaming_unavailable() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "AVFoundation streaming is not implemented yet",
    )
}

fn controls_unavailable() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "AVFoundation camera controls are not implemented yet",
    )
}
