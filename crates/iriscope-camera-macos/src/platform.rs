use std::{
    slice,
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use av_foundation::{
    capture_device::{AVCaptureDevice, AVCaptureDeviceFormat},
    capture_input::AVCaptureDeviceInput,
    capture_output_base::AVCaptureOutput,
    capture_session::{AVCaptureConnection, AVCaptureSession},
    capture_video_data_output::{
        AVCaptureVideoDataOutput, AVCaptureVideoDataOutputSampleBufferDelegate,
    },
    media_format::AVMediaTypeVideo,
};
use core_foundation::base::TCFType;
use core_media::{
    format_description::{CMVideoFormatDescription, kCMMediaType_Video},
    sample_buffer::{CMSampleBuffer, CMSampleBufferRef},
    time::{CMTime, kCMTimeFlags_ImpliedValueFlagsMask, kCMTimeFlags_Valid},
};
use core_video::{
    pixel_buffer::{
        CVPixelBuffer, kCVPixelBufferLock_ReadOnly, kCVPixelFormatType_32BGRA,
        kCVPixelFormatType_422YpCbCr8, kCVPixelFormatType_422YpCbCr8_yuvs,
    },
    r#return::kCVReturnSuccess,
};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use iriscope_core::camera::{
    CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
    CameraDeviceId, CameraError, CameraErrorKind, CameraEvent, CameraResult, CapturedFrame,
    StreamConfiguration,
};
use iriscope_core::capabilities::{
    CameraCapabilities, CameraControlId, CameraControlValue, CameraMode, FrameRate, PixelFormat,
    Resolution,
};
use objc2::{
    AnyThread, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::ProtocolObject,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use crate::capabilities::{frame_rate_from_duration_parts, merge_mode, pixel_format_from_ostype};

/// Native macOS camera backend using AVFoundation.
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
        let devices = video_devices();
        let mut descriptors = devices
            .iter()
            .map(|device| descriptor_from_device(&device))
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
        Ok(Box::new(open_video_device(device_id)?))
    }
}

fn video_devices() -> Retained<objc2_foundation::NSArray<AVCaptureDevice>> {
    // SAFETY: AVMediaTypeVideo is a process-lifetime AVFoundation constant.
    let media_type = unsafe { AVMediaTypeVideo };
    AVCaptureDevice::devices_with_media_type(media_type)
}

struct MacAvFoundationDevice {
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
    command_sender: SyncSender<WorkerCommand>,
    event_receiver: Receiver<CameraResult<CameraEvent>>,
    worker: Option<JoinHandle<()>>,
}

enum WorkerCommand {
    Start(StreamConfiguration, SyncSender<CameraResult<()>>),
    Stop(SyncSender<CameraResult<()>>),
    Shutdown,
}

impl CameraDevice for MacAvFoundationDevice {
    fn descriptor(&self) -> &CameraDescriptor {
        &self.descriptor
    }

    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }

    fn start_stream(&mut self, configuration: &StreamConfiguration) -> CameraResult<()> {
        if self
            .capabilities
            .find_mode(&configuration.pixel_format, configuration.resolution)
            .is_none()
        {
            return Err(CameraError::new(
                CameraErrorKind::InvalidConfiguration,
                "requested AVFoundation camera mode is not advertised",
            ));
        }

        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::Start(configuration.clone(), response_sender))
            .map_err(|error| worker_channel_error("starting the AVFoundation stream", error))?;
        response_receiver.recv().map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("AVFoundation worker stopped while starting the stream: {error}"),
            )
        })?
    }

    fn stop_stream(&mut self) -> CameraResult<()> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::Stop(response_sender))
            .map_err(|error| worker_channel_error("stopping the AVFoundation stream", error))?;
        response_receiver.recv().map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("AVFoundation worker stopped while stopping the stream: {error}"),
            )
        })?
    }

    fn next_event(&mut self, timeout: Duration) -> CameraResult<CameraEvent> {
        match self.event_receiver.recv_timeout(timeout) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => Err(CameraError::new(
                CameraErrorKind::TimedOut,
                "timed out waiting for an AVFoundation camera event",
            )),
            Err(RecvTimeoutError::Disconnected) => Err(CameraError::new(
                CameraErrorKind::Disconnected,
                "AVFoundation camera worker stopped",
            )),
        }
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
        Ok(())
    }
}

