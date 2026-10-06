//! Optional, unprivileged receiver for the DE400 USB status bridge.

use iriscope_core::camera::CameraEvent;
use std::{
    collections::VecDeque,
    fs,
    io::{self, Read, Write},
    os::unix::{fs::MetadataExt, net::UnixStream},
    time::{Duration, Instant},
};

const SOCKET_PATH: &str = "/run/iriscope-button/status.sock";

pub(super) struct ButtonMonitor {
    device_key: String,
    stream: Option<UnixStream>,
    decoder: ButtonDecoder,
    retry_at: Instant,
}

impl ButtonMonitor {
    pub(super) fn new(device_key: String) -> Self {
        Self {
            device_key,
            stream: None,
            decoder: ButtonDecoder::default(),
            retry_at: Instant::now(),
        }
    }

    fn connect(&mut self) -> io::Result<()> {
        // The privileged helper and its parent directory must belong to root.
        // The application itself never needs administrator privileges.
        let parent = fs::metadata("/run/iriscope-button")?;
        let socket = fs::symlink_metadata(SOCKET_PATH)?;
        if parent.uid() != 0 || parent.mode() & 0o022 != 0 || socket.uid() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted button bridge",
            ));
        }
        let mut stream = UnixStream::connect(SOCKET_PATH)?;
        stream.set_write_timeout(Some(Duration::from_millis(20)))?;
        writeln!(stream, "{}", self.device_key)?;
        stream.set_nonblocking(true)?;
        self.decoder = ButtonDecoder::default();
        self.stream = Some(stream);
        Ok(())
    }

    pub(super) fn next_event(&mut self) -> Option<CameraEvent> {
        if let Some(event) = self.decoder.events.pop_front() {
            return Some(event);
        }
        if self.stream.is_none() && Instant::now() >= self.retry_at {
            self.retry_at = Instant::now() + Duration::from_secs(2);
            if self.connect().is_err() {
                return None;
            }
        }
        let stream = self.stream.as_mut()?;
        let mut bytes = [0_u8; 64];
        match stream.read(&mut bytes) {
            Ok(count) if count != 0 && self.decoder.feed(&bytes[..count]) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(_) | Err(_) => self.disconnect(),
        }
        self.decoder.events.pop_front()
    }

    fn disconnect(&mut self) {
        self.stream = None;
        self.decoder = ButtonDecoder::default();
        self.retry_at = Instant::now() + Duration::from_secs(2);
    }

    pub(super) fn reset(&mut self) {
        self.disconnect();
        self.retry_at = Instant::now();
    }
}

#[derive(Default)]
struct ButtonDecoder {
    pending: Vec<u8>,
    events: VecDeque<CameraEvent>,
    ready: bool,
    pressed: bool,
}

impl ButtonDecoder {
    fn feed(&mut self, bytes: &[u8]) -> bool {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            match line.as_slice() {
                b"READY\n" if !self.ready => self.ready = true,
                b"P\n" if self.ready && !self.pressed => {
                    self.pressed = true;
                    self.events.push_back(CameraEvent::HardwareButtonPressed);
                }
                b"R\n" if self.ready && self.pressed => {
                    self.pressed = false;
                    self.events.push_back(CameraEvent::HardwareButtonReleased);
                }
                b"P\n" | b"R\n" if self.ready => {}
                _ => return false,
            }
            if self.events.len() > 16 {
                return false;
            }
        }
        self.pending.len() <= 6
    }
}

#[cfg(test)]
mod tests {
    use super::ButtonDecoder;
    use iriscope_core::camera::CameraEvent;

    #[test]
    fn fragmented_messages_and_held_buttons_capture_once_per_press() {
        let mut decoder = ButtonDecoder::default();
        for part in [b"REA".as_slice(), b"DY\nP", b"\nP\nR\nR\nP\nR\n"] {
            assert!(decoder.feed(part));
        }
        assert!(matches!(
            decoder.events.pop_front(),
            Some(CameraEvent::HardwareButtonPressed)
        ));
        assert!(matches!(
            decoder.events.pop_front(),
            Some(CameraEvent::HardwareButtonReleased)
        ));
        assert!(matches!(
            decoder.events.pop_front(),
            Some(CameraEvent::HardwareButtonPressed)
        ));
        assert!(matches!(
            decoder.events.pop_front(),
            Some(CameraEvent::HardwareButtonReleased)
        ));
        assert!(decoder.events.is_empty());
    }

    #[test]
    fn missing_handshake_invalid_messages_and_floods_are_rejected() {
        assert!(!ButtonDecoder::default().feed(b"P\n"));
        assert!(!ButtonDecoder::default().feed(b"READY\nKEYBOARD\n"));
        assert!(!ButtonDecoder::default().feed(b"1234567"));
        let mut decoder = ButtonDecoder::default();
        assert!(decoder.feed(b"READY\n"));
        assert!(!decoder.feed(b"P\nR\n".repeat(9).as_slice()));
    }
}
