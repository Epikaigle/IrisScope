use std::{
    ffi::c_void,
    ptr, slice,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
    time::Duration,
};

use iriscope_core::{
    camera::{
        CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
        CameraDeviceId, CameraError, CameraErrorKind, CameraEvent, CameraResult,
        StreamConfiguration, UsbDeviceIdentity,
    },
    capabilities::{
        CameraCapabilities, CameraControlId, CameraControlValue, CameraMode, FrameRate,
        PixelFormat, Resolution,
    },
};
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFActivate, IMFAttributes, IMFMediaSource, IMFSourceReader,
            MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_E_NO_MORE_TYPES,
            MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED, MF_MT_FRAME_RATE, MF_MT_FRAME_RATE_RANGE_MAX,
            MF_MT_FRAME_RATE_RANGE_MIN, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE,
            MF_READWRITE_DISABLE_CONVERTERS, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
            MFCreateAttributes, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources,
            MFSTARTUP_LITE, MFShutdown, MFStartup, MFVideoFormat_ARGB32, MFVideoFormat_MJPG,
            MFVideoFormat_NV12, MFVideoFormat_RGB32, MFVideoFormat_YUY2,
        },
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
    },
    core::{GUID, PWSTR},
};

/// Native Windows camera backend using Media Foundation.
#[derive(Debug, Default)]
pub struct WindowsMediaFoundationBackend;

impl WindowsMediaFoundationBackend {
    /// Creates a backend. Media Foundation is initialized on the calling thread as needed.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CameraBackend for WindowsMediaFoundationBackend {
    fn kind(&self) -> CameraBackendKind {
        CameraBackendKind::MediaFoundation
    }

    fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>> {
        enumerate_video_devices()
    }

    fn wait_for_device_event(
        &mut self,
        _timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>> {
        Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "Media Foundation hotplug monitoring is not implemented yet",
        ))
    }

    fn open(&mut self, device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        open_video_device(device_id).map(|device| Box::new(device) as Box<dyn CameraDevice>)
    }
}

struct WindowsCameraDevice {
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
    shutdown_sender: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl CameraDevice for WindowsCameraDevice {
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
        Ok(())
    }
}

impl Drop for WindowsCameraDevice {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn streaming_unavailable() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "Media Foundation streaming is not implemented yet",
    )
}

fn controls_unavailable() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "Media Foundation camera controls are not implemented yet",
    )
}

struct OpenedDevice {
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
}

struct WorkerDevice {
    source: IMFMediaSource,
    source_reader: IMFSourceReader,
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
}

fn open_video_device(device_id: &CameraDeviceId) -> CameraResult<WindowsCameraDevice> {
    let requested_id = device_id.as_str().to_owned();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let (shutdown_sender, shutdown_receiver) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("iriscope-media-foundation".to_owned())
        .spawn(move || device_worker(&requested_id, &ready_sender, &shutdown_receiver))
        .map_err(|error| {
            CameraError::new(
                CameraErrorKind::Backend,
                format!("starting the Media Foundation device thread: {error}"),
            )
        })?;

    match ready_receiver.recv() {
        Ok(Ok(opened)) => Ok(WindowsCameraDevice {
            descriptor: opened.descriptor,
            capabilities: opened.capabilities,
            shutdown_sender: Some(shutdown_sender),
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
                format!("Media Foundation device thread stopped while opening: {error}"),
            ))
        }
    }
}

fn device_worker(
    requested_id: &str,
    ready_sender: &SyncSender<CameraResult<OpenedDevice>>,
    shutdown_receiver: &Receiver<()>,
) {
    let com = match ComApartment::initialize() {
        Ok(com) => com,
        Err(error) => {
            let _ = ready_sender.send(Err(error));
            return;
        }
    };
    let media_foundation = match MediaFoundation::initialize() {
        Ok(media_foundation) => media_foundation,
        Err(error) => {
            let _ = ready_sender.send(Err(error));
            return;
        }
    };
    let worker_device = match open_on_worker(requested_id) {
        Ok(worker_device) => worker_device,
        Err(error) => {
            let _ = ready_sender.send(Err(error));
            return;
        }
    };

    let opened = OpenedDevice {
        descriptor: worker_device.descriptor.clone(),
        capabilities: worker_device.capabilities.clone(),
    };
    if ready_sender.send(Ok(opened)).is_err() {
        shutdown_worker_device(worker_device);
        return;
    }

    let _ = shutdown_receiver.recv();
    shutdown_worker_device(worker_device);
    drop(media_foundation);
    drop(com);
}

