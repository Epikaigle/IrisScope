//! Bounded executor for occasional disk jobs, owned by the application runtime.
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

type Job = Box<dyn FnOnce() + Send>;
const MAX_PENDING_JOBS: usize = 16;
#[derive(Default)]
struct State {
    pending: VecDeque<Job>,
    closed: bool,
}
#[derive(Default)]
pub(crate) struct BackgroundJobs {
    state: Mutex<State>,
    ready: Condvar,
}
impl BackgroundJobs {
    pub(crate) fn submit(&self, job: impl FnOnce() + Send + 'static) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.pending.len() >= MAX_PENDING_JOBS {
            return false;
        }
        state.pending.push_back(Box::new(job));
        self.ready.notify_one();
        true
    }
    pub(crate) fn run(&self) {
        loop {
            let job = {
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
                    return;
                }
                state.pending.pop_front()
            };
            if let Some(job) = job {
                job();
            }
        }
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.state
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
