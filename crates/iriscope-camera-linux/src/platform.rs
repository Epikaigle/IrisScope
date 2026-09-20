use std::{
    collections::HashMap,
    fs, io, mem,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use iriscope_core::camera::{
    CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
    CameraDeviceId, CameraError, CameraErrorKind, CameraEvent, CameraResult, CapturedFrame,
    StreamConfiguration, UsbDeviceIdentity,
};
use iriscope_core::capabilities::{
    CameraCapabilities, CameraControlDescriptor, CameraControlId, CameraControlKind,
    CameraControlMenuItem, CameraControlValue, CameraMode, FrameRate, PixelFormat, Resolution,
    StandardCameraControl,
};
use v4l::{
    Device,
    buffer::Type as BufferType,
    capability::Flags,
    context::{self, Node},
    control::{
        Control, Description as ControlDescription, Flags as ControlFlags, MenuItem,
        Type as ControlType, Value as ControlValue,
    },
    format::{Format, FourCC},
    fraction::Fraction,
    frameinterval::{FrameIntervalEnum, Stepwise as StepwiseFrameInterval},
    framesize::{FrameSizeEnum, Stepwise as StepwiseFrameSize},
    io::{mmap::Stream as MmapStream, traits::CaptureStream},
    v4l_sys::{
        V4L2_CTRL_FLAG_NEXT_COMPOUND, V4L2_CTRL_FLAG_NEXT_CTRL, v4l2_query_ext_ctrl, v4l2_querymenu,
    },
    v4l2,
    video::{Capture, capture::Parameters as CaptureParameters},
};

const MAX_EXPANDED_STEPWISE_MODES: usize = 4_096;
const MAX_EXPANDED_FRAME_RATES: usize = 512;

/// Native Linux camera backend using `V4L2` device nodes.
#[derive(Debug, Default)]
pub struct LinuxV4l2Backend {
    device_paths: HashMap<CameraDeviceId, PathBuf>,
    descriptors: HashMap<CameraDeviceId, CameraDescriptor>,
}

impl LinuxV4l2Backend {
    /// Creates an empty backend. Device discovery happens during enumeration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CameraBackend for LinuxV4l2Backend {
    fn kind(&self) -> CameraBackendKind {
        CameraBackendKind::V4l2
    }

    fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>> {
        let mut descriptors = Vec::new();
        let mut device_paths = HashMap::new();
        let mut descriptors_by_id = HashMap::new();
        let mut first_error = None;

        for node in context::enum_devices() {
            match inspect_node(&node) {
                Ok(Some(descriptor)) => {
                    device_paths.insert(descriptor.id.clone(), node.path().to_path_buf());
                    descriptors_by_id.insert(descriptor.id.clone(), descriptor.clone());
                    descriptors.push(descriptor);
                }
                Ok(None) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }

        if descriptors.is_empty()
            && let Some(error) = first_error
        {
            return Err(error);
        }

        descriptors.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        self.device_paths = device_paths;
        self.descriptors = descriptors_by_id;
        Ok(descriptors)
    }

    fn wait_for_device_event(
        &mut self,
        timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>> {
        let previous = self.descriptors.clone();
        std::thread::sleep(timeout);
        let current = self.enumerate_devices()?;

        for id in previous.keys() {
            if !self.descriptors.contains_key(id) {
                return Ok(Some(CameraDeviceEvent::Disconnected(id.clone())));
            }
        }

        for descriptor in current {
            if !previous.contains_key(&descriptor.id) {
                return Ok(Some(CameraDeviceEvent::Connected(descriptor)));
            }
        }

        Ok(None)
    }

    fn open(&mut self, device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        let path = self.device_paths.get(device_id).ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::DeviceNotFound,
                format!("camera {device_id} is not present in the latest enumeration"),
            )
        })?;
        let descriptor = self.descriptors.get(device_id).cloned().ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("camera {device_id} has no descriptor in the latest enumeration"),
            )
        })?;
        let device = Device::with_path(path)
            .map_err(|error| camera_io_error("opening the V4L2 device", &error))?;
        let (capabilities, native_controls) = discover_capabilities(&device)?;

        Ok(Box::new(LinuxV4l2Device {
            device,
            descriptor,
            capabilities,
            native_controls,
            stream: None,
            configuration: None,
            sequence_number: 0,
        }))
    }
}

struct LinuxV4l2Device {
    device: Device,
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
    native_controls: HashMap<CameraControlId, NativeControl>,
    stream: Option<MmapStream<'static>>,
    configuration: Option<StreamConfiguration>,
    sequence_number: u64,
}