fn open_on_worker(requested_id: &str) -> CameraResult<WorkerDevice> {
    let (source, descriptor) = activate_video_device(requested_id)?;
    let source_reader = match create_source_reader(&source) {
        Ok(source_reader) => source_reader,
        Err(error) => {
            shutdown_source(&source);
            return Err(error);
        }
    };
    let capabilities = match enumerate_capabilities(&source_reader) {
        Ok(capabilities) => capabilities,
        Err(error) => {
            drop(source_reader);
            shutdown_source(&source);
            return Err(error);
        }
    };

    Ok(WorkerDevice {
        source,
        source_reader,
        descriptor,
        capabilities,
    })
}

fn shutdown_worker_device(worker_device: WorkerDevice) {
    drop(worker_device.source_reader);
    shutdown_source(&worker_device.source);
}

fn shutdown_source(source: &IMFMediaSource) {
    // SAFETY: The media source remains valid and is shut down on the worker that activated it.
    let _ = unsafe { source.Shutdown() };
}

fn activate_video_device(requested_id: &str) -> CameraResult<(IMFMediaSource, CameraDescriptor)> {
    let activation_array = video_device_activations()?;

    for activation in activation_array.as_slice().iter().flatten() {
        let symbolic_link = allocated_string(
            activation,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
        )?;
        if !symbolic_link.eq_ignore_ascii_case(requested_id) {
            continue;
        }

        let display_name = allocated_string(activation, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)?;
        // SAFETY: The activation object came from `MFEnumDeviceSources` and is used on the
        // initialized Media Foundation worker thread.
        let source = unsafe { activation.ActivateObject::<IMFMediaSource>() }
            .map_err(|error| windows_device_error("activating the camera", &error))?;
        let descriptor = CameraDescriptor {
            id: CameraDeviceId::new(symbolic_link.clone()),
            display_name,
            backend: CameraBackendKind::MediaFoundation,
            usb: parse_usb_identity(&symbolic_link),
        };

        return Ok((source, descriptor));
    }

    Err(CameraError::new(
        CameraErrorKind::DeviceNotFound,
        format!("Media Foundation camera {requested_id} is no longer available"),
    ))
}

fn create_source_reader(source: &IMFMediaSource) -> CameraResult<IMFSourceReader> {
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: `attributes` points to valid storage for the returned COM interface.
    unsafe { MFCreateAttributes(&raw mut attributes, 1) }
        .map_err(|error| windows_error("creating source reader attributes", &error))?;
    let attributes = attributes.ok_or_else(|| {
        CameraError::new(
            CameraErrorKind::Backend,
            "Media Foundation returned no source reader attribute store",
        )
    })?;

    // SAFETY: The attribute store is valid. Disabling converters makes every enumerated type a
    // type delivered natively by the camera rather than a software-generated conversion.
    unsafe { attributes.SetUINT32(&MF_READWRITE_DISABLE_CONVERTERS, 1) }
        .map_err(|error| windows_error("disabling Media Foundation format converters", &error))?;

    // SAFETY: The source and attributes are valid on the Media Foundation worker thread.
    unsafe { MFCreateSourceReaderFromMediaSource(source, &attributes) }
        .map_err(|error| windows_device_error("creating the camera source reader", &error))
}