impl Drop for MacAvFoundationDevice {
    fn drop(&mut self) {
        let _ = self.command_sender.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Creates the native macOS camera backend.
#[must_use]
pub fn create_backend() -> Box<dyn CameraBackend> {
    Box::new(MacAvFoundationBackend::new())
}

struct OpenedDevice {
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
}

fn open_video_device(device_id: &CameraDeviceId) -> CameraResult<MacAvFoundationDevice> {
    let requested_id = device_id.as_str().to_owned();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let (command_sender, command_receiver) = mpsc::sync_channel(4);
    let (event_sender, event_receiver) = mpsc::sync_channel(2);
    let worker = thread::Builder::new()
        .name("iriscope-avfoundation".to_owned())
        .spawn(move || {
            mac_device_worker(
                &requested_id,
                &ready_sender,
                &command_receiver,
                &event_sender,
            );
        })
        .map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("starting the AVFoundation device thread: {error}"),
            )
        })?;

    match ready_receiver.recv() {
        Ok(Ok(opened)) => Ok(MacAvFoundationDevice {
            descriptor: opened.descriptor,
            capabilities: opened.capabilities,
            command_sender,
            event_receiver,
            worker: Some(worker),
        }),
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(error)
        }
        Err(error) => {
            let _ = worker.join();
            Err(CameraError::new(
                CameraErrorKind::Backend,
                format!("AVFoundation device thread stopped while opening: {error}"),
            ))
        }
    }
}

fn mac_device_worker(
    requested_id: &str,
    ready_sender: &SyncSender<CameraResult<OpenedDevice>>,
    command_receiver: &Receiver<WorkerCommand>,
    event_sender: &SyncSender<CameraResult<CameraEvent>>,
) {
    let unique_id = NSString::from_str(requested_id);
    let Some(device) = AVCaptureDevice::device_with_unique_id(&unique_id) else {
        let _ = ready_sender.send(Err(CameraError::new(
            CameraErrorKind::DeviceNotFound,
            format!("camera {requested_id} is not available through AVFoundation"),
        )));
        return;
    };

    if !device.is_connected().is_true() {
        let _ = ready_sender.send(Err(CameraError::new(
            CameraErrorKind::DeviceNotFound,
            format!("camera {requested_id} was disconnected before it could be opened"),
        )));
        return;
    }

    let opened = OpenedDevice {
        descriptor: descriptor_from_device(&device),
        capabilities: capabilities_from_device(&device),
    };
    if ready_sender.send(Ok(opened)).is_err() {
        return;
    }

    let mut active_stream: Option<MacStream> = None;
    while let Ok(command) = command_receiver.recv() {
        match command {
            WorkerCommand::Start(configuration, response_sender) => {
                if let Some(stream) = active_stream.take() {
                    stop_mac_stream(stream);
                }
                let result = start_mac_stream(&device, &configuration, event_sender.clone());
                match result {
                    Ok(stream) => {
                        active_stream = Some(stream);
                        let _ = response_sender.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = response_sender.send(Err(error));
                    }
                }
            }
            WorkerCommand::Stop(response_sender) => {
                if let Some(stream) = active_stream.take() {
                    stop_mac_stream(stream);
                }
                let _ = response_sender.send(Ok(()));
            }
            WorkerCommand::Shutdown => break,
        }
    }

    if let Some(stream) = active_stream {
        stop_mac_stream(stream);
    }
}

struct MacStream {
    session: Retained<AVCaptureSession>,
    _input: Retained<AVCaptureDeviceInput>,
    _output: Retained<AVCaptureVideoDataOutput>,
    _delegate: Retained<FrameDelegate>,
    _queue: DispatchRetained<DispatchQueue>,
}

