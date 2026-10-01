use crate::app_helpers::settings_snapshot;
use crate::config::{LIBRARY_PAGE_SIZE, NOTICE_INFO};
use crate::library_ui::{ThumbnailCache, load_library_payloads, present_library_payloads};
use crate::playback::show_capture_notice;
use crate::ui::MainWindow;
use iriscope_core::library::recover_library_index_cancellable;
use iriscope_core::session::CaptureSession;
use iriscope_core::settings::AppSettings;
use slint::{ModelRc, VecModel};
use std::sync::{Arc, Condvar, Mutex};

pub(super) struct LibraryRefreshRequest {
    pub(super) directory: std::path::PathBuf,
    pub(super) session: CaptureSession,
    pub(super) filter: i32,
    pub(super) page: usize,
    pub(super) revision: u64,
    pub(super) cache_revision: u64,
}

#[derive(Default)]
pub(super) struct LibraryRefreshMailbox {
    pub(super) state: Mutex<LibraryRefreshState>,
    pub(super) ready: Condvar,
}

#[derive(Default)]
pub(super) struct LibraryRefreshState {
    pub(super) pending: Option<LibraryRefreshRequest>,
    pub(super) filter: i32,
    pub(super) page: usize,
    pub(super) revision: u64,
    pub(super) cache_revision: u64,
    pub(super) closed: bool,
}

impl LibraryRefreshMailbox {
    pub(super) fn invalidate_thumbnails(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cache_revision = state.cache_revision.wrapping_add(1);
    }

    pub(super) fn set_filter(&self, filter: i32) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.filter = filter;
        state.page = 0;
    }

    pub(super) fn set_page(&self, page: usize) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .page = page;
    }

    pub(super) fn request(&self, directory: std::path::PathBuf, session: CaptureSession) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        state.revision = state.revision.wrapping_add(1);
        state.pending = Some(LibraryRefreshRequest {
            directory,
            session,
            filter: state.filter,
            page: state.page,
            revision: state.revision,
            cache_revision: state.cache_revision,
        });
        self.ready.notify_one();
    }

    pub(super) fn receive(&self) -> Option<LibraryRefreshRequest> {
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
        state.pending.take()
    }

    pub(super) fn is_current(&self, revision: u64) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !state.closed && state.revision == revision
    }

    pub(super) fn set_resolved_page(&self, revision: u64, page: usize) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.revision != revision {
            return false;
        }
        state.page = page;
        true
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

pub(super) fn refresh_library_in_background(
    mailbox: &LibraryRefreshMailbox,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
) {
    let directory = settings_snapshot(settings).capture_directory;
    let session = active_session
        .lock()
        .map_or_else(|_| CaptureSession::default(), |guard| guard.clone());
    mailbox.request(directory, session);
}

pub(super) fn run_library_worker(
    mailbox: &Arc<LibraryRefreshMailbox>,
    weak: &slint::Weak<MainWindow>,
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
) {
    let mut cache = ThumbnailCache::default();
    let mut cached_directory = None;
    let mut cached_revision = 0;
    let mut checked_recovery = None;
    while let Some(request) = mailbox.receive() {
        if !mailbox.is_current(request.revision) {
            continue;
        }
        if cached_directory.as_ref() != Some(&request.directory) {
            cache.invalidate();
            cached_directory = Some(request.directory.clone());
        }
        if cached_revision != request.cache_revision {
            cache.invalidate();
            cached_revision = request.cache_revision;
        }
        let recovery_key = (request.directory.clone(), request.cache_revision);
        let mut recovery_error = None;
        let index_recovered = if checked_recovery.as_ref() == Some(&recovery_key) {
            false
        } else {
            match recover_library_index_cancellable(&request.directory, &|| {
                !mailbox.is_current(request.revision)
            }) {
                Ok(recovered) => {
                    checked_recovery = Some(recovery_key);
                    recovered
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    eprintln!("[IrisScope] Récupération de l'index impossible : {error}");
                    recovery_error =
                        Some(format!("Reprise de la bibliothèque impossible : {error}"));
                    false
                }
            }
        };
        let Some(mut result) = load_library_payloads(
            &request.directory,
            &request.session,
            request.filter,
            request.page,
            &mut cache,
            || mailbox.is_current(request.revision),
        ) else {
            continue;
        };
        if result.error.is_none() {
            result.error = recovery_error;
        }
        if !mailbox.is_current(request.revision) {
            continue;
        }
        let mailbox_for_ui = Arc::clone(mailbox);
        let settings_for_ui = Arc::clone(settings);
        let session_for_ui = Arc::clone(active_session);
        let _ = weak.upgrade_in_event_loop(move |win| {
            if win.get_library_filter() != request.filter
                || !library_context_matches(
                    &settings_for_ui,
                    &session_for_ui,
                    &request.directory,
                    &request.session,
                )
                || !mailbox_for_ui.set_resolved_page(request.revision, result.page)
            {
                return;
            }
            let first = result.page * LIBRARY_PAGE_SIZE + usize::from(result.total > 0);
            let last = ((result.page + 1) * LIBRARY_PAGE_SIZE).min(result.total);
            let summary = format!("{first}–{last} sur {}", result.total);
            win.set_library_items(ModelRc::new(VecModel::from(present_library_payloads(
                result.items,
            ))));
            win.set_library_total_count(i32::try_from(result.total).unwrap_or(i32::MAX));
            win.set_library_page(i32::try_from(result.page).unwrap_or(i32::MAX));
            win.set_library_page_summary(summary.into());
            win.set_library_error(result.error.unwrap_or_default().into());
            win.set_library_loading(false);
            if index_recovered {
                show_capture_notice(
                    &win,
                    "Index bibliothèque réparé ; vérifiez les captures sans dossier attribué.",
                    NOTICE_INFO,
                );
            }
        });
    }
}

pub(super) fn library_context_matches(
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    directory: &std::path::Path,
    session: &CaptureSession,
) -> bool {
    settings_snapshot(settings).capture_directory == directory
        && active_session
            .lock()
            .is_ok_and(|current| *current == *session)
}

pub(super) fn recording_context_matches(
    settings: &Arc<Mutex<AppSettings>>,
    active_session: &Arc<Mutex<CaptureSession>>,
    directory: &std::path::Path,
    recorded_session: &CaptureSession,
    form_session: Option<&CaptureSession>,
) -> bool {
    form_session.is_some_and(|current| current == recorded_session)
        && library_context_matches(settings, active_session, directory, recorded_session)
}
