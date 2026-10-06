use crate::app_helpers::settings_snapshot;
use crate::config::{LIBRARY_PAGE_SIZE, NOTICE_INFO};
use crate::library_ui::{
    ThumbnailCache, load_library_metadata, load_payload_thumbnail, present_library_payloads,
};
use crate::playback::show_capture_notice;
use crate::ui::MainWindow;
use iriscope_core::library::recover_library_index_cancellable;
use iriscope_core::session::CaptureSession;
use iriscope_core::settings::AppSettings;
use slint::{Model, ModelRc, VecModel};
use std::sync::{Arc, Condvar, Mutex};

pub(super) struct LibraryRefreshRequest {
    pub(super) directory: std::path::PathBuf,
    pub(super) session: CaptureSession,
    pub(super) filter: i32,
    pub(super) query: iriscope_core::library::LibraryQuery,
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
    pub(super) query: iriscope_core::library::LibraryQuery,
    pub(super) page: usize,
    pub(super) revision: u64,
    pub(super) cache_revision: u64,
    pub(super) closed: bool,
    visible_range: (usize, usize),
    thumbnails_pending: bool,
}

pub(super) enum LibraryWork {
    Refresh(LibraryRefreshRequest),
    Thumbnails(usize, usize),
}

struct ActiveThumbnailPage {
    revision: u64,
    items: Vec<crate::library_ui::LibraryItemPayload>,
    attempted: Vec<bool>,
}

impl ActiveThumbnailPage {
    fn load_range(
        &mut self,
        first: usize,
        end: usize,
        mailbox: &Arc<LibraryRefreshMailbox>,
        weak: &slint::Weak<MainWindow>,
        cache: &mut ThumbnailCache,
    ) {
        for index in first..end.min(self.items.len()) {
            if !mailbox.is_current(self.revision) {
                break;
            }
            if self.attempted[index] || !mailbox.is_visible(index) {
                continue;
            }
            let item = &self.items[index];
            let pixels = load_payload_thumbnail(item, cache, &|| {
                mailbox.is_current(self.revision) && mailbox.is_visible(index)
            });
            if !mailbox.is_current(self.revision) || !mailbox.is_visible(index) {
                continue;
            }
            self.attempted[index] = true;
            let Some(pixels) = pixels else {
                if std::env::var_os("IRISCOPE_TRACE_LIBRARY").is_some() {
                    eprintln!("[IrisScope] Miniature {index} indisponible");
                }
                continue;
            };
            let path = item.file_path.clone();
            let version = item.file_version.clone();
            let revision = self.revision;
            let mailbox = Arc::clone(mailbox);
            let _ = weak.upgrade_in_event_loop(move |win| {
                if !mailbox.is_current(revision) || win.get_library_loading() {
                    return;
                }
                let model = win.get_library_items();
                if let Some(mut row) = model.row_data(index)
                    && row.file_path.as_str() == path
                    && row.file_version.as_str() == version
                {
                    row.thumbnail = slint::Image::from_rgb8(pixels);
                    row.has_thumbnail = true;
                    model.set_row_data(index, row);
                    if std::env::var_os("IRISCOPE_TRACE_LIBRARY").is_some() {
                        eprintln!("[IrisScope] Miniature {index} affichée");
                    }
                }
            });
        }
    }
}

