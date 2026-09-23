use std::{
    collections::HashMap,
    slice,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
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
        CVPixelBuffer, kCVPixelBufferLock_ReadOnly, kCVPixelBufferPixelFormatTypeKey,
        kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_422YpCbCr8,
        kCVPixelFormatType_422YpCbCr8_yuvs,
    },
    r#return::kCVReturnSuccess,
};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
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
use objc2::{
    AnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::ProtocolObject,
};
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};

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
    events: Arc<EventMailbox>,
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
        self.events.next_event(timeout)
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

#[derive(Default)]
struct EventMailbox {
    state: Mutex<EventMailboxState>,
    available: Condvar,
}

#[derive(Default)]
struct EventMailboxState {
    pending: Option<CameraResult<CameraEvent>>,
    closed: bool,
}

impl EventMailbox {
    fn publish(&self, event: CameraResult<CameraEvent>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        if matches!(&event, Ok(CameraEvent::Frame(_)))
            && state
                .pending
                .as_ref()
                .is_some_and(|pending| !matches!(pending, Ok(CameraEvent::Frame(_))))
        {
            return;
        }
        state.pending = Some(event);
        self.available.notify_one();
    }

    fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending = None;
    }

    fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        self.available.notify_all();
    }

    fn next_event(&self, timeout: Duration) -> CameraResult<CameraEvent> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        loop {
            if let Some(event) = state.pending.take() {
                return event;
            }
            if state.closed {
                return Err(CameraError::new(
                    CameraErrorKind::Disconnected,
                    "AVFoundation camera worker stopped",
                ));
            }

            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(CameraError::new(
                    CameraErrorKind::TimedOut,
                    "timed out waiting for an AVFoundation camera event",
                ));
            }

            let waited = self
                .available
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = waited.0;
            if waited.1.timed_out() && state.pending.is_none() {
                return Err(CameraError::new(
                    CameraErrorKind::TimedOut,
                    "timed out waiting for an AVFoundation camera event",
                ));
            }
        }
    }
}

