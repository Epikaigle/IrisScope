//! Backup and restore orchestration: selectors and disk work stay off the UI thread.
use crate::{
    app_helpers::settings_snapshot,
    background::BackgroundJobs,
    camera_queue::CameraSettingsSaveMailbox,
    operation_progress::{OperationControl, ProgressReporter},
    photo_worker::PhotoMailbox,
    runtime::AppRuntime,
    ui::{AppState, MainWindow},
};
use iriscope_core::{
    library::{
        BackupPhase, BackupProgress, BackupReport, backup_library_with_progress,
        restore_library_with_progress,
    },
    settings::{AppSettings, SuccessfulBackup},
};
use slint::ComponentHandle;
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn refresh_backup_status(win: &MainWindow, settings: &AppSettings) {
    let state = win.global::<AppState>();
    state.set_backup_reminder_enabled(settings.backup_reminder_enabled);
    let text = settings.last_backup().map_or_else(
        || "Aucune sauvegarde enregistrée pour ce dossier.".to_owned(),
        |backup| {
            let date = i64::try_from(backup.completed_at_unix)
                .ok()
                .and_then(|stamp| chrono::DateTime::from_timestamp(stamp, 0))
                .map_or_else(
                    || "date inconnue".into(),
                    |date| {
                        date.with_timezone(&chrono::Local)
                            .format("%d/%m/%Y à %H:%M")
                            .to_string()
                    },
                );
            format!(
                "Dernière sauvegarde réussie : {date} · {} fichiers\n{}",
                backup.files,
                backup.destination.display()
            )
        },
    );
    state.set_backup_last_status(text.into());
    state.set_backup_reminder(
        if settings.backup_reminder_due(now_unix()) {
            if settings.last_backup().is_some() {
                "Votre dernière sauvegarde date d’au moins 7 jours."
            } else {
                "Ce dossier n’a pas encore de sauvegarde enregistrée."
            }
        } else {
            ""
        }
        .into(),
    );
}

fn apply_progress(win: &MainWindow, progress: BackupProgress) {
    let state = win.global::<AppState>();
    if !state.get_storage_busy() {
        return;
    }
    state.set_storage_progress_known(progress.phase != BackupPhase::Preparing);
    let percent = match progress.phase {
        BackupPhase::Preparing => 0,
        BackupPhase::Copying => i32::try_from(
            u128::from(progress.bytes_completed) * 90 / u128::from(progress.bytes_total.max(1)),
        )
        .unwrap_or(90),
        BackupPhase::Verifying => 95,
        BackupPhase::Finalizing => 99,
        BackupPhase::Complete => 100,
    };
    state.set_storage_progress(percent.clamp(0, 100));
    let message = match progress.phase {
        BackupPhase::Preparing => "Préparation et vérification des fichiers…".into(),
        BackupPhase::Copying => format!(
            "Copie vérifiée : {} / {} fichiers · {} / {} Mio",
            progress.files_completed,
            progress.files_total,
            progress.bytes_completed / 1_048_576,
            progress.bytes_total / 1_048_576
        ),
        BackupPhase::Verifying => "Vérification finale du dossier source…".into(),
        BackupPhase::Finalizing => "Finalisation de la copie vérifiée…".into(),
        BackupPhase::Complete => "Copie vérifiée terminée.".into(),
    };
    state.set_storage_feedback(message.into());
}