fn start_mac_stream(
    device: &AVCaptureDevice,
    configuration: &StreamConfiguration,
    event_sender: SyncSender<CameraResult<CameraEvent>>,
) -> CameraResult<MacStream> {
    configure_device_format(device, configuration)?;

    let input = AVCaptureDeviceInput::from_device(device).map_err(|error| {
        CameraError::new(
            CameraErrorKind::Backend,
            format!("creating AVFoundation camera input: {error:?}"),
        )
    })?;
    let output = AVCaptureVideoDataOutput::new();
    output.set_always_discards_late_video_frames(true);

    let delegate = FrameDelegate::new();
    let delegate_protocol = ProtocolObject::from_ref(&*delegate);
    let queue = DispatchQueue::new("com.iriscope.camera.frames", DispatchQueueAttr::SERIAL);
    output.set_sample_buffer_delegate(delegate_protocol, &queue);

    let session = AVCaptureSession::new();
    session.begin_configuration();
    if !session.can_add_input(&input) {
        session.commit_configuration();
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation refused the camera input",
        ));
    }
    session.add_input(&input);
    if !session.can_add_output(&output) {
        session.commit_configuration();
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation refused the video data output",
        ));
    }
    session.add_output(&output);
    session.commit_configuration();

    set_callback_state(Some(event_sender), Some(configuration.clone()));
    session.start_running();
    if !session.is_running() {
        set_callback_state(None, None);
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation capture session did not start",
        ));
    }

    Ok(MacStream {
        session,
        _input: input,
        _output: output,
        _delegate: delegate,
        _queue: queue,
    })
}

fn stop_mac_stream(stream: MacStream) {
    stream.session.stop_running();
    set_callback_state(None, None);
}

fn configure_device_format(
    device: &AVCaptureDevice,
    configuration: &StreamConfiguration,
) -> CameraResult<()> {
    let format = find_matching_format(device, configuration).ok_or_else(|| {
        CameraError::new(
            CameraErrorKind::InvalidConfiguration,
            format!(
                "AVFoundation camera does not expose {} {} at {:.3} fps",
                configuration.pixel_format,
                configuration.resolution,
                configuration.frame_rate.frames_per_second()
            ),
        )
    })?;

    device.lock_for_configuration().map_err(|error| {
        CameraError::new(
            CameraErrorKind::Backend,
            format!("locking AVFoundation camera configuration: {error:?}"),
        )
    })?;

    device.set_active_format(&format);
    let timescale = i32::try_from(configuration.frame_rate.numerator()).map_err(|_| {
        CameraError::new(
            CameraErrorKind::InvalidConfiguration,
            "requested frame-rate numerator exceeds AVFoundation CMTime range",
        )
    })?;
    let duration = CMTime::make(i64::from(configuration.frame_rate.denominator()), timescale);
    device.set_active_video_min_frame_duration(duration);
    device.set_active_video_max_frame_duration(duration);
    device.unlock_for_configuration();
    Ok(())
}

fn find_matching_format(
    device: &AVCaptureDevice,
    configuration: &StreamConfiguration,
) -> Option<Retained<AVCaptureDeviceFormat>> {
    for format in &device.formats() {
        let format_description = format.format_description();
        if format_description.get_media_type() != kCMMediaType_Video {
            continue;
        }

        let Some(video_description) =
            format_description.downcast_into::<CMVideoFormatDescription>()
        else {
            continue;
        };
        let dimensions = video_description.get_dimensions();
        let resolution = Resolution::new(
            u32::try_from(dimensions.width).ok()?,
            u32::try_from(dimensions.height).ok()?,
        );
        let pixel_format = pixel_format_from_ostype(video_description.get_codec_type());
        if resolution != configuration.resolution || pixel_format != configuration.pixel_format {
            continue;
        }

        let requested = configuration.frame_rate.frames_per_second();
        let supported = format
            .video_supported_frame_rate_ranges()
            .iter()
            .any(|range| {
                requested + 0.001 >= range.min_frame_rate()
                    && requested - 0.001 <= range.max_frame_rate()
            });
        if supported {
            return Some(format);
        }
    }
    None
}