fn open_video_device(device_id: &CameraDeviceId) -> CameraResult<MacAvFoundationDevice> {
    let requested_id = device_id.as_str().to_owned();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let (command_sender, command_receiver) = mpsc::sync_channel(4);
    let events = Arc::new(EventMailbox::default());
    let worker_events = Arc::clone(&events);
    let worker = thread::Builder::new()
        .name("iriscope-avfoundation".to_owned())
        .spawn(move || {
            mac_device_worker(
                &requested_id,
                &ready_sender,
                &command_receiver,
                &worker_events,
            );
            worker_events.close();
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
            events,
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
    events: &Arc<EventMailbox>,
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
                events.clear();
                let result = start_mac_stream(&device, &configuration, Arc::clone(events));
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
                events.clear();
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
    events: Arc<EventMailbox>,
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

    let delegate = FrameDelegate::new(events, configuration.clone());
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
    if let Err(error) = configure_safe_output_format(&output) {
        session.commit_configuration();
        return Err(error);
    }
    session.commit_configuration();

    session.start_running();
    if !session.is_running() {
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
}

fn configure_safe_output_format(output: &AVCaptureVideoDataOutput) -> CameraResult<()> {
    let available = output
        .get_available_video_cv_pixel_format_types()
        .iter()
        .map(|number| number.as_u32())
        .collect::<Vec<_>>();
    let selected = select_safe_output_pixel_format(&available).ok_or_else(|| {
        CameraError::new(
            CameraErrorKind::Unsupported,
            "AVFoundation offers no full-range NV12 or BGRA camera output",
        )
    })?;

    // SAFETY: CoreFoundation strings and NSString are toll-free bridged, and this key is static.
    let key = unsafe { &*kCVPixelBufferPixelFormatTypeKey.cast::<NSString>() };
    let value = NSNumber::new_u32(selected);
    let settings = NSDictionary::<NSString, NSObject>::from_slices(&[key], &[&*value]);
    output.set_video_settings(&settings);
    Ok(())
}

fn select_safe_output_pixel_format(available: &[u32]) -> Option<u32> {
    [
        kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        kCVPixelFormatType_32BGRA,
    ]
    .into_iter()
    .find(|format| available.contains(format))
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

    let timescale = i32::try_from(configuration.frame_rate.numerator()).map_err(|_| {
        CameraError::new(
            CameraErrorKind::InvalidConfiguration,
            "requested frame-rate numerator exceeds AVFoundation CMTime range",
        )
    })?;
    let duration = CMTime::make(i64::from(configuration.frame_rate.denominator()), timescale);

    device.lock_for_configuration().map_err(|error| {
        CameraError::new(
            CameraErrorKind::Backend,
            format!("locking AVFoundation camera configuration: {error:?}"),
        )
    })?;

    device.set_active_format(&format);
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

struct CallbackState {
    events: Arc<EventMailbox>,
    configuration: StreamConfiguration,
    sequence_number: u64,
    initial_timestamp: Option<Duration>,
}

struct DelegateIvars {
    callback: Mutex<CallbackState>,
}

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
            handle_sample_buffer(sample_buffer, &self.ivars().callback);
        }
    }
);

impl FrameDelegate {
    fn new(events: Arc<EventMailbox>, configuration: StreamConfiguration) -> Retained<Self> {
        let allocated = Self::alloc().set_ivars(DelegateIvars {
            callback: Mutex::new(CallbackState {
                events,
                configuration,
                sequence_number: 0,
                initial_timestamp: None,
            }),
        });
        unsafe { msg_send![super(allocated), init] }
    }
}

fn handle_sample_buffer(sample_buffer_ref: CMSampleBufferRef, callback: &Mutex<CallbackState>) {
    if sample_buffer_ref.is_null() {
        return;
    }

    // SAFETY: The callback owns a valid CMSampleBuffer reference for the duration of this call.
    let sample_buffer = unsafe { CMSampleBuffer::wrap_under_get_rule(sample_buffer_ref) };
    let source_timestamp = sample_timestamp(&sample_buffer);

    let (events, configuration, sequence_number, timestamp) = {
        let Ok(mut state) = callback.lock() else {
            return;
        };
        let events = Arc::clone(&state.events);
        let configuration = state.configuration.clone();
        let sequence_number = state.sequence_number;
        state.sequence_number = state.sequence_number.saturating_add(1);
        let timestamp = source_timestamp.map_or(Duration::ZERO, |timestamp| {
            elapsed_timestamp(timestamp, &mut state.initial_timestamp)
        });
        (events, configuration, sequence_number, timestamp)
    };

    let result =
        frame_from_sample_buffer(&sample_buffer, &configuration, sequence_number, timestamp);
    if let Some(event) = result.transpose() {
        events.publish(event.map(CameraEvent::Frame));
    }
}

fn frame_from_sample_buffer(
    sample_buffer: &CMSampleBuffer,
    configuration: &StreamConfiguration,
    sequence_number: u64,
    timestamp: Duration,
) -> CameraResult<Option<CapturedFrame>> {
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

    if format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange {
        return Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "AVFoundation delivered video-range NV12 despite requesting full-range output",
        ));
    }

    if format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange {
        return copy_locked_nv12_pixel_buffer(
            pixel_buffer,
            Resolution::new(width, height),
            timestamp,
            sequence_number,
        )
        .map(Some);
    }

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

