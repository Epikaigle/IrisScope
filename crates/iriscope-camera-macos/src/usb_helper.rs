//! Connection to the DE400 video, button and controls helper authorized by the Mac launcher.
//! The GUI stays unprivileged; this backend does not install drivers or services.

use crate::{button_protocol::ButtonProtocol, events::EventMailbox, usb_controls};
use iriscope_core::{
    camera::{
        CameraBackend, CameraBackendKind, CameraDescriptor, CameraDevice, CameraDeviceEvent,
        CameraDeviceId, CameraError, CameraErrorKind, CameraEvent, CameraResult, CapturedFrame,
        StreamConfiguration, UsbDeviceIdentity,
    },
    capabilities::{
        CameraCapabilities, CameraControlId, CameraControlKind, CameraControlValue, CameraMode,
        FrameRate, PixelFormat, Resolution, StandardCameraControl,
    },
};
use std::{
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const FRAME_BYTES: usize = 1280 * 1024 * 2;
const HELLO: [u32; 4] = [2, 1280, 1024, 8];

pub(super) struct UsbHelperBackend {
    socket: PathBuf,
}
impl UsbHelperBackend {
    pub(super) fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
}
fn descriptor() -> CameraDescriptor {
    CameraDescriptor {
        id: CameraDeviceId::new("usb:21cd:603b"),
        display_name: "DE400 — vidéo et bouton USB".to_owned(),
        backend: CameraBackendKind::Usb,
        usb: Some(UsbDeviceIdentity {
            vendor_id: 0x21cd,
            product_id: 0x603b,
            serial_number: None,
            hardware_revision: None,
        }),
    }
}
fn configuration() -> StreamConfiguration {
    StreamConfiguration {
        pixel_format: PixelFormat::Yuyv,
        resolution: Resolution::new(1280, 1024),
        frame_rate: FrameRate::new(8, 1).expect("nonzero fixed rate"),
    }
}
fn error(context: &str, error: impl std::fmt::Display) -> CameraError {
    CameraError::new(
        CameraErrorKind::BackendUnavailable,
        format!("DE400 USB: {context}: {error}"),
    )
}
impl CameraBackend for UsbHelperBackend {
    fn kind(&self) -> CameraBackendKind {
        CameraBackendKind::Usb
    }
    fn enumerate_devices(&mut self) -> CameraResult<Vec<CameraDescriptor>> {
        if !self.socket.is_absolute() {
            return Err(error("socket", "absolute path required"));
        }
        Ok(if self.socket.exists() {
            vec![descriptor()]
        } else {
            vec![]
        })
    }
    fn wait_for_device_event(
        &mut self,
        timeout: Duration,
    ) -> CameraResult<Option<CameraDeviceEvent>> {
        thread::sleep(timeout);
        Ok((!self.socket.exists()).then(|| CameraDeviceEvent::Disconnected(descriptor().id)))
    }
    fn open(&mut self, id: &CameraDeviceId) -> CameraResult<Box<dyn CameraDevice>> {
        if id != &descriptor().id {
            return Err(error("identity", "unexpected camera"));
        }
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| error("connect", e))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| error("timeout", e))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| error("timeout", e))?;
        let stop = AtomicBool::new(false);
        let (kind, hello) = read_packet(&mut stream, &stop).map_err(|e| error("handshake", e))?;
        if kind == 3 {
            return Err(error("open", String::from_utf8_lossy(&hello)));
        }
        if kind != 0
            || hello
                != HELLO
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>()
        {
            return Err(error("handshake", "unsupported helper version or mode"));
        }
        let (kind, data) = read_packet(&mut stream, &stop).map_err(|e| error("controls", e))?;
        if kind != 4 {
            return Err(error("controls", "missing control descriptors"));
        }
        let controls = usb_controls::parse(&data)?;
        let mode = configuration();
        Ok(Box::new(UsbHelperDevice {
            descriptor: descriptor(),
            capabilities: CameraCapabilities {
                modes: vec![CameraMode {
                    pixel_format: mode.pixel_format,
                    resolution: mode.resolution,
                    frame_rates: vec![mode.frame_rate],
                }],
                controls,
            },
            stream,
            events: Arc::new(EventMailbox::default()),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            started: false,
            closed: false,
            control_socket: usb_controls::socket_path(&self.socket),
        }))
    }
}
struct UsbHelperDevice {
    descriptor: CameraDescriptor,
    capabilities: CameraCapabilities,
    stream: UnixStream,
    events: Arc<EventMailbox>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    started: bool,
    closed: bool,
    control_socket: PathBuf,
}
impl CameraDevice for UsbHelperDevice {
    fn descriptor(&self) -> &CameraDescriptor {
        &self.descriptor
    }
    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }
    fn start_stream(&mut self, requested: &StreamConfiguration) -> CameraResult<()> {
        if requested != &configuration() {
            return Err(CameraError::new(
                CameraErrorKind::InvalidConfiguration,
                "DE400 USB supports only 1280x1024 YUYV at 8 fps",
            ));
        }
        if self.started || self.closed {
            return Err(error(
                "start",
                "open a new camera connection before restarting this device",
            ));
        }
        let mut input = self.stream.try_clone().map_err(|e| error("clone", e))?;
        self.stream.write_all(b"S").map_err(|e| error("start", e))?;
        input
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| error("timeout", e))?;
        let events = Arc::clone(&self.events);
        let stop = Arc::clone(&self.stop);
        self.events
            .set_button_status("Bouton DE400 : démarrage du canal USB…".to_owned());
        self.worker = Some(
            thread::Builder::new()
                .name("iriscope-usb-helper".to_owned())
                .spawn(move || {
                    let started = Instant::now();
                    let mut sequence = 0;
                    let mut button = ButtonProtocol::default();
                    while !stop.load(Ordering::Acquire) {
                        let event = match read_packet(&mut input, &stop) {
                            Ok((1, data)) => {
                                if sequence == 0 {
                                    events.set_button_status(
                                        "Bouton DE400 : réception USB directe active.".to_owned(),
                                    );
                                }
                                sequence += 1;
                                Ok(CameraEvent::Frame(CapturedFrame {
                                    sequence_number: sequence,
                                    timestamp: started.elapsed(),
                                    pixel_format: PixelFormat::Yuyv,
                                    resolution: Resolution::new(1280, 1024),
                                    data: Arc::from(data),
                                }))
                            }
                            Ok((2, data)) => {
                                if button.pressed(&data) {
                                    events.publish(Ok(CameraEvent::HardwareButtonPressed));
                                }
                                continue;
                            }
                            Ok((3, data)) => Err(error("helper", String::from_utf8_lossy(&data))),
                            Ok(_) => Err(error("protocol", "unexpected packet")),
                            Err(_) if stop.load(Ordering::Acquire) => break,
                            Err(e) => Err(CameraError::new(
                                CameraErrorKind::Disconnected,
                                format!("DE400 USB helper disconnected: {e}"),
                            )),
                        };
                        let terminal = event.is_err();
                        if let Err(error) = &event {
                            events.set_button_status(format!(
                                "Bouton DE400 : réception USB interrompue : {error}"
                            ));
                        }
                        events.publish(event);
                        if terminal {
                            break;
                        }
                    }
                })
                .map_err(|e| error("worker", e))?,
        );
        self.started = true;
        Ok(())
    }
    fn active_configuration(&self) -> Option<StreamConfiguration> {
        self.started.then(configuration)
    }
    fn hardware_button_status(&self) -> Option<String> {
        self.events.button_status()
    }
    fn stop_stream(&mut self) -> CameraResult<()> {
        self.started = false;
        self.closed = true;
        self.stop.store(true, Ordering::Release);
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.events
            .set_button_status("Bouton DE400 : réception USB arrêtée.".to_owned());
        self.events.close();
        Ok(())
    }
    fn next_event(&mut self, timeout: Duration) -> CameraResult<CameraEvent> {
        self.events.next_event(timeout)
    }
    fn control_value(&self, id: &CameraControlId) -> CameraResult<CameraControlValue> {
        let descriptor = self
            .capabilities
            .controls
            .iter()
            .find(|d| &d.id == id)
            .ok_or_else(unsupported_controls)?;
        let value = usb_controls::request(
            &self.control_socket,
            0,
            usb_controls::control_number(id)?,
            0,
        )?;
        usb_controls::decode(descriptor, value)
    }
    fn set_control_value(
        &mut self,
        id: &CameraControlId,
        value: &CameraControlValue,
    ) -> CameraResult<()> {
        let descriptor = self
            .capabilities
            .controls
            .iter()
            .find(|d| &d.id == id)
            .ok_or_else(unsupported_controls)?;
        let requested = usb_controls::encode(descriptor, value)?;
        let actual = usb_controls::request(
            &self.control_socket,
            1,
            usb_controls::control_number(id)?,
            requested,
        )?;
        if actual != requested {
            return Err(error(
                "controls",
                "camera did not retain the requested value",
            ));
        }
        Ok(())
    }
    fn reset_controls(&mut self) -> CameraResult<()> {
        let mut controls = self.capabilities.controls.clone();
        controls.sort_by_key(|d| matches!(d.kind, CameraControlKind::Integer { .. }));
        for descriptor in controls {
            if descriptor.read_only {
                continue;
            }
            if descriptor.id == CameraControlId::Standard(StandardCameraControl::WhiteBalanceManual)
                && self
                    .control_value(&CameraControlId::Standard(
                        StandardCameraControl::WhiteBalanceAutomatic,
                    ))
                    .ok()
                    == Some(CameraControlValue::Boolean(true))
            {
                continue;
            }
            if descriptor.id == CameraControlId::Standard(StandardCameraControl::Exposure)
                && self
                    .control_value(&CameraControlId::Standard(
                        StandardCameraControl::ExposureMode,
                    ))
                    .ok()
                    .is_some_and(|v| v != CameraControlValue::Menu(1))
            {
                continue;
            }
            let value = match descriptor.kind {
                CameraControlKind::Integer { default, .. } => CameraControlValue::Integer(default),
                CameraControlKind::Boolean { default } => CameraControlValue::Boolean(default),
                CameraControlKind::Menu { default, .. } => CameraControlValue::Menu(default),
                _ => continue,
            };
            self.set_control_value(&descriptor.id, &value)?;
        }
        Ok(())
    }
}
fn unsupported_controls() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "This control is not exposed by the connected DE400",
    )
}
impl Drop for UsbHelperDevice {
    fn drop(&mut self) {
        let _ = self.stop_stream();
    }
}

