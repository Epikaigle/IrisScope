//! User-selected MP4 exports through a background job, with cancellable progress.
use crate::{
    operation_progress::ProgressReporter,
    runtime::AppRuntime,
    ui::{AppState, MainWindow},
};
use iriscope_core::library::CaptureFileVersion;
use slint::ComponentHandle;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

fn apply_progress(win: &MainWindow, percent: i32) {
    let state = win.global::<AppState>();
    if state.get_export_busy() {
        state.set_export_progress_known(true);
        state.set_export_progress(percent);
        state.set_export_feedback(
            format!("Export MP4 : {percent} % · vidéo originale conservée").into(),
        );
    }
}
fn finish(win: &MainWindow, result: io::Result<Option<PathBuf>>) {
    let state = win.global::<AppState>();
    state.set_export_busy(false);
    state.set_export_cancelling(false);
    state.set_file_dialog_pending(false);
    state.set_export_feedback_error(result.is_err());
    state.set_export_feedback(
        match result {
            Ok(Some(path)) => {
                state.set_export_progress(100);
                format!("Export MP4 terminé : {}", path.display())
            }
            Ok(None) => "Export annulé. La vidéo originale est conservée.".into(),
            Err(error) => format!("Export MP4 impossible : {error}"),
        }
        .into(),
    );
}

struct ExportJob {
    window: slint::Weak<MainWindow>,
    source: PathBuf,
    expected: CaptureFileVersion,
    control: Arc<crate::operation_progress::OperationControl>,
    closing: Arc<std::sync::atomic::AtomicBool>,
    generation: u64,
}
impl ExportJob {
    fn run(&self) -> io::Result<Option<PathBuf>> {
        if self.closing.load(Ordering::Acquire) || self.control.is_cancelled(self.generation) {
            return Ok(None);
        }
        let filename = self.source.with_extension("mp4").file_name().map_or_else(
            || "IrisScope.mp4".into(),
            |name| name.to_string_lossy().into_owned(),
        );
        let dialog = rfd::FileDialog::new()
            .set_title("IrisScope — Exporter une copie MP4")
            .set_file_name(filename)
            .add_filter("Vidéo MP4", &["mp4"]);
        let dialog = self.source.parent().map_or_else(
            || dialog.clone(),
            |parent| dialog.clone().set_directory(parent),
        );
        let Some(destination) = dialog.save_file() else {
            return Ok(None);
        };
        let _ = self.window.upgrade_in_event_loop(|win| {
            win.global::<AppState>().set_file_dialog_pending(false);
            win.global::<AppState>()
                .set_export_feedback("Vérification de la vidéo et préparation de l’export…".into());
        });
        let cancel =
            || self.closing.load(Ordering::Acquire) || self.control.is_cancelled(self.generation);
        let reporter = ProgressReporter::new(
            self.window.clone(),
            Arc::clone(&self.control),
            self.generation,
            apply_progress,
        );
        match crate::video_export::export_mp4(
            &self.source,
            &self.expected,
            &destination,
            &cancel,
            &mut |percent| {
                reporter.report(percent);
            },
        ) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted && cancel() => Ok(None),
            result => result.map(Some),
        }
    }
    fn execute(self) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run()))
            .unwrap_or_else(|_| Err(io::Error::other("L’export vidéo n’a pas pu se terminer.")));
        self.control.finish(self.generation);
        let _ = self
            .window
            .upgrade_in_event_loop(move |win| finish(&win, result));
    }
}
fn bind_start(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    let control = Arc::clone(&runtime.export_operation);
    let closing = Arc::clone(&runtime.closing);
    window.global::<AppState>().on_export_viewer_mp4(move || {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let state = win.global::<AppState>();
        if !state.get_viewer_open()
            || !state.get_viewer_is_video()
            || state.get_viewer_loading()
            || state.get_export_busy()
            || state.get_storage_busy()
            || state.get_file_dialog_pending()
            || closing.load(Ordering::Acquire)
        {
            return;
        }
        let Ok(expected) = CaptureFileVersion::from_token(state.get_viewer_file_version().as_str())
        else {
            state.set_export_feedback_error(true);
            state.set_export_feedback(
                "Rouvrez la vidéo depuis la bibliothèque avant de l’exporter.".into(),
            );
            return;
        };
        let generation = control.begin();
        state.set_export_busy(true);
        state.set_export_cancelling(false);
        state.set_export_progress(0);
        state.set_export_progress_known(false);
        state.set_export_feedback_error(false);
        state.set_export_feedback("Choisissez le nom et l’emplacement de la copie MP4.".into());
        state.set_file_dialog_pending(true);
        let job = ExportJob {
            window: weak.clone(),
            source: PathBuf::from(state.get_viewer_path().as_str()),
            expected,
            control: Arc::clone(&control),
            closing: Arc::clone(&closing),
            generation,
        };
        if !jobs.submit(move || job.execute()) {
            control.finish(generation);
            finish(
                &win,
                Err(io::Error::other(
                    "Traitement disque en cours. Réessayez dans un instant.",
                )),
            );
        }
    });
}
pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    bind_start(window, runtime);
    let weak = window.as_weak();
    let control = Arc::clone(&runtime.export_operation);
    window.global::<AppState>().on_cancel_video_export(move || {
        if let Some(win) = weak.upgrade()
            && win.global::<AppState>().get_export_busy()
        {
            control.cancel();
            win.global::<AppState>().set_export_cancelling(true);
            win.global::<AppState>()
                .set_export_feedback("Annulation et nettoyage de l’export incomplet…".into());
        }
    });
}
