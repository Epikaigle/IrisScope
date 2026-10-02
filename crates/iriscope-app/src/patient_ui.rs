use crate::app_helpers::settings_snapshot;
use crate::config::NOTICE_ERROR;
use crate::playback::show_capture_notice;
use crate::ui::{MainWindow, PatientCandidateData};
use iriscope_core::library::{PatientRecord, search_patients};
use iriscope_core::settings::AppSettings;
use slint::{ModelRc, VecModel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(super) struct PatientSearchRequest {
    pub(super) directory: std::path::PathBuf,
    pub(super) first_name: String,
    pub(super) last_name: String,
    pub(super) generation: u64,
}

#[derive(Default)]
struct PatientSearchState {
    pending: Option<PatientSearchRequest>,
    closed: bool,
}

#[derive(Default)]
pub(super) struct PatientSearchMailbox {
    state: Mutex<PatientSearchState>,
    ready: Condvar,
}

impl PatientSearchMailbox {
    pub(super) fn request(&self, request: PatientSearchRequest) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.closed {
            state.pending = Some(request);
            self.ready.notify_one();
        }
    }

    pub(super) fn receive(&self) -> Option<PatientSearchRequest> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.pending.is_none() && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if state.closed {
            return None;
        }
        let mut request = state.pending.take()?;
        loop {
            let (next, result) = self
                .ready
                .wait_timeout(state, Duration::from_millis(180))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            if state.closed {
                return None;
            }
            if let Some(newest) = state.pending.take() {
                request = newest;
                continue;
            }
            if result.timed_out() {
                return Some(request);
            }
        }
    }

    pub(super) fn close(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.pending = None;
        self.ready.notify_all();
    }
}

pub(super) fn patient_candidate_data(record: PatientRecord) -> PatientCandidateData {
    PatientCandidateData {
        id: record.id.to_string().into(),
        dossier_number: record.dossier_number.into(),
        first_name: record.first_name.into(),
        last_name: record.last_name.into(),
        last_capture: record.last_capture.unwrap_or_default().into(),
    }
}

pub(super) fn clear_patient_candidates(win: &MainWindow) {
    win.set_patient_candidates(ModelRc::new(VecModel::from(Vec::new())));
}

pub(super) fn run_patient_search_worker(
    mailbox: &PatientSearchMailbox,
    generation: &Arc<AtomicU64>,
    settings: &Arc<Mutex<AppSettings>>,
    weak: &slint::Weak<MainWindow>,
) {
    while let Some(request) = mailbox.receive() {
        let result = search_patients(&request.directory, &request.first_name, &request.last_name);
        if generation.load(Ordering::Acquire) != request.generation {
            continue;
        }
        let settings = Arc::clone(settings);
        let generation = Arc::clone(generation);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if generation.load(Ordering::Acquire) != request.generation
                || settings_snapshot(&settings).capture_directory != request.directory
                || win.get_patient_first_name().trim() != request.first_name
                || win.get_patient_last_name().trim() != request.last_name
                || !win.get_patient_id().is_empty()
            {
                return;
            }
            win.set_patient_search_pending(false);
            match result {
                Ok(records) => {
                    let candidates = records
                        .into_iter()
                        .take(25)
                        .map(patient_candidate_data)
                        .collect::<Vec<_>>();
                    win.set_patient_candidates(ModelRc::new(VecModel::from(candidates)));
                }
                Err(error) => {
                    clear_patient_candidates(&win);
                    show_capture_notice(
                        &win,
                        format!("Recherche des dossiers impossible : {error}"),
                        NOTICE_ERROR,
                    );
                }
            }
        });
    }
}
