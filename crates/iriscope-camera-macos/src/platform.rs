use std::{
    collections::HashMap,
    slice,
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use av_foundation::{
    capture_device::{
        AVCaptureDevice, AVCaptureDeviceFormat, AVCaptureExposureModeAutoExpose,
        AVCaptureExposureModeContinuousAutoExposure, AVCaptureExposureModeCustom,
        AVCaptureExposureModeLocked, AVCaptureWhiteBalanceModeAutoWhiteBalance,
        AVCaptureWhiteBalanceModeContinuousAutoWhiteBalance, AVCaptureWhiteBalanceModeLocked,
    },
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
    CameraCapabilities, CameraControlDescriptor, CameraControlId, CameraControlKind,
    CameraControlMenuItem, CameraControlValue, CameraMode, FrameRate, PixelFormat, Resolution,
    StandardCameraControl,
};
use objc2::{
    AnyThread, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::ProtocolObject,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use crate::capabilities::{frame_rate_from_duration_parts, merge_mode, pixel_format_from_ostype};

/// Native macOS camera backend using `AVFoundation`.
#[derive(Debug, Default)]
pub struct MacAvFoundationBackend {
    known_devices: HashMap<CameraDeviceId, CameraDescriptor>,
}

impl MacAvFoundationBackend {
    /// Creates a backend. Device discovery happens during enumeration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        self.known_devices = descriptors
            .iter()
            .cloned()
            .map(|descriptor| (descriptor.id.clone(), descriptor))
            .collect();
        Ok(descriptors)
    }

    fn wait_for_device_event(
        &mut self,
        timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>> {
        let previous = self.known_devices.clone();
        thread::sleep(timeout);
        let current = self.enumerate_devices()?;

        for id in previous.keys() {
            if !self.known_devices.contains_key(id) {
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
    GetControl(
        CameraControlId,
        SyncSender<CameraResult<CameraControlValue>>,
    ),
    SetControl(
        CameraControlId,
        CameraControlValue,
        SyncSender<CameraResult<()>>,
    ),
    ResetControls(SyncSender<CameraResult<()>>),
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

    fn control_value(&self, control_id: &CameraControlId) -> CameraResult<CameraControlValue> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::GetControl(
                control_id.clone(),
                response_sender,
            ))
            .map_err(|error| worker_channel_error("reading an AVFoundation control", error))?;
        response_receiver.recv().map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("AVFoundation worker stopped while reading a control: {error}"),
            )
        })?
    }

    fn set_control_value(
        &mut self,
        control_id: &CameraControlId,
        value: &CameraControlValue,
    ) -> CameraResult<()> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::SetControl(
                control_id.clone(),
                value.clone(),
                response_sender,
            ))
            .map_err(|error| worker_channel_error("setting an AVFoundation control", error))?;
        response_receiver.recv().map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("AVFoundation worker stopped while setting a control: {error}"),
            )
        })?
    }

    fn reset_controls(&mut self) -> CameraResult<()> {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.command_sender
            .send(WorkerCommand::ResetControls(response_sender))
            .map_err(|error| worker_channel_error("resetting AVFoundation controls", error))?;
        response_receiver.recv().map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("AVFoundation worker stopped while resetting controls: {error}"),
            )
        })?
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
                    stop_mac_stream(&stream);
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
                    stop_mac_stream(&stream);
                }
                let _ = response_sender.send(Ok(()));
            }
            WorkerCommand::GetControl(control_id, response_sender) => {
                let _ = response_sender.send(mac_control_value(&device, &control_id));
            }
            WorkerCommand::SetControl(control_id, value, response_sender) => {
                let _ = response_sender.send(set_mac_control_value(&device, &control_id, &value));
            }
            WorkerCommand::ResetControls(response_sender) => {
                let _ = response_sender.send(reset_mac_controls(&device));
            }
            WorkerCommand::Shutdown => break,
        }
    }

    if let Some(stream) = active_stream {
        stop_mac_stream(&stream);
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

fn stop_mac_stream(stream: &MacStream) {
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
            Ok(()) | Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {}
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
        controls: mac_control_descriptors(device),
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

fn mac_control_descriptors(device: &AVCaptureDevice) -> Vec<CameraControlDescriptor> {
    let mut controls = Vec::new();

    let white_balance_auto_supported =
        device
            .is_white_balance_mode_supported(AVCaptureWhiteBalanceModeContinuousAutoWhiteBalance)
            .is_true()
            || device
                .is_white_balance_mode_supported(AVCaptureWhiteBalanceModeAutoWhiteBalance)
                .is_true();
    let white_balance_lock_supported = device
        .is_white_balance_mode_supported(AVCaptureWhiteBalanceModeLocked)
        .is_true();

    if white_balance_auto_supported && white_balance_lock_supported {
        controls.push(CameraControlDescriptor {
            id: CameraControlId::Standard(StandardCameraControl::WhiteBalanceAutomatic),
            name: "Balance des blancs automatique".to_owned(),
            kind: CameraControlKind::Boolean {
                default: device.white_balance_mode() != AVCaptureWhiteBalanceModeLocked,
            },
            read_only: false,
        });
    }

    let mut exposure_items = Vec::new();
    for (value, mode, label) in [
        (0_i64, AVCaptureExposureModeLocked, "Verrouillée"),
        (1_i64, AVCaptureExposureModeAutoExpose, "Automatique"),
        (
            2_i64,
            AVCaptureExposureModeContinuousAutoExposure,
            "Automatique continue",
        ),
        (3_i64, AVCaptureExposureModeCustom, "Manuelle"),
    ] {
        if device.is_exposure_mode_supported(mode).is_true() {
            exposure_items.push(CameraControlMenuItem {
                value,
                label: label.to_owned(),
            });
        }
    }

    if exposure_items.len() > 1 {
        controls.push(CameraControlDescriptor {
            id: CameraControlId::Standard(StandardCameraControl::ExposureMode),
            name: "Mode d'exposition".to_owned(),
            kind: CameraControlKind::Menu {
                items: exposure_items,
                default: mac_exposure_mode_value(device.exposure_mode()),
            },
            read_only: false,
        });
    }

    controls
}

fn mac_exposure_mode_value(mode: isize) -> i64 {
    if mode == AVCaptureExposureModeLocked {
        0
    } else if mode == AVCaptureExposureModeAutoExpose {
        1
    } else if mode == AVCaptureExposureModeContinuousAutoExposure {
        2
    } else if mode == AVCaptureExposureModeCustom {
        3
    } else {
        0
    }
}

fn exposure_mode_from_value(value: i64) -> Option<isize> {
    match value {
        0 => Some(AVCaptureExposureModeLocked),
        1 => Some(AVCaptureExposureModeAutoExpose),
        2 => Some(AVCaptureExposureModeContinuousAutoExposure),
        3 => Some(AVCaptureExposureModeCustom),
        _ => None,
    }
}

fn mac_control_value(
    device: &AVCaptureDevice,
    control_id: &CameraControlId,
) -> CameraResult<CameraControlValue> {
    match control_id {
        CameraControlId::Standard(StandardCameraControl::WhiteBalanceAutomatic) => {
            Ok(CameraControlValue::Boolean(
                device.white_balance_mode() != AVCaptureWhiteBalanceModeLocked,
            ))
        }
        CameraControlId::Standard(StandardCameraControl::ExposureMode) => Ok(
            CameraControlValue::Menu(mac_exposure_mode_value(device.exposure_mode())),
        ),
        _ => Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "AVFoundation does not expose this camera control",
        )),
    }
}