impl CameraDevice for LinuxV4l2Device {
    fn descriptor(&self) -> &CameraDescriptor {
        &self.descriptor
    }

    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }

    fn start_stream(&mut self, configuration: &StreamConfiguration) -> CameraResult<()> {
        self.stop_stream()?;

        let fourcc = match &configuration.pixel_format {
            PixelFormat::Mjpeg => FourCC::new(b"MJPG"),
            PixelFormat::Yuyv => FourCC::new(b"YUYV"),
            PixelFormat::Nv12 => FourCC::new(b"NV12"),
            PixelFormat::Bgra8 => FourCC::new(b"BGRA"),
            PixelFormat::Other(name) => {
                let bytes = name.as_bytes();
                if bytes.len() == 4 {
                    FourCC::new(&[bytes[0], bytes[1], bytes[2], bytes[3]])
                } else {
                    return Err(CameraError::new(
                        CameraErrorKind::InvalidConfiguration,
                        format!("unsupported FourCC format: {name}"),
                    ));
                }
            }
            _ => {
                return Err(CameraError::new(
                    CameraErrorKind::InvalidConfiguration,
                    "unsupported pixel format",
                ));
            }
        };

        let format = Format::new(
            configuration.resolution.width,
            configuration.resolution.height,
            fourcc,
        );
        Capture::set_format(&self.device, &format)
            .map_err(|error| camera_io_error("setting V4L2 format", &error))?;

        let fps =
            configuration.frame_rate.numerator() / configuration.frame_rate.denominator().max(1);
        let mut params = CaptureParameters::with_fps(fps.max(1));
        params.interval = Fraction::new(
            configuration.frame_rate.denominator(),
            configuration.frame_rate.numerator(),
        );
        let _ = Capture::set_params(&self.device, &params);

        let stream = MmapStream::with_buffers(&self.device, BufferType::VideoCapture, 4)
            .map_err(|error| camera_io_error("allocating V4L2 MMAP stream buffers", &error))?;

        self.stream = Some(stream);
        self.configuration = Some(configuration.clone());
        self.sequence_number = 0;
        Ok(())
    }

    fn stop_stream(&mut self) -> CameraResult<()> {
        self.stream = None;
        self.configuration = None;
        Ok(())
    }

    fn next_event(&mut self, timeout: Duration) -> CameraResult<CameraEvent> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            CameraError::new(CameraErrorKind::Backend, "V4L2 stream is not running")
        })?;

        stream.set_timeout(timeout);

        let (buffer, metadata) = CaptureStream::next(stream).map_err(|error| {
            if error.kind() == io::ErrorKind::TimedOut {
                CameraError::new(
                    CameraErrorKind::TimedOut,
                    "timed out waiting for V4L2 frame",
                )
            } else if matches!(error.raw_os_error(), Some(5 | 19)) {
                CameraError::new(
                    CameraErrorKind::Disconnected,
                    format!("V4L2 camera disconnected while reading a frame: {error}"),
                )
                .with_platform_code(i64::from(error.raw_os_error().unwrap_or_default()))
            } else {
                camera_io_error("reading next V4L2 frame", &error)
            }
        })?;

        let bytes_used = (metadata.bytesused as usize).min(buffer.len());
        let payload = Arc::from(&buffer[..bytes_used]);
        self.sequence_number = self.sequence_number.saturating_add(1);

        let configuration = self.configuration.as_ref().ok_or_else(|| {
            CameraError::new(CameraErrorKind::Backend, "stream configuration is missing")
        })?;

        let sec = u64::try_from(metadata.timestamp.sec).unwrap_or_default();
        let usec = u64::try_from(metadata.timestamp.usec).unwrap_or_default();
        let timestamp = Duration::from_micros(sec.saturating_mul(1_000_000).saturating_add(usec));

        Ok(CameraEvent::Frame(CapturedFrame {
            sequence_number: self.sequence_number,
            timestamp,
            pixel_format: configuration.pixel_format.clone(),
            resolution: configuration.resolution,
            data: payload,
        }))
    }

    fn control_value(&self, control_id: &CameraControlId) -> CameraResult<CameraControlValue> {
        let control = self.native_control(control_id)?;
        if !control.readable {
            return Err(CameraError::new(
                CameraErrorKind::Unsupported,
                format!("V4L2 control {} cannot be read", control.name),
            ));
        }

        let value = self
            .device
            .control(control.native_id)
            .map_err(|error| camera_io_error("reading a V4L2 control", &error))?;
        control.map_native_value(value.value)
    }

    fn set_control_value(
        &mut self,
        control_id: &CameraControlId,
        value: &CameraControlValue,
    ) -> CameraResult<()> {
        let control = self.native_control(control_id)?;
        control.validate_value(value)?;
        let native_value = control.native_value(value)?;

        self.device
            .set_control(Control {
                id: control.native_id,
                value: native_value,
            })
            .map_err(|error| camera_io_error("writing a V4L2 control", &error))
    }

    fn reset_controls(&mut self) -> CameraResult<()> {
        for control in self
            .native_controls
            .values()
            .filter(|control| control.writable)
        {
            self.device
                .set_control(Control {
                    id: control.native_id,
                    value: control.native_default(),
                })
                .map_err(|error| {
                    camera_io_error(&format!("resetting V4L2 control {}", control.name), &error)
                })?;
        }

        Ok(())
    }
}

