use crate::app_helpers::{persist_settings, settings_error};
use crate::config::NOTICE_ERROR;
use crate::playback::show_capture_notice;
use crate::ui::MainWindow;
use iriscope_core::camera::CapturedFrame;
use iriscope_core::capabilities::{CameraControlId, CameraControlValue};
use iriscope_core::settings::AppSettings;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
pub(super) struct ControlCommandMailbox {
    pub(super) state: Mutex<ControlCommandState>,
}

#[derive(Default)]
pub(super) struct ControlCommandState {
    pub(super) updates: VecDeque<(CameraControlId, CameraControlValue)>,
    pub(super) reset: bool,
    pub(super) stop: bool,
}

pub(super) struct ControlCommandBatch {
    pub(super) updates: VecDeque<(CameraControlId, CameraControlValue)>,
    pub(super) reset: bool,
    pub(super) stop: bool,
}

impl ControlCommandMailbox {
    pub(super) fn set_control(&self, id: CameraControlId, value: CameraControlValue) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.stop {
            if let Some(previous) = state.updates.iter().position(|(key, _)| *key == id) {
                state.updates.remove(previous);
            }
            state.updates.push_back((id, value));
        }
    }

    pub(super) fn reset_controls(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.updates.clear();
        state.reset = true;
    }

    pub(super) fn stop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.stop = true;
        state.reset = false;
        state.updates.clear();
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stop
    }

    pub(super) fn take(&self) -> ControlCommandBatch {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ControlCommandBatch {
            updates: std::mem::take(&mut state.updates),
            reset: std::mem::take(&mut state.reset),
            stop: state.stop,
        }
    }
}

/// Coalesces settings updates before writing them to disk.
#[derive(Default)]
pub(super) struct CameraSettingsSaveMailbox {
    pub(super) state: Mutex<CameraSettingsSaveState>,
    pub(super) changed: Condvar,
}

#[derive(Default)]
pub(super) struct CameraSettingsSaveState {
    pub(super) generation: u64,
    pub(super) dirty: bool,
    pub(super) closed: bool,
}

impl CameraSettingsSaveMailbox {
    pub(super) fn mark_dirty(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
        state.dirty = true;
        self.changed.notify_one();
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        self.changed.notify_one();
    }
}

pub(super) fn run_camera_settings_save_worker(
    mailbox: &CameraSettingsSaveMailbox,
    settings: &Arc<Mutex<AppSettings>>,
    path: &std::path::Path,
    window: Option<&slint::Weak<MainWindow>>,
) {
    let mut state = mailbox
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    loop {
        while !state.dirty && !state.closed {
            state = mailbox
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if !state.closed {
            let generation = state.generation;
            let (next, _) = mailbox
                .changed
                .wait_timeout_while(state, Duration::from_millis(500), |current| {
                    !current.closed && current.generation == generation
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            if !state.closed && state.generation != generation {
                continue;
            }
        }

        let dirty = std::mem::take(&mut state.dirty);
        let closed = state.closed;
        let saved_generation = state.generation;
        drop(state);
        if dirty {
            let result = persist_settings(settings, path);
            let current_generation = mailbox
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .generation;
            if let Some(window) = window
                && current_generation == saved_generation
            {
                match result {
                    Ok(()) => {
                        let _ = window.upgrade_in_event_loop(|win| {
                            if win.get_settings_feedback().starts_with("Enregistrement") {
                                win.set_settings_feedback("Paramètres enregistrés.".into());
                                win.set_settings_feedback_is_error(false);
                            }
                        });
                    }
                    Err(error) => {
                        eprintln!("Impossible d'enregistrer les paramètres : {error}");
                        let message =
                            format!("Paramètres appliqués, mais non enregistrés : {error}");
                        let _ = window.upgrade_in_event_loop(move |win| {
                            settings_error(&win, &message);
                            show_capture_notice(&win, message, NOTICE_ERROR);
                        });
                    }
                }
            } else if let Err(error) = result {
                eprintln!("Impossible d'enregistrer les paramètres : {error}");
            }
        }
        if closed {
            break;
        }
        state = mailbox
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
}

#[cfg(test)]
mod control_command_tests {
    use iriscope_core::capabilities::{CameraControlId, CameraControlValue, StandardCameraControl};

    use super::ControlCommandMailbox;

    #[test]
    pub(super) fn slider_burst_keeps_only_the_newest_value_and_stop_wins() {
        let mailbox = ControlCommandMailbox::default();
        let id = CameraControlId::Standard(StandardCameraControl::Brightness);
        for value in 0..10_000 {
            mailbox.set_control(id.clone(), CameraControlValue::Integer(value));
        }
        let batch = mailbox.take();
        assert_eq!(batch.updates.len(), 1);
        assert_eq!(
            batch.updates.front(),
            Some(&(id.clone(), CameraControlValue::Integer(9_999)))
        );

        mailbox.set_control(id.clone(), CameraControlValue::Integer(1));
        mailbox.reset_controls();
        mailbox.set_control(id.clone(), CameraControlValue::Integer(2));
        let batch = mailbox.take();
        assert!(batch.reset);
        assert_eq!(
            batch.updates.front(),
            Some(&(id.clone(), CameraControlValue::Integer(2)))
        );

        mailbox.set_control(id, CameraControlValue::Integer(3));
        mailbox.stop();
        let batch = mailbox.take();
        assert!(batch.stop);
        assert!(batch.updates.is_empty());
    }

    #[test]
    pub(super) fn dependent_controls_keep_the_order_of_the_latest_changes() {
        let mailbox = ControlCommandMailbox::default();
        let automatic = CameraControlId::Standard(StandardCameraControl::ExposureMode);
        let manual = CameraControlId::Standard(StandardCameraControl::Exposure);
        mailbox.set_control(manual.clone(), CameraControlValue::Integer(20));
        mailbox.set_control(automatic.clone(), CameraControlValue::Menu(1));
        mailbox.set_control(manual.clone(), CameraControlValue::Integer(30));

        let updates = mailbox.take().updates.into_iter().collect::<Vec<_>>();
        assert_eq!(
            updates,
            vec![
                (automatic, CameraControlValue::Menu(1)),
                (manual, CameraControlValue::Integer(30))
            ]
        );
    }
}

#[derive(Debug, Default)]
pub(super) struct DecodeMailbox {
    pub(super) state: Mutex<DecodeMailboxState>,
    pub(super) ready: Condvar,
}

#[derive(Debug, Default)]
pub(super) struct DecodeMailboxState {
    pub(super) frame: Option<QueuedDecodeFrame>,
    pub(super) closed: bool,
}

#[derive(Debug)]
pub(super) struct QueuedDecodeFrame {
    pub(super) generation: u64,
    pub(super) frame: CapturedFrame,
}

impl QueuedDecodeFrame {
    pub(super) fn is_current(&self, generation: &AtomicU64) -> bool {
        self.generation == generation.load(Ordering::Acquire)
    }
}

impl DecodeMailbox {
    pub(super) fn publish(&self, generation: u64, frame: CapturedFrame) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return false;
        }
        let replaced = state
            .frame
            .replace(QueuedDecodeFrame { generation, frame })
            .is_some();
        self.ready.notify_one();
        replaced
    }

    pub(super) fn receive(&self) -> Option<QueuedDecodeFrame> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.frame.is_none() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.frame.take()
    }

    pub(super) fn clear(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .frame = None;
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.frame = None;
        self.ready.notify_all();
    }
}