fn enumerate_capabilities(source_reader: &IMFSourceReader) -> CameraResult<CameraCapabilities> {
    let mut modes = Vec::new();
    let stream_index = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0.cast_unsigned();
    let mut media_type_index = 0;

    loop {
        // SAFETY: The source reader is valid and indices are advanced until Media Foundation
        // reports `MF_E_NO_MORE_TYPES`.
        let media_type =
            match unsafe { source_reader.GetNativeMediaType(stream_index, media_type_index) } {
                Ok(media_type) => media_type,
                Err(error) if error.code() == MF_E_NO_MORE_TYPES => break,
                Err(error) => {
                    return Err(windows_device_error(
                        "enumerating native camera media types",
                        &error,
                    ));
                }
            };
        media_type_index += 1;

        // SAFETY: The returned native media type is an `IMFAttributes` implementation.
        let subtype = unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }
            .map_err(|error| windows_error("reading a native camera pixel format", &error))?;
        // SAFETY: `MF_MT_FRAME_SIZE` is a packed width/height pair on a video media type.
        let packed_size = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|error| windows_error("reading a native camera frame size", &error))?;
        let resolution = unpack_resolution(packed_size).ok_or_else(|| {
            CameraError::new(
                CameraErrorKind::Backend,
                "Media Foundation reported a zero-sized native camera mode",
            )
        })?;

        let mut frame_rates = Vec::new();
        for key in [
            &MF_MT_FRAME_RATE,
            &MF_MT_FRAME_RATE_RANGE_MIN,
            &MF_MT_FRAME_RATE_RANGE_MAX,
        ] {
            // SAFETY: Frame-rate attributes are packed numerator/denominator pairs. Not every
            // camera supplies the optional range attributes, so missing values are ignored.
            if let Ok(packed_rate) = unsafe { media_type.GetUINT64(key) }
                && let Some(frame_rate) = unpack_frame_rate(packed_rate)
                && !frame_rates.contains(&frame_rate)
            {
                frame_rates.push(frame_rate);
            }
        }
        if frame_rates.is_empty() {
            continue;
        }
        sort_frame_rates(&mut frame_rates);

        merge_mode(
            &mut modes,
            pixel_format_from_subtype(subtype),
            resolution,
            frame_rates,
        );
    }

    if modes.is_empty() {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "Media Foundation reported no usable native video modes for this camera",
        ));
    }

    Ok(CameraCapabilities {
        modes,
        controls: Vec::new(),
    })
}

fn unpack_resolution(packed: u64) -> Option<Resolution> {
    let width = u32::try_from(packed >> 32).ok()?;
    let height = u32::try_from(packed & u64::from(u32::MAX)).ok()?;
    (width != 0 && height != 0).then(|| Resolution::new(width, height))
}

fn unpack_frame_rate(packed: u64) -> Option<FrameRate> {
    let numerator = u32::try_from(packed >> 32).ok()?;
    let denominator = u32::try_from(packed & u64::from(u32::MAX)).ok()?;
    if numerator == 0 || denominator == 0 {
        return None;
    }
    let divisor = greatest_common_divisor(numerator, denominator);
    FrameRate::new(numerator / divisor, denominator / divisor)
}

const fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn sort_frame_rates(frame_rates: &mut [FrameRate]) {
    frame_rates.sort_by(|left, right| {
        let left_scaled = u64::from(left.numerator()) * u64::from(right.denominator());
        let right_scaled = u64::from(right.numerator()) * u64::from(left.denominator());
        left_scaled.cmp(&right_scaled)
    });
}

fn merge_mode(
    modes: &mut Vec<CameraMode>,
    pixel_format: PixelFormat,
    resolution: Resolution,
    frame_rates: Vec<FrameRate>,
) {
    if let Some(mode) = modes
        .iter_mut()
        .find(|mode| mode.pixel_format == pixel_format && mode.resolution == resolution)
    {
        for frame_rate in frame_rates {
            if !mode.frame_rates.contains(&frame_rate) {
                mode.frame_rates.push(frame_rate);
            }
        }
        sort_frame_rates(&mut mode.frame_rates);
    } else {
        modes.push(CameraMode {
            pixel_format,
            resolution,
            frame_rates,
        });
    }
}

fn pixel_format_from_subtype(subtype: GUID) -> PixelFormat {
    if subtype == MFVideoFormat_MJPG {
        PixelFormat::Mjpeg
    } else if subtype == MFVideoFormat_YUY2 {
        PixelFormat::Yuyv
    } else if subtype == MFVideoFormat_NV12 {
        PixelFormat::Nv12
    } else if subtype == MFVideoFormat_ARGB32 || subtype == MFVideoFormat_RGB32 {
        PixelFormat::Bgra8
    } else {
        PixelFormat::Other(format!("Media Foundation {subtype:?}"))
    }
}

/// Creates the native Windows camera backend.
#[must_use]
pub fn create_backend() -> Box<dyn CameraBackend> {
    Box::new(WindowsMediaFoundationBackend::new())
}

