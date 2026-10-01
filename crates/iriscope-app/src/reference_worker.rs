//! Latest reference requests share a worker and cancel before expensive decoding.
use crate::app_helpers::settings_error;
use crate::camera_queue::CameraSettingsSaveMailbox;
use crate::config::DecodedFrame;
use crate::library_ui::load_reference_cancellable;
use crate::ui::MainWindow;
use iriscope_core::settings::AppSettings;
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

pub(crate) struct ReferenceRequest {
    pub(crate) path: PathBuf,
    pub(crate) generation: u64,
    pub(crate) current_generation: Arc<AtomicU64>,
    pub(crate) is_map: bool,
    pub(crate) persist: bool,
}
#[derive(Default)]
struct State {
    pending: VecDeque<ReferenceRequest>,
    closed: bool,
}
#[derive(Default)]
pub(crate) struct ReferenceMailbox {
    state: Mutex<State>,
    ready: Condvar,
}
impl ReferenceMailbox {
    pub(crate) fn request(&self, request: ReferenceRequest) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        state.pending.retain(|old| old.is_map != request.is_map);
        state.pending.push_back(request);
        self.ready.notify_one();
    }
    fn receive(&self) -> Option<ReferenceRequest> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.pending.is_empty() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if state.closed {
            None
        } else {
            state.pending.pop_front()
        }
    }
    fn is_current(&self, request: &ReferenceRequest) -> bool {
        request.current_generation.load(Ordering::Acquire) == request.generation
            && !self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .closed
    }
    pub(crate) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.pending.clear();
        self.ready.notify_all();
    }
}

pub(crate) fn run_reference_worker(
    mailbox: &ReferenceMailbox,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    save: &Arc<CameraSettingsSaveMailbox>,
) {
    run_reference_worker_with_loader(mailbox, weak, settings, save, load_reference_cancellable);
}
fn run_reference_worker_with_loader(
    mailbox: &ReferenceMailbox,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    save: &Arc<CameraSettingsSaveMailbox>,
    loader: impl Fn(&std::path::Path, &dyn Fn() -> bool) -> Option<DecodedFrame>,
) {
    while let Some(request) = mailbox.receive() {
        if !mailbox.is_current(&request) {
            continue;
        }
        let decoded = loader(&request.path, &|| mailbox.is_current(&request));
        if !mailbox.is_current(&request) {
            continue;
        }
        let pixels = decoded.map(|(width, height, rgb)| {
            SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height)
        });
        let settings = Arc::clone(settings);
        let save = Arc::clone(save);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if request.current_generation.load(Ordering::Acquire) != request.generation {
                return;
            }
            let Some(pixels) = pixels else {
                if request.persist {
                    settings_error(&win, "Référence introuvable ou image illisible.");
                }
                return;
            };
            if request.is_map {
                win.set_has_iridology_map(true);
                win.set_iridology_map_image(slint::Image::from_rgb8(pixels));
                if request.persist {
                    win.set_settings_iridology_map_path(
                        request.path.to_string_lossy().into_owned().into(),
                    );
                }
            } else {
                win.set_has_iridology_symbols(true);
                win.set_iridology_symbols_image(slint::Image::from_rgb8(pixels));
                if request.persist {
                    win.set_settings_iridology_symbols_path(
                        request.path.to_string_lossy().into_owned().into(),
                    );
                }
            }
            if request.persist {
                if let Ok(mut settings) = settings.lock() {
                    if request.is_map {
                        settings.iridology_map_path = Some(request.path);
                    } else {
                        settings.iridology_symbols_path = Some(request.path);
                    }
                }
                save.mark_dirty();
                win.set_settings_feedback_is_error(false);
                win.set_settings_feedback("Enregistrement de la référence…".into());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{ReferenceMailbox, ReferenceRequest};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    fn request(generation: u64, current: &Arc<AtomicU64>, is_map: bool) -> ReferenceRequest {
        ReferenceRequest {
            path: "not-opened".into(),
            generation,
            current_generation: Arc::clone(current),
            is_map,
            persist: true,
        }
    }
    #[test]
    fn latest_reference_per_slot_supersedes_queued_work_and_close_cancels_all() {
        let mailbox = ReferenceMailbox::default();
        let map = Arc::new(AtomicU64::new(1));
        let symbols = Arc::new(AtomicU64::new(1));
        mailbox.request(request(1, &map, true));
        mailbox.request(request(1, &symbols, false));
        map.store(2, Ordering::Release);
        mailbox.request(request(2, &map, true));
        let first = mailbox.receive().unwrap();
        assert!(!first.is_map);
        let latest = mailbox.receive().unwrap();
        assert_eq!(latest.generation, 2);
        assert!(mailbox.is_current(&latest));
        map.store(3, Ordering::Release);
        assert!(!mailbox.is_current(&latest));
        mailbox.request(request(3, &map, true));
        mailbox.close();
        assert!(!mailbox.is_current(&first));
        assert!(mailbox.receive().is_none());
    }
}