#[derive(Clone)]
struct StorageController {
    window: slint::Weak<MainWindow>,
    settings: Arc<Mutex<AppSettings>>,
    jobs: Arc<BackgroundJobs>,
    photos: Arc<PhotoMailbox>,
    closing: Arc<AtomicBool>,
    control: Arc<OperationControl>,
    save: Arc<CameraSettingsSaveMailbox>,
}
impl StorageController {
    fn start(&self, restore: bool) {
        let Some(win) = self.window.upgrade() else {
            return;
        };
        let state = win.global::<AppState>();
        if state.get_storage_busy()
            || state.get_export_busy()
            || state.get_file_dialog_pending()
            || state.get_recording_busy()
            || state.get_patient_action_pending()
            || self.closing.load(Ordering::Acquire)
        {
            return;
        }
        if self.photos.work_count() > 0 {
            state.set_storage_feedback_error(true);
            state.set_storage_feedback(
                "Attendez la fin de l’enregistrement des photos avant de continuer.".into(),
            );
            return;
        }
        let initial = settings_snapshot(&self.settings).capture_directory;
        let generation = self.control.begin();
        state.set_storage_busy(true);
        state.set_file_dialog_pending(true);
        state.set_storage_cancelling(false);
        state.set_storage_progress(0);
        state.set_storage_progress_known(false);
        state.set_storage_feedback_error(false);
        state.set_storage_feedback(
            if restore {
                "Choisissez une sauvegarde puis son dossier de destination."
            } else {
                "Choisissez où créer la sauvegarde complète."
            }
            .into(),
        );
        let controller = self.clone();
        if !self.jobs.submit(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                controller.run(restore, initial, generation)
            }))
            .unwrap_or_else(|_| {
                Err(io::Error::other(
                    "L’opération de copie n’a pas pu se terminer.",
                ))
            });
            controller.control.finish(generation);
            let window = controller.window.clone();
            let _ =
                window.upgrade_in_event_loop(move |win| controller.finish(&win, restore, result));
        }) {
            self.control.finish(generation);
            state.set_storage_busy(false);
            state.set_file_dialog_pending(false);
            state.set_storage_feedback_error(true);
            state.set_storage_feedback(
                "Traitement disque en cours. Réessayez dans un instant.".into(),
            );
        }
    }
    fn run(
        &self,
        restore: bool,
        initial: PathBuf,
        generation: u64,
    ) -> io::Result<Option<(PathBuf, BackupReport)>> {
        if self.closing.load(Ordering::Acquire) || self.control.is_cancelled(generation) {
            return Ok(None);
        }
        let dialog = rfd::FileDialog::new().set_directory(&initial);
        let source = if restore {
            let Some(source) = dialog
                .clone()
                .set_title("IrisScope — Sauvegarde à restaurer")
                .pick_folder()
            else {
                return Ok(None);
            };
            source
        } else {
            initial
        };
        if self.closing.load(Ordering::Acquire) || self.control.is_cancelled(generation) {
            return Ok(None);
        }
        let Some(parent) = dialog
            .set_title(if restore {
                "IrisScope — Emplacement du nouveau dossier restauré"
            } else {
                "IrisScope — Emplacement de la sauvegarde"
            })
            .pick_folder()
        else {
            return Ok(None);
        };
        let _ = self
            .window
            .upgrade_in_event_loop(|win| win.global::<AppState>().set_file_dialog_pending(false));
        let cancel =
            || self.closing.load(Ordering::Acquire) || self.control.is_cancelled(generation);
        let reporter = ProgressReporter::new(
            self.window.clone(),
            Arc::clone(&self.control),
            generation,
            apply_progress,
        );
        let result = if restore {
            restore_library_with_progress(&source, &parent, &cancel, &mut |value| {
                reporter.report(value);
            })
        } else {
            backup_library_with_progress(&source, &parent, &cancel, &mut |value| {
                reporter.report(value);
            })
        };
        match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted && cancel() => Ok(None),
            result => result.map(|report| {
                if !restore && let Ok(mut settings) = self.settings.lock() {
                    settings.record_backup(SuccessfulBackup {
                        source: source.clone(),
                        destination: report.directory.clone(),
                        completed_at_unix: now_unix(),
                        files: report.files,
                        bytes: report.bytes,
                    });
                    self.save.mark_dirty();
                }
                Some((source, report))
            }),
        }
    }
    fn finish(
        &self,
        win: &MainWindow,
        restore: bool,
        result: io::Result<Option<(PathBuf, BackupReport)>>,
    ) {
        let state = win.global::<AppState>();
        state.set_storage_busy(false);
        state.set_file_dialog_pending(false);
        state.set_storage_cancelling(false);
        match result {
            Ok(Some((_source, report))) => {
                state.set_storage_progress(100);
                state.set_storage_feedback_error(false);
                state.set_storage_feedback(
                    format!(
                        "{} : {} fichiers · {} Mio · {}",
                        if restore {
                            "Restauration vérifiée"
                        } else {
                            "Sauvegarde terminée"
                        },
                        report.files,
                        report.bytes / 1_048_576,
                        report.directory.display()
                    )
                    .into(),
                );
                if restore {
                    if let Some(path) = report.directory.to_str() {
                        state.invoke_update_capture_directory(path.into());
                    } else {
                        state.set_storage_feedback("Restauration terminée. Le chemin ne peut pas être ouvert dans IrisScope.".into());
                    }
                } else if let Ok(settings) = self.settings.lock() {
                    refresh_backup_status(win, &settings);
                }
            }
            Ok(None) => {
                state.set_storage_feedback_error(false);
                state.set_storage_feedback(
                    "Opération annulée. Les captures actuelles sont conservées.".into(),
                );
            }
            Err(error) => {
                state.set_storage_feedback_error(true);
                state.set_storage_feedback(
                    format!(
                        "{} impossible : {error}",
                        if restore {
                            "Restauration"
                        } else {
                            "Sauvegarde"
                        }
                    )
                    .into(),
                );
            }
        }
    }
}

pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    window
        .global::<AppState>()
        .set_app_version(env!("CARGO_PKG_VERSION").into());
    refresh_backup_status(window, &settings_snapshot(&runtime.settings));
    let controller = StorageController {
        window: window.as_weak(),
        settings: Arc::clone(&runtime.settings),
        jobs: Arc::clone(&runtime.background_jobs),
        photos: Arc::clone(&runtime.photo_mailbox),
        closing: Arc::clone(&runtime.closing),
        control: Arc::clone(&runtime.storage_operation),
        save: Arc::clone(&runtime.camera_settings_save),
    };
    for restore in [false, true] {
        let controller = controller.clone();
        if restore {
            window
                .global::<AppState>()
                .on_restore_library(move || controller.start(true));
        } else {
            window
                .global::<AppState>()
                .on_backup_library(move || controller.start(false));
        }
    }
    let weak = window.as_weak();
    let cancel = Arc::clone(&runtime.storage_operation);
    window
        .global::<AppState>()
        .on_cancel_storage_operation(move || {
            if let Some(win) = weak.upgrade()
                && win.global::<AppState>().get_storage_busy()
            {
                cancel.cancel();
                win.global::<AppState>().set_storage_cancelling(true);
                win.global::<AppState>()
                    .set_storage_feedback("Annulation et nettoyage de la copie incomplète…".into());
            }
        });
    let weak = window.as_weak();
    let settings = Arc::clone(&runtime.settings);
    let save = Arc::clone(&runtime.camera_settings_save);
    window
        .global::<AppState>()
        .on_set_backup_reminder(move |enabled| {
            if let Some(win) = weak.upgrade()
                && let Ok(mut settings) = settings.lock()
            {
                settings.backup_reminder_enabled = enabled;
                refresh_backup_status(&win, &settings);
                save.mark_dirty();
            }
        });
}