#[derive(Default)]
struct CallbackState {
    sender: Option<SyncSender<CameraResult<CameraEvent>>>,
    configuration: Option<StreamConfiguration>,
    sequence_number: u64,
}

static CALLBACK_STATE: OnceLock<Mutex<CallbackState>> = OnceLock::new();

fn callback_state() -> &'static Mutex<CallbackState> {
    CALLBACK_STATE.get_or_init(|| Mutex::new(CallbackState::default()))
}

fn set_callback_state(
    sender: Option<SyncSender<CameraResult<CameraEvent>>>,
    configuration: Option<StreamConfiguration>,
) {
    if let Ok(mut state) = callback_state().lock() {
        state.sender = sender;
        state.configuration = configuration;
        state.sequence_number = 0;
    }
}

struct DelegateIvars {}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "IrisScopeOutputSampleBufferDelegate"]
    #[ivars = DelegateIvars]
    struct FrameDelegate;

    unsafe impl NSObjectProtocol for FrameDelegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for FrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn capture_output_did_output_sample_buffer(
            &self,
            _capture_output: &AVCaptureOutput,
            sample_buffer: CMSampleBufferRef,
            _connection: &AVCaptureConnection,
        ) {
            handle_sample_buffer(sample_buffer);
        }
    }

    impl FrameDelegate {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Option<Retained<Self>> {
            let this = this.set_ivars(DelegateIvars {});
            unsafe { msg_send![super(this), init] }
        }
    }
);

impl FrameDelegate {
    fn new() -> Retained<Self> {
        unsafe { msg_send![Self::alloc(), init] }
    }
}

fn handle_sample_buffer(sample_buffer_ref: CMSampleBufferRef) {
    if sample_buffer_ref.is_null() {
        return;
    }

    let (sender, configuration, sequence_number) = {
        let Ok(mut state) = callback_state().lock() else {
            return;
        };
        let (Some(sender), Some(configuration)) =
            (state.sender.clone(), state.configuration.clone())
        else {
            return;
        };
        let sequence_number = state.sequence_number;
        state.sequence_number = state.sequence_number.saturating_add(1);
        (sender, configuration, sequence_number)
    };

    // SAFETY: The callback owns a valid CMSampleBuffer reference for the duration of this call.
    let sample_buffer = unsafe { CMSampleBuffer::wrap_under_get_rule(sample_buffer_ref) };
    let result = frame_from_sample_buffer(&sample_buffer, &configuration, sequence_number);
    if let Some(event) = result.transpose() {
        match sender.try_send(event.map(CameraEvent::Frame)) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn frame_from_sample_buffer(
    sample_buffer: &CMSampleBuffer,
    configuration: &StreamConfiguration,
    sequence_number: u64,
) -> CameraResult<Option<CapturedFrame>> {
    let timestamp = sample_timestamp(sample_buffer);

    if let Some(data_buffer) = sample_buffer.get_data_buffer() {
        let length = data_buffer.get_data_length();
        if length != 0 {
            let mut data = vec![0_u8; length];
            data_buffer
                .copy_data_bytes(0, &mut data)
                .map_err(|status| {
                    CameraError::new(
                        CameraErrorKind::Backend,
                        format!("copying AVFoundation compressed sample bytes failed: {status}"),
                    )
                })?;

            return Ok(Some(CapturedFrame {
                sequence_number,
                timestamp,
                pixel_format: configuration.pixel_format.clone(),
                resolution: configuration.resolution,
                data: Arc::from(data),
            }));
        }
    }

    let Some(image_buffer) = sample_buffer.get_image_buffer() else {
        return Ok(None);
    };
    let Some(pixel_buffer) = image_buffer.downcast::<CVPixelBuffer>() else {
        return Ok(None);
    };
    copy_pixel_buffer(&pixel_buffer, timestamp, sequence_number)
}

fn copy_pixel_buffer(
    pixel_buffer: &CVPixelBuffer,
    timestamp: Duration,
    sequence_number: u64,
) -> CameraResult<Option<CapturedFrame>> {
    if pixel_buffer.lock_base_address(kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "locking AVFoundation pixel buffer failed",
        ));
    }

    let result = copy_locked_pixel_buffer(pixel_buffer, timestamp, sequence_number);
    let unlock_status = pixel_buffer.unlock_base_address(kCVPixelBufferLock_ReadOnly);
    if unlock_status != kCVReturnSuccess {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "unlocking AVFoundation pixel buffer failed",
        ));
    }
    result
}

