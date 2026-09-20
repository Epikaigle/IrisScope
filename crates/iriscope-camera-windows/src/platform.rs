use std::{ffi::c_void, ptr, slice, time::Duration};

use iriscope_core::camera::{
    CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
    CameraDeviceId, CameraError, CameraErrorKind, CameraResult, UsbDeviceIdentity,
};
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFActivate, IMFAttributes, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
            MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_VERSION,
            MFCreateAttributes, MFEnumDeviceSources, MFSTARTUP_LITE, MFShutdown, MFStartup,
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

    fn open(&mut self, _device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "Media Foundation streaming is not implemented yet",
        ))
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
    let activation_array = ActivationArray::new(raw_devices, device_count as usize);

    let mut descriptors = Vec::with_capacity(device_count as usize);
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
    use super::parse_usb_identity;

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
}