impl LibraryRefreshMailbox {
    pub(super) fn set_query(&self, query: iriscope_core::library::LibraryQuery) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.query = query;
        state.page = 0;
    }
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
            query: state.query.clone(),
            page: state.page,
            revision: state.revision,
            cache_revision: state.cache_revision,
        });
        self.ready.notify_one();
    }

    #[cfg(test)]
    pub(super) fn receive(&self) -> Option<LibraryRefreshRequest> {
        match self.receive_work()? {
            LibraryWork::Refresh(request) => Some(request),
            LibraryWork::Thumbnails(_, _) => None,
        }
    }

    pub(super) fn set_visible_range(&self, first: i32, end: i32) {
        let range = (
            usize::try_from(first).unwrap_or(0).min(LIBRARY_PAGE_SIZE),
            usize::try_from(end).unwrap_or(0).min(LIBRARY_PAGE_SIZE),
        );
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.visible_range == range {
            return;
        }
        state.visible_range = range;
        state.thumbnails_pending = true;
        self.ready.notify_one();
    }

    fn is_visible(&self, index: usize) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        index >= state.visible_range.0 && index < state.visible_range.1
    }

    pub(super) fn receive_work(&self) -> Option<LibraryWork> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.pending.is_none() && !state.thumbnails_pending && !state.closed {
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if state.closed {
            return None;
        }
        if let Some(request) = state.pending.take() {
            return Some(LibraryWork::Refresh(request));
        }
        state.thumbnails_pending = false;
        Some(LibraryWork::Thumbnails(
            state.visible_range.0,
            state.visible_range.1,
        ))
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
        state.thumbnails_pending = true;
        self.ready.notify_one();
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

#[allow(clippy::too_many_lines)] // Keep generation validation beside the UI publication.
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
    let mut active_page: Option<ActiveThumbnailPage> = None;
    while let Some(work) = mailbox.receive_work() {
        let request = match work {
            LibraryWork::Refresh(request) => request,
            LibraryWork::Thumbnails(first, end) => {
                if let Some(page) = active_page.as_mut() {
                    page.load_range(first, end, mailbox, weak, &mut cache);
                }
                continue;
            }
        };
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
        cache.set_query(request.query.clone());
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
        let Some(mut result) = load_library_metadata(
            &request.directory,
            &request.session,
            request.filter,
            request.page,
            &mut cache,
            || mailbox.is_current(request.revision),
        ) else {
            continue;
        };
        result.error = result.error.or(recovery_error);
        if !mailbox.is_current(request.revision) {
            continue;
        }
        active_page = Some(ActiveThumbnailPage {
            revision: request.revision,
            attempted: vec![false; result.items.len()],
            items: result.items.clone(),
        });
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
            let summary = library_page_summary(result.page, result.total);
            win.set_library_items(ModelRc::new(VecModel::from(present_library_payloads(
                result.items,
            ))));
            win.set_library_total_count(i32::try_from(result.total).unwrap_or(i32::MAX));
            win.set_library_page(i32::try_from(result.page).unwrap_or(i32::MAX));
            win.set_library_page_summary(summary.into());
            win.set_library_error(result.error.unwrap_or_default().into());
            win.set_library_loading(false);
            crate::viewer_controller::continue_navigation(&win);
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

fn library_page_summary(page: usize, total: usize) -> String {
    let first = page * LIBRARY_PAGE_SIZE + usize::from(total > 0);
    let last = ((page + 1) * LIBRARY_PAGE_SIZE).min(total);
    format!("{first}–{last} sur {total}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_thumbnail_work_is_coalesced_and_refresh_has_priority() {
        let mailbox = LibraryRefreshMailbox::default();
        mailbox.set_visible_range(0, 10);
        mailbox.set_visible_range(20, 30);
        mailbox.set_visible_range(-1, i32::MAX);
        assert!(matches!(
            mailbox.receive_work(),
            Some(LibraryWork::Thumbnails(0, LIBRARY_PAGE_SIZE))
        ));
        mailbox.set_visible_range(40, 50);
        mailbox.request(
            std::path::PathBuf::from("library"),
            CaptureSession::default(),
        );
        assert!(matches!(
            mailbox.receive_work(),
            Some(LibraryWork::Refresh(_))
        ));
        assert!(matches!(
            mailbox.receive_work(),
            Some(LibraryWork::Thumbnails(40, 50))
        ));
        mailbox.close();
        assert!(mailbox.receive_work().is_none());
    }
}
