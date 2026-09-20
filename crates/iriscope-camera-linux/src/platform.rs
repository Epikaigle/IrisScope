use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use iriscope_core::camera::{
    CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
    CameraDeviceId, CameraError, CameraErrorKind, CameraResult, UsbDeviceIdentity,
};
use v4l::{
    Device,
    capability::Flags,
    context::{self, Node},
};

/// Native Linux camera backend using `V4L2` device nodes.
#[derive(Debug, Default)]
pub struct LinuxV4l2Backend {
    device_paths: HashMap<CameraDeviceId, PathBuf>,
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
        let mut first_error = None;

        for node in context::enum_devices() {
            match inspect_node(&node) {
                Ok(Some(descriptor)) => {
                    device_paths.insert(descriptor.id.clone(), node.path().to_path_buf());
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
        Ok(descriptors)
    }

    fn wait_for_device_event(
        &mut self,
        _timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>> {
        Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "V4L2 hotplug monitoring is not implemented yet",
        ))
    }

    fn open(&mut self, device_id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        if !self.device_paths.contains_key(device_id) {
            return Err(CameraError::new(
                CameraErrorKind::DeviceNotFound,
                format!("camera {device_id} is not present in the latest enumeration"),
            ));
        }

        Err(CameraError::new(
            CameraErrorKind::Unsupported,
            "V4L2 streaming is not implemented yet",
        ))
    }
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
    use iriscope_core::camera::UsbDeviceIdentity;

    use super::{format_bcd_revision, stable_device_id};

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
}