fn set_mac_control_value(
    device: &AVCaptureDevice,
    control_id: &CameraControlId,
    value: &CameraControlValue,
) -> CameraResult<()> {
    match (control_id, value) {
        (
            CameraControlId::Standard(StandardCameraControl::WhiteBalanceAutomatic),
            CameraControlValue::Boolean(automatic),
        ) => {
            let requested = if *automatic
                && device
                    .is_white_balance_mode_supported(
                        AVCaptureWhiteBalanceModeContinuousAutoWhiteBalance,
                    )
                    .is_true()
            {
                AVCaptureWhiteBalanceModeContinuousAutoWhiteBalance
            } else if *automatic {
                AVCaptureWhiteBalanceModeAutoWhiteBalance
            } else {
                AVCaptureWhiteBalanceModeLocked
            };

            if !device.is_white_balance_mode_supported(requested).is_true() {
                return Err(CameraError::new(
                    CameraErrorKind::Unsupported,
                    "requested AVFoundation white-balance mode is unavailable",
                ));
            }

            device.lock_for_configuration().map_err(|error| {
                CameraError::new(
                    CameraErrorKind::Backend,
                    format!("locking AVFoundation white-balance control: {error:?}"),
                )
            })?;
            device.set_white_balance_mode(requested);
            device.unlock_for_configuration();
            Ok(())
        }
        (
            CameraControlId::Standard(StandardCameraControl::ExposureMode),
            CameraControlValue::Menu(value),
        ) => {
            let requested = exposure_mode_from_value(*value).ok_or_else(|| {
                CameraError::new(
                    CameraErrorKind::InvalidConfiguration,
                    "unknown AVFoundation exposure mode",
                )
            })?;
            if !device.is_exposure_mode_supported(requested).is_true() {
                return Err(CameraError::new(
                    CameraErrorKind::Unsupported,
                    "requested AVFoundation exposure mode is unavailable",
                ));
            }

            device.lock_for_configuration().map_err(|error| {
                CameraError::new(
                    CameraErrorKind::Backend,
                    format!("locking AVFoundation exposure control: {error:?}"),
                )
            })?;
            device.set_exposure_mode(requested);
            device.unlock_for_configuration();
            Ok(())
        }
        _ => Err(CameraError::new(
            CameraErrorKind::InvalidConfiguration,
            "camera control value does not match the AVFoundation control",
        )),
    }
}

fn reset_mac_controls(device: &AVCaptureDevice) -> CameraResult<()> {
    let white_balance = if device
        .is_white_balance_mode_supported(AVCaptureWhiteBalanceModeContinuousAutoWhiteBalance)
        .is_true()
    {
        Some(AVCaptureWhiteBalanceModeContinuousAutoWhiteBalance)
    } else if device
        .is_white_balance_mode_supported(AVCaptureWhiteBalanceModeAutoWhiteBalance)
        .is_true()
    {
        Some(AVCaptureWhiteBalanceModeAutoWhiteBalance)
    } else {
        None
    };

    let exposure = if device
        .is_exposure_mode_supported(AVCaptureExposureModeContinuousAutoExposure)
        .is_true()
    {
        Some(AVCaptureExposureModeContinuousAutoExposure)
    } else if device
        .is_exposure_mode_supported(AVCaptureExposureModeAutoExpose)
        .is_true()
    {
        Some(AVCaptureExposureModeAutoExpose)
    } else {
        None
    };

    if white_balance.is_none() && exposure.is_none() {
        return Ok(());
    }

    device.lock_for_configuration().map_err(|error| {
        CameraError::new(
            CameraErrorKind::Backend,
            format!("locking AVFoundation controls for reset: {error:?}"),
        )
    })?;

    if let Some(mode) = white_balance {
        device.set_white_balance_mode(mode);
    }
    if let Some(mode) = exposure {
        device.set_exposure_mode(mode);
    }
    device.unlock_for_configuration();
    Ok(())
}
