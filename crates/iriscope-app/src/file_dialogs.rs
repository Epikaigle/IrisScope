//! Native path pickers run outside the Slint event loop.
use crate::{
    app_helpers::{settings_error, settings_snapshot},
    runtime::AppRuntime,
    ui::{AppState, MainWindow},
};
use slint::ComponentHandle;
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    let settings = Arc::clone(&runtime.settings);
    let closing = Arc::clone(&runtime.closing);
    window.global::<AppState>().on_browse_settings_path(move |kind| {
        let Some(win) = weak.upgrade() else { return; };
        let state = win.global::<AppState>();
        if state.get_file_dialog_pending() || closing.load(Ordering::Acquire)
            || !(0..=2).contains(&kind) || (kind == 0 && state.get_recording_busy()) { return; }
        let snapshot = settings_snapshot(&settings);
        let initial = match kind {
            0 => snapshot.capture_directory,
            1 => snapshot.iridology_map_path.unwrap_or(snapshot.capture_directory),
            _ => snapshot.iridology_symbols_path.unwrap_or(snapshot.capture_directory),
        };
        state.set_file_dialog_pending(true);
        let weak = weak.clone();
        let closing = Arc::clone(&closing);
        if !jobs.submit(move || {
            let result = std::panic::catch_unwind(|| pick_path(kind, &initial));
            let _ = weak.upgrade_in_event_loop(move |win| {
                let state = win.global::<AppState>();
                state.set_file_dialog_pending(false);
                if closing.load(Ordering::Acquire) { return; }
                match result {
                    Ok(Some(path)) => {
                        let Some(path) = path.to_str() else {
                            settings_error(&win, "Ce chemin contient des caractères non pris en charge.");
                            return;
                        };
                        match kind {
                            0 => state.invoke_update_capture_directory(path.into()),
                            1 => state.invoke_update_iridology_map_path(path.into()),
                            _ => state.invoke_update_iridology_symbols_path(path.into()),
                        }
                    }
                    Ok(None) => {},
                    Err(_) => settings_error(&win, "Impossible d’ouvrir le sélecteur de fichiers. Vous pouvez saisir le chemin."),
                }
            });
        }) {
            state.set_file_dialog_pending(false);
            settings_error(&win, "Le sélecteur est indisponible pendant une autre opération. Réessayez.");
        }
    });
}

fn pick_path(kind: i32, initial: &Path) -> Option<std::path::PathBuf> {
    let directory = if initial.is_dir() {
        initial.to_path_buf()
    } else {
        initial.parent()?.to_path_buf()
    };
    let dialog = rfd::FileDialog::new().set_directory(directory);
    if kind == 0 {
        dialog
            .set_title("IrisScope — Dossier des captures")
            .pick_folder()
    } else {
        dialog
            .set_title(if kind == 1 {
                "IrisScope — Carte d’iridologie"
            } else {
                "IrisScope — Signes et symboles"
            })
            .add_filter("Images JPEG et PNG", &["jpg", "jpeg", "png"])
            .pick_file()
    }
}