fn enumerate_video_devices() -> CameraResult<Vec<CameraDescriptor>> {
    let _com = ComApartment::initialize()?;
    let _media_foundation = MediaFoundation::initialize()?;
    let activation_array = video_device_activations()?;

    let mut descriptors = Vec::with_capacity(activation_array.len());
    for activation in activation_array.as_slice().iter().flatten() {
        let display_name = allocated_string(activation, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)?;
        let symbolic_link = allocated_string(
            activation,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
        )?;

        descriptors.push(CameraDescriptor {
            id: CameraDeviceId::new(symbolic_link.clone()),
            display_name,
            backend: CameraBackendKind::MediaFoundation,
            usb: parse_usb_identity(&symbolic_link),
        });
    }

    descriptors.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    Ok(descriptors)
}

fn video_device_activations() -> CameraResult<ActivationArray> {
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: `attributes` points to valid storage for the returned COM interface.
    unsafe { MFCreateAttributes(&raw mut attributes, 1) }
        .map_err(|error| windows_error("creating Media Foundation attributes", &error))?;
    let attributes = attributes.ok_or_else(|| {
        CameraError::new(
            CameraErrorKind::Backend,
            "Media Foundation returned no attribute store",
        )
    })?;

    // SAFETY: The attribute store is valid and both GUID pointers live for this call.
    unsafe {
        attributes.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )
    }
    .map_err(|error| windows_error("configuring video device enumeration", &error))?;

    let mut raw_devices: *mut Option<IMFActivate> = ptr::null_mut();
    let mut device_count = 0;
    // SAFETY: Both output pointers are valid. Media Foundation allocates the returned array.
    unsafe { MFEnumDeviceSources(&attributes, &raw mut raw_devices, &raw mut device_count) }
        .map_err(|error| windows_error("enumerating Media Foundation cameras", &error))?;

    Ok(ActivationArray::new(
        raw_devices,
        usize::try_from(device_count).expect("a u32 device count fits in usize on Windows"),
    ))
}

fn allocated_string(activation: &IMFActivate, key: &GUID) -> CameraResult<String> {
    let mut value = PWSTR(ptr::null_mut());
    let mut length = 0;
    // SAFETY: Both output pointers are valid. The returned string is freed below with
    // `CoTaskMemFree`, as required by `GetAllocatedString`.
    let result = unsafe { activation.GetAllocatedString(key, &raw mut value, &raw mut length) };
    if let Err(error) = result {
        free_task_memory(value.0.cast());
        return Err(windows_error(
            "reading a Media Foundation camera property",
            &error,
        ));
    }
    if value.is_null() {
        return Err(CameraError::new(
            CameraErrorKind::Backend,
            "Media Foundation returned an empty camera property",
        ));
    }

    // SAFETY: Media Foundation reports `length` UTF-16 code units at `value`.
    let text = String::from_utf16_lossy(unsafe {
        slice::from_raw_parts(value.0.cast_const(), length as usize)
    });
    free_task_memory(value.0.cast());
    Ok(text)
}

fn free_task_memory(pointer: *const c_void) {
    if !pointer.is_null() {
        // SAFETY: The pointer was allocated by a COM API requiring `CoTaskMemFree`.
        unsafe { CoTaskMemFree(Some(pointer)) };
    }
}

fn parse_usb_identity(symbolic_link: &str) -> Option<UsbDeviceIdentity> {
    let lowercase = symbolic_link.to_ascii_lowercase();
    let vendor_id = parse_hex_marker(&lowercase, "vid_")?;
    let product_id = parse_hex_marker(&lowercase, "pid_")?;
    let serial_number = symbolic_link
        .split('#')
        .nth(2)
        .filter(|value| !value.is_empty() && !value.contains('&'))
        .map(str::to_owned);

    Some(UsbDeviceIdentity {
        vendor_id,
        product_id,
        serial_number,
        hardware_revision: None,
    })
}

fn parse_hex_marker(value: &str, marker: &str) -> Option<u16> {
    let start = value.find(marker)? + marker.len();
    let digits = value.get(start..start + 4)?;
    u16::from_str_radix(digits, 16).ok()
}

fn windows_error(context: &str, error: &windows::core::Error) -> CameraError {
    CameraError::new(CameraErrorKind::Backend, format!("{context}: {error}"))
        .with_platform_code(i64::from(error.code().0))
}