fn copy_locked_pixel_buffer(
    pixel_buffer: &CVPixelBuffer,
    timestamp: Duration,
    sequence_number: u64,
) -> CameraResult<Option<CapturedFrame>> {
    let width = u32::try_from(pixel_buffer.get_width()).map_err(|_| {
        CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation frame width is too large",
        )
    })?;
    let height = u32::try_from(pixel_buffer.get_height()).map_err(|_| {
        CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation frame height is too large",
        )
    })?;
    let format = pixel_buffer.get_pixel_format();

    let (pixel_format, bytes_per_pixel, convert_uyvy) = if format == kCVPixelFormatType_32BGRA {
        (PixelFormat::Bgra8, 4_usize, false)
    } else if format == kCVPixelFormatType_422YpCbCr8_yuvs {
        (PixelFormat::Yuyv, 2_usize, false)
    } else if format == kCVPixelFormatType_422YpCbCr8 {
        (PixelFormat::Yuyv, 2_usize, true)
    } else {
        return Ok(None);
    };

    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(bytes_per_pixel))
        .ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Backend,
                "AVFoundation frame row is too large",
            )
        })?;
    let source_row_bytes = pixel_buffer.get_bytes_per_row();
    if source_row_bytes < row_bytes {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation pixel buffer stride is shorter than a visible row",
        ));
    }

    // SAFETY: The pixel buffer is locked for the duration of this function.
    let base = unsafe { pixel_buffer.get_base_address() };
    if base.is_null() {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation returned a null pixel-buffer base address",
        ));
    }

    let source_length = source_row_bytes
        .checked_mul(usize::try_from(height).unwrap_or_default())
        .ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Backend,
                "AVFoundation frame buffer is too large",
            )
        })?;
    // SAFETY: The locked pixel buffer guarantees at least stride * height readable bytes.
    let source = unsafe { slice::from_raw_parts(base.cast::<u8>(), source_length) };
    let destination_length = row_bytes
        .checked_mul(usize::try_from(height).unwrap_or_default())
        .ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Backend,
                "AVFoundation frame buffer is too large",
            )
        })?;
    let mut data = Vec::with_capacity(destination_length);

    for row in 0..usize::try_from(height).unwrap_or_default() {
        let start = row * source_row_bytes;
        let visible = &source[start..start + row_bytes];
        data.extend_from_slice(visible);
    }

    if convert_uyvy {
        for pair in data.chunks_exact_mut(4) {
            let u = pair[0];
            let y0 = pair[1];
            let v = pair[2];
            let y1 = pair[3];
            pair.copy_from_slice(&[y0, u, y1, v]);
        }
    }

    Ok(Some(CapturedFrame {
        sequence_number,
        timestamp,
        pixel_format,
        resolution: Resolution::new(width, height),
        data: Arc::from(data),
    }))
}

fn sample_timestamp(sample_buffer: &CMSampleBuffer) -> Duration {
    let seconds = sample_buffer.get_presentation_time_stamp().get_seconds();
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f64(seconds)
    } else {
        Duration::ZERO
    }
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

fn worker_channel_error<T: std::fmt::Display>(context: &str, error: T) -> CameraError {
    CameraError::new(CameraErrorKind::Backend, format!("{context}: {error}"))
}

fn controls_unavailable() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "AVFoundation camera controls are not implemented yet",
    )
}