fn copy_locked_nv12_pixel_buffer(
    pixel_buffer: &CVPixelBuffer,
    resolution: Resolution,
    timestamp: Duration,
    sequence_number: u64,
) -> CameraResult<CapturedFrame> {
    if pixel_buffer.get_plane_count() < 2 {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation NV12 buffer does not contain two planes",
        ));
    }

    let width = usize::try_from(resolution.width).map_err(|_| {
        CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation NV12 frame width is too large",
        )
    })?;
    let height = usize::try_from(resolution.height).map_err(|_| {
        CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation NV12 frame height is too large",
        )
    })?;
    let chroma_height = height.div_ceil(2);
    let chroma_row_bytes = width.div_ceil(2).checked_mul(2).ok_or_else(|| {
        CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation NV12 chroma row is too large",
        )
    })?;
    let capacity = width
        .checked_mul(height)
        .and_then(|luma| {
            chroma_row_bytes
                .checked_mul(chroma_height)
                .and_then(|chroma| luma.checked_add(chroma))
        })
        .ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Backend,
                "AVFoundation NV12 frame buffer is too large",
            )
        })?;
    let mut data = Vec::with_capacity(capacity);

    copy_pixel_buffer_plane(pixel_buffer, 0, width, height, &mut data)?;
    copy_pixel_buffer_plane(pixel_buffer, 1, chroma_row_bytes, chroma_height, &mut data)?;

    Ok(CapturedFrame {
        sequence_number,
        timestamp,
        pixel_format: PixelFormat::Nv12,
        resolution,
        data: Arc::from(data),
    })
}

fn copy_pixel_buffer_plane(
    pixel_buffer: &CVPixelBuffer,
    plane_index: usize,
    visible_row_bytes: usize,
    visible_rows: usize,
    destination: &mut Vec<u8>,
) -> CameraResult<()> {
    let source_row_bytes = pixel_buffer.get_bytes_per_row_of_plane(plane_index);
    let source_rows = pixel_buffer.get_height_of_plane(plane_index);
    if source_row_bytes < visible_row_bytes || source_rows < visible_rows {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation NV12 plane is smaller than the visible image",
        ));
    }

    // SAFETY: The pixel buffer is locked, and the plane remains valid for this function.
    let base = unsafe { pixel_buffer.get_base_address_of_plane(plane_index) };
    if base.is_null() {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation returned a null NV12 plane address",
        ));
    }
    let source_length = source_row_bytes.checked_mul(source_rows).ok_or_else(|| {
        CameraError::new(
            CameraErrorKind::Backend,
            "AVFoundation NV12 plane is too large",
        )
    })?;
    // SAFETY: The locked plane exposes at least stride * plane height readable bytes.
    let source = unsafe { slice::from_raw_parts(base.cast::<u8>(), source_length) };
    for row in 0..visible_rows {
        let start = row * source_row_bytes;
        destination.extend_from_slice(&source[start..start + visible_row_bytes]);
    }
    Ok(())
}

fn sample_timestamp(sample_buffer: &CMSampleBuffer) -> Option<Duration> {
    let seconds = sample_buffer.get_presentation_time_stamp().get_seconds();
    if (0.0..18_446_744_073_709_551_616.0).contains(&seconds) {
        Some(Duration::from_secs_f64(seconds))
    } else {
        None
    }
}

fn elapsed_timestamp(timestamp: Duration, initial_timestamp: &mut Option<Duration>) -> Duration {
    let initial = *initial_timestamp.get_or_insert(timestamp);
    timestamp.saturating_sub(initial)
}

fn descriptor_from_device(device: &AVCaptureDevice) -> CameraDescriptor {
    let unique_id = device.unique_id().to_string();

    CameraDescriptor {
        id: CameraDeviceId::new(unique_id.clone()),
        display_name: device.localized_name().to_string(),
        backend: CameraBackendKind::AvFoundation,
        usb: parse_usb_identity(&unique_id),
    }
}