fn windows_device_error(context: &str, error: &windows::core::Error) -> CameraError {
    const ACCESS_DENIED: windows::core::HRESULT = windows::core::HRESULT(-2_147_024_891);
    const SHARING_VIOLATION: windows::core::HRESULT = windows::core::HRESULT(-2_147_024_864);
    const BUSY: windows::core::HRESULT = windows::core::HRESULT(-2_147_024_726);

    let kind = match error.code() {
        ACCESS_DENIED => CameraErrorKind::PermissionDenied,
        SHARING_VIOLATION | BUSY => CameraErrorKind::DeviceBusy,
        MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED => CameraErrorKind::Disconnected,
        _ => CameraErrorKind::Backend,
    };
    CameraError::new(kind, format!("{context}: {error}"))
        .with_platform_code(i64::from(error.code().0))
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> CameraResult<Self> {
        // SAFETY: COM is initialized and uninitialized on the same thread by this guard.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        result
            .ok()
            .map_err(|error| windows_error("initializing COM", &error))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: This balances the successful `CoInitializeEx` call on this thread.
        unsafe { CoUninitialize() };
    }
}

struct MediaFoundation;

impl MediaFoundation {
    fn initialize() -> CameraResult<Self> {
        // SAFETY: COM is initialized and this call is balanced by the guard's drop.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }
            .map_err(|error| windows_error("starting Media Foundation", &error))?;
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        // SAFETY: This balances the successful `MFStartup` call on this thread.
        let _ = unsafe { MFShutdown() };
    }
}

struct ActivationArray {
    pointer: *mut Option<IMFActivate>,
    length: usize,
}

impl ActivationArray {
    const fn new(pointer: *mut Option<IMFActivate>, length: usize) -> Self {
        Self { pointer, length }
    }

    fn as_slice(&self) -> &[Option<IMFActivate>] {
        if self.pointer.is_null() || self.length == 0 {
            &[]
        } else {
            // SAFETY: Media Foundation returned an array containing `length` entries.
            unsafe { slice::from_raw_parts(self.pointer, self.length) }
        }
    }

    const fn len(&self) -> usize {
        self.length
    }
}

impl Drop for ActivationArray {
    fn drop(&mut self) {
        if self.pointer.is_null() {
            return;
        }

        // SAFETY: Every array entry owns one COM reference and must be dropped before the
        // COM-allocated backing array is released.
        unsafe {
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(self.pointer, self.length));
        }
        free_task_memory(self.pointer.cast());
    }
}

#[cfg(test)]
mod tests {
    use iriscope_core::capabilities::{FrameRate, PixelFormat, Resolution};

    use super::{merge_mode, parse_usb_identity, unpack_frame_rate, unpack_resolution};

    #[test]
    fn extracts_usb_identity_from_media_foundation_link() {
        let identity = parse_usb_identity(
            r"\\?\usb#vid_21cd&pid_603b&mi_00#VTU603EB#{e5323777-f976-4f5b-9b55-b94699c46e44}",
        )
        .expect("the symbolic link contains a USB identity");

        assert_eq!(identity.vendor_id, 0x21cd);
        assert_eq!(identity.product_id, 0x603b);
        assert_eq!(identity.serial_number.as_deref(), Some("VTU603EB"));
    }

    #[test]
    fn unpacks_media_foundation_attribute_pairs() {
        let packed_size = (u64::from(1_280_u32) << 32) | u64::from(1_024_u32);
        let packed_rate = (u64::from(30_000_u32) << 32) | u64::from(1_001_u32);

        assert_eq!(
            unpack_resolution(packed_size),
            Some(Resolution::new(1280, 1024))
        );
        let frame_rate = unpack_frame_rate(packed_rate).expect("the frame rate is valid");
        assert_eq!(frame_rate.numerator(), 30_000);
        assert_eq!(frame_rate.denominator(), 1_001);
        assert!(unpack_frame_rate(1).is_none());
    }

    #[test]
    fn combines_rates_for_the_same_native_mode() {
        let mut modes = Vec::new();
        let resolution = Resolution::new(640, 480);
        let rate_30 = FrameRate::new(30, 1).expect("valid rate");
        let rate_15 = FrameRate::new(15, 1).expect("valid rate");

        merge_mode(&mut modes, PixelFormat::Mjpeg, resolution, vec![rate_30]);
        merge_mode(
            &mut modes,
            PixelFormat::Mjpeg,
            resolution,
            vec![rate_15, rate_30],
        );

        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].frame_rates, vec![rate_15, rate_30]);
    }
}
