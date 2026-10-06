//! Dossier consultation view with verified photos and durable general notes.
use crate::{
    runtime::AppRuntime,
    ui::{AppState, ConsultationDayData, MainWindow, PatientCandidateData, SessionPhotoData},
};
use iriscope_core::library::{self, CaptureFileVersion, LibraryEntry};
use slint::{ComponentHandle, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
pub(super) fn french_date(value: &str) -> String {
    crate::date_controller::parse(value)
        .map_or_else(|| value.to_owned(), |d| d.format("%d/%m/%Y").to_string())
}
pub(super) fn local_saved_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value).map_or_else(
        |_| String::new(),
        |d| {
            d.with_timezone(&chrono::Local)
                .format("%d/%m/%Y à %H:%M")
                .to_string()
        },
    )
}
pub(super) fn thumbnail(
    path: &Path,
    version: &CaptureFileVersion,
    cancel: &dyn Fn() -> bool,
) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
    let (w, h, rgb) =
        crate::playback::read_verified_photo_cancellable(path, version, cancel).ok()??;
    let (w, h, rgb) = iriscope_imaging::resize_rgb8_to_fit(&rgb, w, h, 160).ok()?;
    Some(SharedPixelBuffer::clone_from_slice(&rgb, w, h))
}
fn verified_entries(
    directory: &Path,
    id: u64,
    cancel: &dyn Fn() -> bool,
) -> io::Result<Vec<LibraryEntry>> {
    let candidates = library::try_scan_library_directory_metadata_cancellable(directory, cancel)?;
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|c| c.patient_id_hint == Some(id))
        .collect();
    Ok(
        library::resolve_library_candidates_cancellable(directory, &candidates, cancel)?
            .into_iter()
            .filter(|e| e.patient_id == Some(id))
            .collect(),
    )
}
fn day_model(
    directory: &Path,
    id: u64,
    cancel: &dyn Fn() -> bool,
) -> io::Result<Vec<ConsultationDayData>> {
    let mut days: BTreeMap<String, (i32, i32, i32)> = BTreeMap::new();
    for entry in verified_entries(directory, id, cancel)? {
        if crate::date_controller::parse(&entry.date_str).is_none() {
            continue;
        }
        let count = days.entry(entry.date_str).or_default();
        if entry.kind == library::CaptureKind::Video {
            count.2 += 1;
        } else if entry.eye == iriscope_core::session::Eye::Left {
            count.0 += 1;
        } else if entry.eye == iriscope_core::session::Eye::Right {
            count.1 += 1;
        }
    }
    for day in library::consultation_dates(directory, id)? {
        days.entry(day).or_default();
    }
    days.entry(chrono::Local::now().format("%Y-%m-%d").to_string())
        .or_default();
    Ok(days
        .into_iter()
        .rev()
        .map(|(date, (left, right, videos))| ConsultationDayData {
            label: french_date(&date).into(),
            date: date.into(),
            left,
            right,
            videos,
        })
        .collect())
}
#[derive(Clone)]
struct PhotoRow {
    path: String,
    version: String,
    caption: String,
    eye: i32,
    retained: bool,
    thumbnail: Option<SharedPixelBuffer<Rgb8Pixel>>,
}
fn photo_rows(
    directory: &Path,
    id: u64,
    day: &str,
    cancel: &dyn Fn() -> bool,
) -> io::Result<Vec<PhotoRow>> {
    let mut entries = verified_entries(directory, id, cancel)?;
    entries.retain(|e| e.date_str == day && e.kind == library::CaptureKind::Photo);
    entries.sort_by_key(|e| std::cmp::Reverse(e.time_str.clone()));
    entries.truncate(200);
    Ok(entries
        .into_iter()
        .filter_map(|e| {
            let version = e.file_version?;
            let retained = library::load_photo_review(&e.file_path, &version, cancel)
                .map(|r| r.retained)
                .unwrap_or(false);
            let image = thumbnail(&e.file_path, &version, cancel);
            Some(PhotoRow {
                path: e.file_path.to_string_lossy().into_owned(),
                version: version.token(),
                caption: format!(
                    "{} · {}{}",
                    e.time_str,
                    e.eye.filename_label(),
                    if retained { " · Retenue" } else { "" }
                ),
                eye: match e.eye {
                    iriscope_core::session::Eye::Left => 1,
                    iriscope_core::session::Eye::Right => 2,
                    iriscope_core::session::Eye::Unspecified => 0,
                },
                retained,
                thumbnail: image,
            })
        })
        .collect())
}
fn to_model(rows: Vec<PhotoRow>) -> ModelRc<SessionPhotoData> {
    ModelRc::new(VecModel::from(
        rows.into_iter()
            .map(|r| SessionPhotoData {
                path: r.path.into(),
                version: r.version.into(),
                caption: r.caption.into(),
                eye: r.eye,
                retained: r.retained,
                thumbnail: r
                    .thumbnail
                    .map_or_else(slint::Image::default, slint::Image::from_rgb8),
            })
            .collect::<Vec<_>>(),
    ))
}
fn bind_search(window: &MainWindow, runtime: &AppRuntime, generation: &Arc<AtomicU64>) {
    let weak = window.as_weak();
    let settings = Arc::clone(&runtime.settings);
    let jobs = Arc::clone(&runtime.background_jobs);
    let generation = Arc::clone(generation);
    window.global::<AppState>().on_search_dossiers(move || {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let s = win.global::<AppState>();
        let query = s.get_dossier_search().to_string();
        let directory = crate::app_helpers::settings_snapshot(&settings).capture_directory;
        let weak = weak.clone();
        let gen_id = generation.fetch_add(1, Ordering::AcqRel) + 1;
        let generation = Arc::clone(&generation);
        s.set_dossier_search_busy(true);
        if !jobs.submit(move || {
            let result = library::search_dossiers(&directory, &query);
            let _ = weak.upgrade_in_event_loop(move |win| {
                if generation.load(Ordering::Acquire) != gen_id {
                    return;
                }
                let s = win.global::<AppState>();
                s.set_dossier_search_busy(false);
                match result {
                    Ok(rows) => s.set_dossier_candidates(ModelRc::new(VecModel::from(
                        rows.into_iter()
                            .map(|p| PatientCandidateData {
                                id: p.id.to_string().into(),
                                dossier_number: p.dossier_number.into(),
                                first_name: p.first_name.into(),
                                last_name: p.last_name.into(),
                                last_capture: p.last_capture.unwrap_or_default().into(),
                            })
                            .collect::<Vec<_>>(),
                    ))),
                    Err(e) => {
                        s.set_consultation_feedback(format!("Recherche impossible : {e}").into());
                    }
                }
            });
        }) {
            s.set_dossier_search_busy(false);
            s.set_consultation_feedback("Traitement disque en cours. Réessayez.".into());
        }
    });
}
fn bind_dossier(window: &MainWindow, runtime: &AppRuntime, generation: &Arc<AtomicU64>) {
    let weak = window.as_weak();
    let settings = Arc::clone(&runtime.settings);
    let jobs = Arc::clone(&runtime.background_jobs);
    let generation = Arc::clone(generation);
    let closing = Arc::clone(&runtime.closing);
    window.global::<AppState>().on_select_dossier(move |value| {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let s = win.global::<AppState>();
        if s.get_consultation_dirty() || s.get_consultation_saving() {
            s.set_consultation_feedback(
                "Enregistrez les notes avant de changer de dossier.".into(),
            );
            return;
        }
        let Ok(id) = value.parse::<u64>() else {
            return;
        };
        let directory = crate::app_helpers::settings_snapshot(&settings).capture_directory;
        let gen_id = generation.fetch_add(1, Ordering::AcqRel) + 1;
        let generation = Arc::clone(&generation);
        let weak = weak.clone();
        let closing = Arc::clone(&closing);
        s.set_consultation_loading(true);
        s.set_consultation_patient_id("".into());
        s.set_consultation_day("".into());
        s.set_consultation_notes("".into());
        s.set_consultation_saved_at("".into());
        s.set_consultation_title("".into());
        s.set_consultation_days(ModelRc::default());
        s.set_consultation_photos(ModelRc::default());
        s.set_consultation_left_photos(ModelRc::default());
        s.set_consultation_right_photos(ModelRc::default());
        s.set_consultation_feedback("".into());
        if !jobs.submit(move || {
            let cancel =
                || generation.load(Ordering::Acquire) != gen_id || closing.load(Ordering::Acquire);
            let result = library::get_patient(&directory, id)
                .and_then(|p| p.ok_or_else(|| io::Error::other("Dossier introuvable.")))
                .and_then(|p| Ok((p, day_model(&directory, id, &cancel)?)));
            let _ = weak.upgrade_in_event_loop(move |win| {
                if generation.load(Ordering::Acquire) != gen_id {
                    return;
                }
                let s = win.global::<AppState>();
                s.set_consultation_loading(false);
                match result {
                    Ok((p, days)) => {
                        s.set_consultation_patient_id(id.to_string().into());
                        s.set_consultation_directory(
                            directory.to_string_lossy().into_owned().into(),
                        );
                        s.set_consultation_title(
                            format!("{} {} · {}", p.first_name, p.last_name, p.dossier_number)
                                .into(),
                        );
                        let first = days.first().map(|d| d.date.clone());
                        s.set_consultation_days(ModelRc::new(VecModel::from(days)));
                        if let Some(day) = first {
                            s.invoke_select_consultation_day(day);
                        }
                    }
                    Err(e) => {
                        s.set_consultation_feedback(format!("Ouverture impossible : {e}").into());
                    }
                }
            });
        }) {
            s.set_consultation_loading(false);
            s.set_consultation_feedback("Traitement disque en cours. Réessayez.".into());
        }
    });
}
fn bind_day(window: &MainWindow, runtime: &AppRuntime, generation: &Arc<AtomicU64>) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    let generation = Arc::clone(generation);
    let closing = Arc::clone(&runtime.closing);
    window
        .global::<AppState>()
        .on_select_consultation_day(move |day| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if s.get_consultation_dirty() || s.get_consultation_saving() {
                s.set_consultation_pending_day(day);
                if !s.get_consultation_saving() {
                    s.invoke_save_consultation_notes();
                }
                return;
            }
            let Ok(id) = s.get_consultation_patient_id().parse::<u64>() else {
                return;
            };
            let directory = PathBuf::from(s.get_consultation_directory().as_str());
            let day = day.to_string();
            let gen_id = generation.fetch_add(1, Ordering::AcqRel) + 1;
            let generation = Arc::clone(&generation);
            let closing = Arc::clone(&closing);
            let weak = weak.clone();
            s.set_consultation_loading(true);
            s.set_consultation_photos(ModelRc::default());
            s.set_consultation_left_photos(ModelRc::default());
            s.set_consultation_right_photos(ModelRc::default());
            s.set_consultation_day(day.clone().into());
            s.set_consultation_feedback("".into());
            if !jobs.submit(move || {
                let cancel = || {
                    generation.load(Ordering::Acquire) != gen_id || closing.load(Ordering::Acquire)
                };
                let result = library::load_consultation_notes(&directory, id, &day)
                    .and_then(|notes| Ok((notes, photo_rows(&directory, id, &day, &cancel)?)));
                let _ = weak.upgrade_in_event_loop(move |win| {
                    if generation.load(Ordering::Acquire) != gen_id {
                        return;
                    }
                    let s = win.global::<AppState>();
                    s.set_consultation_loading(false);
                    s.set_consultation_load_error(result.is_err());
                    match result {
                        Ok((notes, rows)) => {
                            s.set_consultation_notes(notes.notes.into());
                            s.set_consultation_saved_at(local_saved_time(&notes.updated_at).into());
                            let left =
                                to_model(rows.iter().filter(|r| r.eye == 1).cloned().collect());
                            let right =
                                to_model(rows.iter().filter(|r| r.eye == 2).cloned().collect());
                            s.set_consultation_left_photos(left);
                            s.set_consultation_right_photos(right);
                            s.set_consultation_photos(to_model(rows));
                        }
                        Err(e) => s.set_consultation_feedback(
                            format!(
                                "Lecture impossible : {e}. Les notes existantes sont conservées."
                            )
                            .into(),
                        ),
                    }
                });
            }) {
                s.set_consultation_loading(false);
                s.set_consultation_load_error(true);
                s.set_consultation_feedback("Traitement disque en cours. Réessayez.".into());
            }
        });
}
fn finish_notes(win: &MainWindow, result: io::Result<()>, revision: i32) {
    let s = win.global::<AppState>();
    s.set_consultation_saving(false);
    if let Err(e) = result {
        s.set_consultation_paused(true);
        s.set_consultation_feedback(
            format!("Enregistrement impossible : {e}. Brouillon conservé.").into(),
        );
        s.set_consultation_close_app_pending(false);
        s.set_consultation_pending_close(false);
        s.set_consultation_pending_day("".into());
        return;
    }
    s.set_consultation_saved_at(chrono::Local::now().format("%H:%M").to_string().into());
    if s.get_consultation_revision() != revision {
        if s.get_consultation_pending_close()
            || s.get_consultation_close_app_pending()
            || !s.get_consultation_pending_day().is_empty()
        {
            s.invoke_save_consultation_notes();
        }
        return;
    }
    s.set_consultation_dirty(false);
    s.set_consultation_feedback("".into());
    if s.get_consultation_close_app_pending() {
        s.set_consultation_close_app_pending(false);
        win.window()
            .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    } else if s.get_consultation_pending_close() {
        s.set_consultation_pending_close(false);
        s.invoke_close_consultation();
    } else if !s.get_consultation_pending_day().is_empty() {
        let day = s.get_consultation_pending_day();
        s.set_consultation_pending_day("".into());
        s.invoke_select_consultation_day(day);
    }
}
fn bind_save(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    window.global::<AppState>().on_consultation_edited(move || {
        if let Some(win) = weak.upgrade() {
            let s = win.global::<AppState>();
            s.set_consultation_revision(s.get_consultation_revision().wrapping_add(1));
            s.set_consultation_dirty(true);
            s.set_consultation_paused(false);
        }
    });
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    window
        .global::<AppState>()
        .on_save_consultation_notes(move || {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if !s.get_consultation_dirty()
                || s.get_consultation_saving()
                || s.get_consultation_load_error()
            {
                return;
            }
            let Ok(id) = s.get_consultation_patient_id().parse::<u64>() else {
                return;
            };
            let directory = PathBuf::from(s.get_consultation_directory().as_str());
            let day = s.get_consultation_day().to_string();
            let notes = s.get_consultation_notes().to_string();
            let revision = s.get_consultation_revision();
            let weak = weak.clone();
            s.set_consultation_saving(true);
            if !jobs.submit(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let result =
                    library::save_consultation_notes(&directory, id, &day, &notes, &|| {
                        std::time::Instant::now() >= deadline
                    });
                let _ = weak.upgrade_in_event_loop(move |win| finish_notes(&win, result, revision));
            }) {
                finish_notes(
                    &win,
                    Err(io::Error::other("Traitement disque en cours. Réessayez.")),
                    revision,
                );
            }
        });
}
pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    let generation = Arc::new(AtomicU64::new(0));
    bind_search(window, runtime, &Arc::new(AtomicU64::new(0)));
    bind_dossier(window, runtime, &generation);
    bind_day(window, runtime, &generation);
    bind_save(window, runtime);
    let weak = window.as_weak();
    window.global::<AppState>().on_open_consultation(move || {
        if let Some(win) = weak.upgrade() {
            let s = win.global::<AppState>();
            s.set_consultation_open(true);
            if !s.get_consultation_dirty() && !s.get_patient_id().is_empty() {
                s.set_dossier_search(s.get_patient_dossier_number());
                s.invoke_search_dossiers();
                s.invoke_select_dossier(s.get_patient_id());
            }
        }
    });
    let weak = window.as_weak();
    window.global::<AppState>().on_close_consultation(move || {
        if let Some(win) = weak.upgrade() {
            let s = win.global::<AppState>();
            if s.get_consultation_dirty() || s.get_consultation_saving() {
                s.set_consultation_pending_close(true);
                if !s.get_consultation_saving() {
                    s.invoke_save_consultation_notes();
                }
                return;
            }
            generation.fetch_add(1, Ordering::AcqRel);
            s.set_consultation_open(false);
            s.set_consultation_loading(false);
            s.invoke_focus_main_controls();
        }
    });
}
