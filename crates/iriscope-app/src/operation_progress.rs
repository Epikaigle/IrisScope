//! Coalesced progress updates and cancellation for long operations off the UI thread.
use crate::ui::MainWindow;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Default)]
pub(crate) struct OperationControl {
    cancelled: AtomicBool,
    generation: AtomicU64,
    running: AtomicBool,
}
impl OperationControl {
    pub(crate) fn begin(&self) -> u64 {
        self.cancelled.store(false, Ordering::Release);
        self.running.store(true, Ordering::Release);
        self.generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    pub(crate) fn finish(&self, generation: u64) {
        if self.generation.load(Ordering::Acquire) == generation {
            self.running.store(false, Ordering::Release);
        }
    }
    pub(crate) fn is_cancelled(&self, generation: u64) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self.generation.load(Ordering::Acquire) != generation
    }
}

pub(crate) struct ProgressReporter<T> {
    latest: Arc<Mutex<Option<T>>>,
    pending: Arc<AtomicBool>,
    window: slint::Weak<MainWindow>,
    control: Arc<OperationControl>,
    generation: u64,
    apply: fn(&MainWindow, T),
}
impl<T: Send + 'static> ProgressReporter<T> {
    pub(crate) fn new(
        window: slint::Weak<MainWindow>,
        control: Arc<OperationControl>,
        generation: u64,
        apply: fn(&MainWindow, T),
    ) -> Self {
        Self {
            latest: Arc::new(Mutex::new(None)),
            pending: Arc::new(AtomicBool::new(false)),
            window,
            control,
            generation,
            apply,
        }
    }
    pub(crate) fn report(&self, value: T) {
        *self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let latest = Arc::clone(&self.latest);
        let pending = Arc::clone(&self.pending);
        let control = Arc::clone(&self.control);
        let generation = self.generation;
        let apply = self.apply;
        if self
            .window
            .upgrade_in_event_loop(move |win| {
                pending.store(false, Ordering::Release);
                if !control.is_cancelled(generation)
                    && let Some(value) = latest
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                {
                    apply(&win, value);
                }
            })
            .is_err()
        {
            self.pending.store(false, Ordering::Release);
        }
    }
}