fn parse_usb_identity(unique_id: &str) -> Option<UsbDeviceIdentity> {
    let hexadecimal = unique_id
        .strip_prefix("0x")
        .or_else(|| unique_id.strip_prefix("0X"))?;
    let value = u64::from_str_radix(hexadecimal, 16).ok()?;
    let vendor_id = u16::try_from((value >> 16) & 0xffff).ok()?;
    let product_id = u16::try_from(value & 0xffff).ok()?;

    if vendor_id == 0 || product_id == 0 {
        return None;
    }

    Some(UsbDeviceIdentity {
        vendor_id,
        product_id,
        serial_number: None,
        hardware_revision: None,
    })
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

    let white_balance_auto_supported = device
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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use iriscope_core::{
        camera::{CameraErrorKind, CameraEvent, CapturedFrame},
        capabilities::{PixelFormat, Resolution},
    };

    use core_video::pixel_buffer::{
        kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    };

    use super::{
        EventMailbox, elapsed_timestamp, parse_usb_identity, select_safe_output_pixel_format,
    };

    #[test]
    fn extracts_usb_identity_from_avfoundation_uvc_unique_id() {
        let identity = parse_usb_identity("0x1420000021cd603b")
            .expect("the AVFoundation UVC unique ID contains VID and PID");

        assert_eq!(identity.vendor_id, 0x21cd);
        assert_eq!(identity.product_id, 0x603b);
    }

    #[test]
    fn ignores_non_uvc_avfoundation_unique_id() {
        assert!(parse_usb_identity("FaceTime HD Camera").is_none());
        assert!(parse_usb_identity("0x0000000000000000").is_none());
    }

    #[test]
    fn normalizes_presentation_timestamps_to_stream_elapsed_time() {
        let mut initial = None;

        assert_eq!(
            elapsed_timestamp(Duration::from_secs(42), &mut initial),
            Duration::ZERO
        );
        assert_eq!(
            elapsed_timestamp(Duration::from_millis(42_033), &mut initial),
            Duration::from_millis(33)
        );
    }

    #[test]
    fn keeps_latest_frame_and_preserves_conversion_error() {
        let mailbox = EventMailbox::default();
        mailbox.publish(Ok(CameraEvent::Frame(frame(1))));
        mailbox.publish(Ok(CameraEvent::Frame(frame(2))));

        let CameraEvent::Frame(latest) = mailbox
            .next_event(Duration::ZERO)
            .expect("latest frame should be ready")
        else {
            panic!("expected a frame event");
        };
        assert_eq!(latest.sequence_number, 2);

        mailbox.publish(Err(iriscope_core::camera::CameraError::new(
            CameraErrorKind::Backend,
            "conversion failed",
        )));
        mailbox.publish(Ok(CameraEvent::Frame(frame(3))));
        assert_eq!(
            mailbox
                .next_event(Duration::ZERO)
                .expect_err("a pending error must not be hidden by a newer frame")
                .kind(),
            CameraErrorKind::Backend
        );
    }

    #[test]
    fn clearing_mailbox_discards_previous_stream_frame() {
        let mailbox = EventMailbox::default();
        mailbox.publish(Ok(CameraEvent::Frame(frame(1))));
        mailbox.clear();

        assert_eq!(
            mailbox
                .next_event(Duration::ZERO)
                .expect_err("a previous stream frame must not remain pending")
                .kind(),
            CameraErrorKind::TimedOut
        );
    }

    #[test]
    fn avoids_video_range_nv12_output() {
        let video_range = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;
        let full_range = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;
        let bgra = kCVPixelFormatType_32BGRA;

        assert_eq!(select_safe_output_pixel_format(&[video_range]), None);
        assert_eq!(
            select_safe_output_pixel_format(&[video_range, bgra]),
            Some(bgra)
        );
        assert_eq!(
            select_safe_output_pixel_format(&[video_range, bgra, full_range]),
            Some(full_range)
        );
    }

    fn frame(sequence_number: u64) -> CapturedFrame {
        CapturedFrame {
            sequence_number,
            timestamp: Duration::ZERO,
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(640, 480),
            data: Arc::from([]),
        }
    }
}