fn read_bytes(input: &mut impl Read, output: &mut [u8], stop: &AtomicBool) -> io::Result<()> {
    let started = Instant::now();
    let mut offset = 0;
    while offset < output.len() {
        if stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match input.read(&mut output[offset..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => offset += count,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) && started.elapsed() < Duration::from_secs(5) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn read_packet(input: &mut impl Read, stop: &AtomicBool) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0; 8];
    read_bytes(input, &mut header, stop)?;
    let kind = u32::from_le_bytes(header[..4].try_into().expect("fixed header"));
    let length = u32::from_le_bytes(header[4..].try_into().expect("fixed header")) as usize;
    if !matches!(
        (kind, length),
        (0, 16) | (1, FRAME_BYTES) | (2, 4) | (3, 1..=1024) | (4, 0..=416)
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid helper packet kind or length",
        ));
    }
    if kind == 4 && length % 32 != 0 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut data = vec![0; length];
    read_bytes(input, &mut data, stop)?;
    Ok((kind, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_partial_packets_across_timeouts() {
        struct Fragments<'a> {
            bytes: &'a [u8],
            timeout: bool,
        }
        impl Read for Fragments<'_> {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.timeout = !self.timeout;
                if self.timeout {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                self.bytes.read(&mut output[..1])
            }
        }
        let data = [
            2_u32.to_le_bytes().as_slice(),
            4_u32.to_le_bytes().as_slice(),
            &[2, 1, 0, 1],
        ]
        .concat();
        let mut input = Fragments {
            bytes: &data,
            timeout: false,
        };
        assert_eq!(
            read_packet(&mut input, &AtomicBool::new(false)).unwrap(),
            (2, vec![2, 1, 0, 1])
        );
    }
    #[test]
    fn refuses_partial_frames_and_large_packets_before_allocating_or_capturing() {
        for (kind, size) in [
            (1_u32, u32::try_from(FRAME_BYTES).unwrap() - 1),
            (2, 5),
            (3, 1025),
            (4, u32::MAX),
        ] {
            let data = [kind.to_le_bytes(), size.to_le_bytes()].concat();
            assert_eq!(
                read_packet(&mut data.as_slice(), &AtomicBool::new(false))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
    #[test]
    fn truncated_release_cannot_rearm_or_generate_a_capture() {
        let data = [
            2_u32.to_le_bytes().as_slice(),
            4_u32.to_le_bytes().as_slice(),
            &[2, 1, 0],
        ]
        .concat();
        assert_eq!(
            read_packet(&mut data.as_slice(), &AtomicBool::new(false))
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