impl LinuxV4l2Device {
    fn native_control(&self, control_id: &CameraControlId) -> CameraResult<&NativeControl> {
        self.native_controls.get(control_id).ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Unsupported,
                "the requested V4L2 control was not advertised by this camera",
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeControlKind {
    Integer,
    Boolean,
    Menu,
}

#[derive(Clone, Debug)]
struct NativeControl {
    native_id: u32,
    name: String,
    kind: NativeControlKind,
    minimum: i64,
    maximum: i64,
    step: i64,
    default: i64,
    menu_values: Vec<i64>,
    readable: bool,
    writable: bool,
}

impl NativeControl {
    fn map_native_value(&self, value: ControlValue) -> CameraResult<CameraControlValue> {
        match (self.kind, value) {
            (NativeControlKind::Integer, ControlValue::Integer(value)) => {
                Ok(CameraControlValue::Integer(value))
            }
            (NativeControlKind::Boolean, ControlValue::Boolean(value)) => {
                Ok(CameraControlValue::Boolean(value))
            }
            (NativeControlKind::Menu, ControlValue::Integer(value)) => {
                Ok(CameraControlValue::Menu(value))
            }
            _ => Err(CameraError::new(
                CameraErrorKind::Backend,
                format!("V4L2 returned an unexpected value type for {}", self.name),
            )),
        }
    }

    fn validate_value(&self, value: &CameraControlValue) -> CameraResult<()> {
        if !self.writable {
            return Err(CameraError::new(
                CameraErrorKind::Unsupported,
                format!("V4L2 control {} is read-only", self.name),
            ));
        }

        let valid = match (self.kind, value) {
            (NativeControlKind::Integer, CameraControlValue::Integer(value)) => {
                *value >= self.minimum
                    && *value <= self.maximum
                    && (*value - self.minimum) % self.step == 0
            }
            (NativeControlKind::Boolean, CameraControlValue::Boolean(_)) => true,
            (NativeControlKind::Menu, CameraControlValue::Menu(value)) => {
                self.menu_values.contains(value)
            }
            _ => false,
        };

        if valid {
            Ok(())
        } else {
            Err(CameraError::new(
                CameraErrorKind::InvalidConfiguration,
                format!("invalid value for V4L2 control {}", self.name),
            ))
        }
    }

    fn native_value(&self, value: &CameraControlValue) -> CameraResult<ControlValue> {
        match (self.kind, value) {
            (
                NativeControlKind::Integer | NativeControlKind::Menu,
                CameraControlValue::Integer(value),
            )
            | (NativeControlKind::Menu, CameraControlValue::Menu(value)) => {
                Ok(ControlValue::Integer(*value))
            }
            (NativeControlKind::Boolean, CameraControlValue::Boolean(value)) => {
                Ok(ControlValue::Boolean(*value))
            }
            _ => Err(CameraError::new(
                CameraErrorKind::InvalidConfiguration,
                format!("invalid value type for V4L2 control {}", self.name),
            )),
        }
    }

    fn native_default(&self) -> ControlValue {
        match self.kind {
            NativeControlKind::Integer | NativeControlKind::Menu => {
                ControlValue::Integer(self.default)
            }
            NativeControlKind::Boolean => ControlValue::Boolean(self.default != 0),
        }
    }
}

fn discover_capabilities(
    device: &Device,
) -> CameraResult<(CameraCapabilities, HashMap<CameraControlId, NativeControl>)> {
    let modes = discover_modes(device)?;
    let (controls, native_controls) = discover_controls(device)?;

    Ok((CameraCapabilities { modes, controls }, native_controls))
}

fn discover_modes(device: &Device) -> CameraResult<Vec<CameraMode>> {
    let formats = device
        .enum_formats()
        .map_err(|error| camera_io_error("enumerating V4L2 pixel formats", &error))?;
    let mut modes = Vec::new();

    for format in formats {
        let pixel_format = map_pixel_format(format.fourcc);
        let frame_sizes = device
            .enum_framesizes(format.fourcc)
            .map_err(|error| camera_io_error("enumerating V4L2 frame sizes", &error))?;

        for frame_size in frame_sizes {
            for resolution in expand_frame_sizes(frame_size.size) {
                let intervals = device
                    .enum_frameintervals(format.fourcc, resolution.width, resolution.height)
                    .map_err(|error| camera_io_error("enumerating V4L2 frame intervals", &error))?;
                let mut frame_rates = Vec::new();

                for interval in intervals {
                    for frame_rate in expand_frame_intervals(interval.interval) {
                        push_unique(&mut frame_rates, frame_rate);
                    }
                }

                let mode = CameraMode {
                    pixel_format: pixel_format.clone(),
                    resolution,
                    frame_rates,
                };
                if !modes.contains(&mode) {
                    modes.push(mode);
                }
            }
        }
    }

    Ok(modes)
}

fn discover_controls(
    device: &Device,
) -> CameraResult<(
    Vec<CameraControlDescriptor>,
    HashMap<CameraControlId, NativeControl>,
)> {
    let descriptions = query_control_descriptions(device)?;
    let mut controls = Vec::new();
    let mut native_controls = HashMap::new();

    for description in descriptions {
        let Some((descriptor, native_control)) = map_control_description(&description) else {
            continue;
        };
        native_controls.insert(descriptor.id.clone(), native_control);
        controls.push(descriptor);
    }

    Ok((controls, native_controls))
}

fn query_control_descriptions(device: &Device) -> CameraResult<Vec<ControlDescription>> {
    let mut controls = Vec::new();
    // SAFETY: The all-zero value is the documented starting state for `VIDIOC_QUERY_EXT_CTRL`.
    let mut query = unsafe { mem::zeroed::<v4l2_query_ext_ctrl>() };

    loop {
        query.id |= V4L2_CTRL_FLAG_NEXT_CTRL | V4L2_CTRL_FLAG_NEXT_COMPOUND;
        // SAFETY: `query` is a valid writable `v4l2_query_ext_ctrl` for the lifetime of the call,
        // and the file descriptor belongs to the open device retained by `device`.
        let result = unsafe {
            v4l2::ioctl(
                device.handle().fd(),
                v4l2::vidioc::VIDIOC_QUERY_EXT_CTRL,
                (&raw mut query).cast(),
            )
        };

        match result {
            Ok(()) => {
                let mut description = ControlDescription::from(query);
                if matches!(
                    description.typ,
                    ControlType::Menu | ControlType::IntegerMenu
                ) {
                    description.items = Some(query_menu_items(device, &description));
                }
                controls.push(description);
            }
            Err(error)
                if error.kind() == io::ErrorKind::InvalidInput
                    || (error.raw_os_error() == Some(5) && !controls.is_empty()) =>
            {
                break;
            }
            Err(error) => {
                return Err(camera_io_error("enumerating V4L2 camera controls", &error));
            }
        }
    }

    Ok(controls)
}

fn query_menu_items(device: &Device, description: &ControlDescription) -> Vec<(u32, MenuItem)> {
    let mut items = Vec::new();
    let Ok(minimum) = u32::try_from(description.minimum) else {
        return items;
    };
    let Ok(maximum) = u32::try_from(description.maximum) else {
        return items;
    };
    let step = u32::try_from(description.step).unwrap_or(u32::MAX).max(1);

    for index in (minimum..=maximum).step_by(step as usize) {
        // SAFETY: The all-zero value is a valid base for a V4L2 query-menu request.
        let mut query = unsafe { mem::zeroed::<v4l2_querymenu>() };
        query.id = description.id;
        query.index = index;
        // SAFETY: `query` is a valid writable `v4l2_querymenu` for the lifetime of the call,
        // and the file descriptor belongs to the open device retained by `device`.
        let result = unsafe {
            v4l2::ioctl(
                device.handle().fd(),
                v4l2::vidioc::VIDIOC_QUERYMENU,
                (&raw mut query).cast(),
            )
        };
        if result.is_ok()
            && let Ok(item) = MenuItem::try_from((description.typ, query))
        {
            items.push((index, item));
        }
    }

    items
}

fn map_pixel_format(fourcc: FourCC) -> PixelFormat {
    match &fourcc.repr {
        b"MJPG" => PixelFormat::Mjpeg,
        b"YUYV" | b"YUY2" => PixelFormat::Yuyv,
        b"NV12" => PixelFormat::Nv12,
        b"BGRA" | b"BGR4" => PixelFormat::Bgra8,
        bytes => PixelFormat::Other(fourcc_label(*bytes)),
    }
}

fn fourcc_label(bytes: [u8; 4]) -> String {
    if bytes.iter().all(u8::is_ascii_graphic) {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        format!(
            "0x{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }
}

fn expand_frame_sizes(frame_size: FrameSizeEnum) -> Vec<Resolution> {
    match frame_size {
        FrameSizeEnum::Discrete(size) => vec![Resolution::new(size.width, size.height)],
        FrameSizeEnum::Stepwise(size) => expand_stepwise_frame_sizes(&size),
    }
}

fn expand_stepwise_frame_sizes(size: &StepwiseFrameSize) -> Vec<Resolution> {
    if size.step_width == 0
        || size.step_height == 0
        || size.min_width > size.max_width
        || size.min_height > size.max_height
    {
        return Vec::new();
    }

    let width_count = u64::from((size.max_width - size.min_width) / size.step_width) + 1;
    let height_count = u64::from((size.max_height - size.min_height) / size.step_height) + 1;
    if width_count.saturating_mul(height_count) > MAX_EXPANDED_STEPWISE_MODES as u64 {
        let mut boundaries = Vec::new();
        for resolution in [
            Resolution::new(size.min_width, size.min_height),
            Resolution::new(size.min_width, size.max_height),
            Resolution::new(size.max_width, size.min_height),
            Resolution::new(size.max_width, size.max_height),
        ] {
            push_unique(&mut boundaries, resolution);
        }
        return boundaries;
    }

    let capacity = usize::try_from(width_count * height_count)
        .expect("the stepwise mode count is capped before allocation");
    let mut resolutions = Vec::with_capacity(capacity);
    for width_index in 0..width_count {
        let width =
            size.min_width + u32::try_from(width_index).unwrap_or(u32::MAX) * size.step_width;
        for height_index in 0..height_count {
            let height = size.min_height
                + u32::try_from(height_index).unwrap_or(u32::MAX) * size.step_height;
            resolutions.push(Resolution::new(width, height));
        }
    }
    resolutions
}

fn expand_frame_intervals(interval: FrameIntervalEnum) -> Vec<FrameRate> {
    match interval {
        FrameIntervalEnum::Discrete(interval) => {
            interval_to_frame_rate(interval).into_iter().collect()
        }
        FrameIntervalEnum::Stepwise(interval) => expand_stepwise_frame_intervals(&interval),
    }
}

fn expand_stepwise_frame_intervals(interval: &StepwiseFrameInterval) -> Vec<FrameRate> {
    let Some(common_denominator) = common_denominator([
        interval.min.denominator,
        interval.max.denominator,
        interval.step.denominator,
    ]) else {
        return boundary_frame_rates(interval);
    };
    let Some(minimum) = scale_fraction(interval.min, common_denominator) else {
        return boundary_frame_rates(interval);
    };
    let Some(maximum) = scale_fraction(interval.max, common_denominator) else {
        return boundary_frame_rates(interval);
    };
    let Some(step) = scale_fraction(interval.step, common_denominator) else {
        return boundary_frame_rates(interval);
    };

    if step == 0 || minimum > maximum {
        return boundary_frame_rates(interval);
    }

    let count = ((maximum - minimum) / step).saturating_add(1);
    if count > MAX_EXPANDED_FRAME_RATES as u64 {
        return boundary_frame_rates(interval);
    }

    let capacity =
        usize::try_from(count).expect("the frame-rate count is capped before allocation");
    let mut frame_rates = Vec::with_capacity(capacity);
    for index in 0..count {
        let numerator = minimum + index * step;
        if let Some(frame_rate) = reciprocal_frame_rate(numerator, common_denominator) {
            push_unique(&mut frame_rates, frame_rate);
        }
    }
    frame_rates
}

fn boundary_frame_rates(interval: &StepwiseFrameInterval) -> Vec<FrameRate> {
    let mut frame_rates = Vec::new();
    for boundary in [interval.min, interval.max] {
        if let Some(frame_rate) = interval_to_frame_rate(boundary) {
            push_unique(&mut frame_rates, frame_rate);
        }
    }
    frame_rates
}

fn interval_to_frame_rate(interval: Fraction) -> Option<FrameRate> {
    FrameRate::new(interval.denominator, interval.numerator)
}

fn reciprocal_frame_rate(interval_numerator: u64, interval_denominator: u64) -> Option<FrameRate> {
    if interval_numerator == 0 || interval_denominator == 0 {
        return None;
    }
    let divisor = greatest_common_divisor(interval_numerator, interval_denominator);
    FrameRate::new(
        u32::try_from(interval_denominator / divisor).ok()?,
        u32::try_from(interval_numerator / divisor).ok()?,
    )
}

fn common_denominator(denominators: [u32; 3]) -> Option<u64> {
    denominators
        .into_iter()
        .try_fold(1_u64, |common, denominator| {
            if denominator == 0 {
                return None;
            }
            let denominator = u64::from(denominator);
            common
                .checked_div(greatest_common_divisor(common, denominator))?
                .checked_mul(denominator)
        })
}

fn scale_fraction(fraction: Fraction, denominator: u64) -> Option<u64> {
    if fraction.denominator == 0 {
        return None;
    }
    u64::from(fraction.numerator).checked_mul(denominator / u64::from(fraction.denominator))
}

const fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn map_control_description(
    description: &ControlDescription,
) -> Option<(CameraControlDescriptor, NativeControl)> {
    if description.flags.contains(ControlFlags::DISABLED) {
        return None;
    }

    let (kind, native_kind, menu_values, readable) = match description.typ {
        ControlType::Integer | ControlType::Integer64 | ControlType::Bitmask => (
            CameraControlKind::Integer {
                minimum: description.minimum,
                maximum: description.maximum,
                step: i64::try_from(description.step).unwrap_or(i64::MAX).max(1),
                default: description.default,
                unit: None,
            },
            NativeControlKind::Integer,
            Vec::new(),
            description.typ != ControlType::Bitmask,
        ),
        ControlType::Boolean => (
            CameraControlKind::Boolean {
                default: description.default != 0,
            },
            NativeControlKind::Boolean,
            Vec::new(),
            true,
        ),
        ControlType::Menu | ControlType::IntegerMenu => {
            let items = description
                .items
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|(value, item)| CameraControlMenuItem {
                    value: i64::from(*value),
                    label: match item {
                        MenuItem::Name(name) => name.clone(),
                        MenuItem::Value(value) => value.to_string(),
                    },
                })
                .collect::<Vec<_>>();
            let menu_values = items.iter().map(|item| item.value).collect();
            (
                CameraControlKind::Menu {
                    items,
                    default: description.default,
                },
                NativeControlKind::Menu,
                menu_values,
                description.typ == ControlType::Menu,
            )
        }
        ControlType::Button
        | ControlType::CtrlClass
        | ControlType::String
        | ControlType::U8
        | ControlType::U16
        | ControlType::U32
        | ControlType::Area => return None,
    };
    let id = map_control_id(description.id);
    let descriptor = CameraControlDescriptor {
        id: id.clone(),
        name: description.name.clone(),
        kind,
        read_only: description.flags.contains(ControlFlags::READ_ONLY),
    };
    let native_control = NativeControl {
        native_id: description.id,
        name: description.name.clone(),
        kind: native_kind,
        minimum: description.minimum,
        maximum: description.maximum,
        step: i64::try_from(description.step).unwrap_or(i64::MAX).max(1),
        default: description.default,
        menu_values,
        readable: readable && !description.flags.contains(ControlFlags::WRITE_ONLY),
        writable: !description.flags.contains(ControlFlags::READ_ONLY),
    };

    Some((descriptor, native_control))
}

#[allow(clippy::match_same_arms)]
fn map_control_id(native_id: u32) -> CameraControlId {
    const V4L2_CID_BRIGHTNESS: u32 = 0x0098_0900;
    const V4L2_CID_CONTRAST: u32 = 0x0098_0901;
    const V4L2_CID_SATURATION: u32 = 0x0098_0902;
    const V4L2_CID_HUE: u32 = 0x0098_0903;
    const V4L2_CID_AUTO_WHITE_BALANCE: u32 = 0x0098_090c;
    const V4L2_CID_GAMMA: u32 = 0x0098_0910;
    const V4L2_CID_POWER_LINE_FREQUENCY: u32 = 0x0098_0918;
    const V4L2_CID_WHITE_BALANCE_TEMPERATURE: u32 = 0x0098_091a;
    const V4L2_CID_SHARPNESS: u32 = 0x0098_091b;
    const V4L2_CID_EXPOSURE_AUTO: u32 = 0x009a_0901;
    const V4L2_CID_EXPOSURE_ABSOLUTE: u32 = 0x009a_0902;
    const V4L2_CID_FOCUS_ABSOLUTE: u32 = 0x009a_090a;

    let standard = match native_id {
        V4L2_CID_BRIGHTNESS => StandardCameraControl::Brightness,
        V4L2_CID_CONTRAST => StandardCameraControl::Contrast,
        V4L2_CID_SATURATION => StandardCameraControl::Saturation,
        V4L2_CID_HUE => StandardCameraControl::Hue,
        V4L2_CID_AUTO_WHITE_BALANCE => StandardCameraControl::WhiteBalanceAutomatic,
        V4L2_CID_WHITE_BALANCE_TEMPERATURE => StandardCameraControl::WhiteBalanceManual,
        V4L2_CID_GAMMA => StandardCameraControl::Gamma,
        V4L2_CID_POWER_LINE_FREQUENCY => StandardCameraControl::PowerLineFrequency,
        V4L2_CID_SHARPNESS => StandardCameraControl::Sharpness,
        V4L2_CID_EXPOSURE_AUTO => StandardCameraControl::ExposureMode,
        V4L2_CID_EXPOSURE_ABSOLUTE => StandardCameraControl::Exposure,
        V4L2_CID_FOCUS_ABSOLUTE => StandardCameraControl::Focus,
        _ => {
            return CameraControlId::PlatformSpecific(format!("v4l2:0x{native_id:08x}"));
        }
    };
    CameraControlId::Standard(standard)
}

/// Creates the native Linux camera backend.
#[must_use]
pub fn create_backend() -> Box<dyn CameraBackend> {
    Box::new(LinuxV4l2Backend::new())
}

fn inspect_node(node: &Node) -> CameraResult<Option<CameraDescriptor>> {
    let device = Device::with_path(node.path())
        .map_err(|error| camera_io_error("querying the V4L2 device", &error))?;
    let capabilities = device
        .query_caps()
        .map_err(|error| camera_io_error("reading V4L2 capabilities", &error))?;

    if !capabilities
        .capabilities
        .intersects(Flags::VIDEO_CAPTURE | Flags::VIDEO_CAPTURE_MPLANE)
    {
        return Ok(None);
    }

    let usb = usb_identity(node.path());
    let stream_index = stream_index(node.path()).unwrap_or(0);
    let id = stable_device_id(node.path(), &capabilities.bus, stream_index, usb.as_ref());

    Ok(Some(CameraDescriptor {
        id,
        display_name: capabilities.card,
        backend: CameraBackendKind::V4l2,
        usb,
    }))
}

fn stable_device_id(
    node_path: &Path,
    bus: &str,
    stream_index: u32,
    usb: Option<&UsbDeviceIdentity>,
) -> CameraDeviceId {
    if let Some(identity) = usb
        && let Some(serial_number) = identity.serial_number.as_deref()
    {
        return CameraDeviceId::new(format!(
            "v4l2:usb:{:04x}:{:04x}:{serial_number}:{stream_index}",
            identity.vendor_id, identity.product_id
        ));
    }

    if !bus.is_empty() {
        return CameraDeviceId::new(format!("v4l2:{bus}:{stream_index}"));
    }

    CameraDeviceId::new(format!("v4l2:{}", node_path.display()))
}

fn stream_index(node_path: &Path) -> Option<u32> {
    let name = node_path.file_name()?.to_str()?;
    read_trimmed(Path::new("/sys/class/video4linux").join(name).join("index"))
        .and_then(|value| value.parse().ok())
}

fn usb_identity(node_path: &Path) -> Option<UsbDeviceIdentity> {
    let name = node_path.file_name()?.to_str()?;
    let device_path = fs::canonicalize(
        Path::new("/sys/class/video4linux")
            .join(name)
            .join("device"),
    )
    .ok()?;

    device_path.ancestors().find_map(|ancestor| {
        let vendor_id = read_hex_u16(ancestor.join("idVendor"))?;
        let product_id = read_hex_u16(ancestor.join("idProduct"))?;

        Some(UsbDeviceIdentity {
            vendor_id,
            product_id,
            serial_number: read_trimmed(ancestor.join("serial")),
            hardware_revision: read_trimmed(ancestor.join("bcdDevice"))
                .map(|value| format_bcd_revision(&value)),
        })
    })
}

fn read_hex_u16(path: impl AsRef<Path>) -> Option<u16> {
    u16::from_str_radix(&read_trimmed(path)?, 16).ok()
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn format_bcd_revision(raw: &str) -> String {
    if raw.len() == 4 && raw.bytes().all(|byte| byte.is_ascii_digit()) {
        let major = raw[..2].trim_start_matches('0');
        let major = if major.is_empty() { "0" } else { major };
        format!("{major}.{}", &raw[2..])
    } else {
        raw.to_owned()
    }
}

fn camera_io_error(context: &str, error: &io::Error) -> CameraError {
    let kind = match error.kind() {
        io::ErrorKind::PermissionDenied => CameraErrorKind::PermissionDenied,
        io::ErrorKind::NotFound => CameraErrorKind::DeviceNotFound,
        io::ErrorKind::TimedOut => CameraErrorKind::TimedOut,
        _ if error.raw_os_error() == Some(16) => CameraErrorKind::DeviceBusy,
        _ => CameraErrorKind::Backend,
    };
    let platform_code = error.raw_os_error();
    let camera_error = CameraError::new(kind, format!("{context}: {error}"));

    platform_code.map_or(camera_error.clone(), |code| {
        camera_error.with_platform_code(i64::from(code))
    })
}

#[cfg(test)]
mod tests {
    use iriscope_core::{
        camera::{CameraBackend, UsbDeviceIdentity},
        capabilities::{
            CameraControlId, CameraControlKind, CameraControlValue, FrameRate, PixelFormat,
            Resolution, StandardCameraControl,
        },
    };
    use v4l::{
        control::{Description as ControlDescription, Flags as ControlFlags, MenuItem, Type},
        fraction::Fraction,
        frameinterval::Stepwise as StepwiseFrameInterval,
        framesize::Stepwise as StepwiseFrameSize,
    };

    use super::{
        LinuxV4l2Backend, expand_stepwise_frame_intervals, expand_stepwise_frame_sizes,
        format_bcd_revision, map_control_description, map_pixel_format, stable_device_id,
    };

    #[test]
    fn usb_serial_produces_a_stable_device_id() {
        let identity = UsbDeviceIdentity {
            vendor_id: 0x21cd,
            product_id: 0x603b,
            serial_number: Some("VTU603EB".to_owned()),
            hardware_revision: Some("3.27".to_owned()),
        };

        let id = stable_device_id(
            std::path::Path::new("/dev/video0"),
            "usb-0000:02:00.0-3",
            0,
            Some(&identity),
        );

        assert_eq!(id.as_str(), "v4l2:usb:21cd:603b:VTU603EB:0");
    }

    #[test]
    fn bcd_revision_is_human_readable() {
        assert_eq!(format_bcd_revision("0327"), "3.27");
        assert_eq!(format_bcd_revision("0100"), "1.00");
        assert_eq!(format_bcd_revision("not-bcd"), "not-bcd");
    }

    #[test]
    fn maps_common_and_unknown_v4l2_formats() {
        assert_eq!(
            map_pixel_format(v4l::format::FourCC::new(b"MJPG")),
            PixelFormat::Mjpeg
        );
        assert_eq!(
            map_pixel_format(v4l::format::FourCC::new(b"YUYV")),
            PixelFormat::Yuyv
        );
        assert_eq!(
            map_pixel_format(v4l::format::FourCC::new(b"NV12")),
            PixelFormat::Nv12
        );
        assert_eq!(
            map_pixel_format(v4l::format::FourCC::new(b"GREY")),
            PixelFormat::Other("GREY".to_owned())
        );
    }

    #[test]
    fn expands_small_stepwise_ranges_without_losing_values() {
        let resolutions = expand_stepwise_frame_sizes(&StepwiseFrameSize {
            min_width: 320,
            max_width: 640,
            step_width: 320,
            min_height: 240,
            max_height: 480,
            step_height: 240,
        });

        assert_eq!(
            resolutions,
            vec![
                Resolution::new(320, 240),
                Resolution::new(320, 480),
                Resolution::new(640, 240),
                Resolution::new(640, 480),
            ]
        );

        let frame_rates = expand_stepwise_frame_intervals(&StepwiseFrameInterval {
            min: Fraction::new(1, 30),
            max: Fraction::new(1, 10),
            step: Fraction::new(1, 30),
        });
        assert_eq!(
            frame_rates,
            vec![
                FrameRate::new(30, 1).expect("valid frame rate"),
                FrameRate::new(15, 1).expect("valid frame rate"),
                FrameRate::new(10, 1).expect("valid frame rate"),
            ]
        );
    }

    #[test]
    fn maps_standard_and_menu_controls() {
        let brightness = ControlDescription {
            id: 0x0098_0900,
            typ: Type::Integer,
            name: "Brightness".to_owned(),
            minimum: 0,
            maximum: 255,
            step: 1,
            default: 127,
            flags: ControlFlags::empty(),
            items: None,
        };
        let (descriptor, native) =
            map_control_description(&brightness).expect("brightness is representable");
        assert_eq!(
            descriptor.id,
            CameraControlId::Standard(StandardCameraControl::Brightness)
        );
        assert_eq!(
            descriptor.kind,
            CameraControlKind::Integer {
                minimum: 0,
                maximum: 255,
                step: 1,
                default: 127,
                unit: None,
            }
        );
        assert!(
            native
                .validate_value(&CameraControlValue::Integer(200))
                .is_ok()
        );
        assert!(
            native
                .validate_value(&CameraControlValue::Integer(256))
                .is_err()
        );

        let power_line_frequency = ControlDescription {
            id: 0x0098_0918,
            typ: Type::Menu,
            name: "Power Line Frequency".to_owned(),
            minimum: 0,
            maximum: 2,
            step: 1,
            default: 2,
            flags: ControlFlags::empty(),
            items: Some(vec![
                (0, MenuItem::Name("Disabled".to_owned())),
                (1, MenuItem::Name("50 Hz".to_owned())),
                (2, MenuItem::Name("60 Hz".to_owned())),
            ]),
        };
        let (descriptor, native) = map_control_description(&power_line_frequency)
            .expect("power-line frequency is representable");
        assert_eq!(
            descriptor.id,
            CameraControlId::Standard(StandardCameraControl::PowerLineFrequency)
        );
        assert!(native.validate_value(&CameraControlValue::Menu(1)).is_ok());
        assert!(native.validate_value(&CameraControlValue::Menu(3)).is_err());
    }

    #[test]
    #[ignore = "requires a connected DE400 iridoscope"]
    fn connected_de400_exposes_modes_and_readable_controls() {
        let mut backend = LinuxV4l2Backend::new();
        let descriptor = backend
            .enumerate_devices()
            .expect("V4L2 enumeration should succeed")
            .into_iter()
            .find(|descriptor| {
                descriptor
                    .usb
                    .as_ref()
                    .is_some_and(|usb| usb.vendor_id == 0x21cd && usb.product_id == 0x603b)
            })
            .expect("the DE400 should be connected");
        let device = backend.open(&descriptor.id).expect("the DE400 should open");
        let capabilities = device.capabilities();

        assert_eq!(device.descriptor(), &descriptor);
        assert!(capabilities.modes.iter().any(|mode| {
            mode.pixel_format == PixelFormat::Mjpeg
                && mode.resolution == Resolution::new(1280, 1024)
                && mode
                    .frame_rates
                    .contains(&FrameRate::new(8, 1).expect("valid frame rate"))
        }));
        assert!(capabilities.modes.iter().any(|mode| {
            mode.pixel_format == PixelFormat::Yuyv
                && mode.resolution == Resolution::new(640, 480)
                && mode
                    .frame_rates
                    .contains(&FrameRate::new(30, 1).expect("valid frame rate"))
        }));
        assert!(capabilities.controls.len() >= 10);

        for control in &capabilities.controls {
            let _ = device
                .control_value(&control.id)
                .expect("every readable DE400 control should return its current value");
        }
    }
}
