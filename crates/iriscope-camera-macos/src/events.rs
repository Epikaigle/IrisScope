//! Latest-frame mailbox with independent, bounded hardware button events.
use iriscope_core::camera::{CameraError, CameraErrorKind, CameraEvent, CameraResult};
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct EventMailbox {
    state: Mutex<EventMailboxState>,
    available: Condvar,
}

#[derive(Default)]
struct EventMailboxState {
    pending: Option<CameraResult<CameraEvent>>,
    buttons: VecDeque<CameraEvent>,
    button_status: Option<String>,
    closed: bool,
}

impl EventMailbox {
    pub(super) fn publish(&self, event: CameraResult<CameraEvent>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        if let Ok(CameraEvent::HardwareButtonPressed) = &event {
            if state.buttons.len() < 16 {
                state.buttons.push_back(CameraEvent::HardwareButtonPressed);
            }
            self.available.notify_one();
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

    pub(super) fn set_button_status(&self, status: String) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .button_status = Some(status);
    }
    pub(super) fn button_status(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .button_status
            .clone()
    }

    pub(super) fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending = None;
        state.buttons.clear();
        state.button_status = None;
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending = None;
        state.buttons.clear();
        state.closed = true;
        self.available.notify_all();
    }

    pub(super) fn next_event(&self, timeout: Duration) -> CameraResult<CameraEvent> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        loop {
            if state
                .pending
                .as_ref()
                .is_some_and(|e| !matches!(e, Ok(CameraEvent::Frame(_))))
            {
                state.buttons.clear();
                return state.pending.take().expect("terminal event exists");
            }
            if let Some(event) = state.buttons.pop_front() {
                return Ok(event);
            }
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
            if waited.1.timed_out() && state.pending.is_none() && state.buttons.is_empty() {
                return Err(CameraError::new(
                    CameraErrorKind::TimedOut,
                    "timed out waiting for an AVFoundation camera event",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iriscope_core::{
        camera::CapturedFrame,
        capabilities::{PixelFormat, Resolution},
    };
    use std::sync::Arc;
    fn frame(sequence_number: u64) -> CameraEvent {
        CameraEvent::Frame(CapturedFrame {
            sequence_number,
            timestamp: Duration::ZERO,
            pixel_format: PixelFormat::Mjpeg,
            resolution: Resolution::new(640, 480),
            data: Arc::from([]),
        })
    }
    #[test]
    fn frames_cannot_replace_button_presses_and_stream_restart_clears_them() {
        let mailbox = EventMailbox::default();
        mailbox.publish(Ok(CameraEvent::HardwareButtonPressed));
        mailbox.publish(Ok(frame(1)));
        mailbox.publish(Ok(CameraEvent::HardwareButtonPressed));
        mailbox.publish(Ok(frame(2)));
        for _ in 0..2 {
            assert_eq!(
                mailbox.next_event(Duration::ZERO).unwrap(),
                CameraEvent::HardwareButtonPressed
            );
        }
        assert_eq!(mailbox.next_event(Duration::ZERO).unwrap(), frame(2));
        mailbox.publish(Ok(CameraEvent::HardwareButtonPressed));
        mailbox.set_button_status("active".to_owned());
        assert_eq!(mailbox.button_status().as_deref(), Some("active"));
        mailbox.clear();
        assert!(mailbox.button_status().is_none());
        assert_eq!(
            mailbox.next_event(Duration::ZERO).unwrap_err().kind(),
            CameraErrorKind::TimedOut
        );
        mailbox.close();
        mailbox.publish(Ok(CameraEvent::HardwareButtonPressed));
        assert_eq!(
            mailbox.next_event(Duration::ZERO).unwrap_err().kind(),
            CameraErrorKind::Disconnected
        );
    }
    #[test]
    fn disconnect_has_priority_and_does_not_capture_stale_presses() {
        let mailbox = EventMailbox::default();
        for _ in 0..100 {
            mailbox.publish(Ok(CameraEvent::HardwareButtonPressed));
        }
        assert_eq!(mailbox.state.lock().unwrap().buttons.len(), 16);
        mailbox.publish(Ok(CameraEvent::Disconnected));
        mailbox.publish(Ok(frame(3)));
        assert_eq!(
            mailbox.next_event(Duration::ZERO).unwrap(),
            CameraEvent::Disconnected
        );
        assert!(mailbox.state.lock().unwrap().buttons.is_empty());
    }
}
