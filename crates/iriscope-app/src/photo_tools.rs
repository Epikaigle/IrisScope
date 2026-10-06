//! Photo observations, display-only transforms and verified comparison selection.
use crate::{
    runtime::AppRuntime,
    ui::{AnnotationData, AppState, ComparisonCandidateData, MainWindow},
};
use iriscope_core::library::{Annotation, AnnotationKind, CaptureFileVersion, PhotoReview};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) fn snapshot(win: &MainWindow) -> PhotoReview {
    let state = win.global::<AppState>();
    PhotoReview {
        notes: state.get_viewer_notes().to_string(),
        annotations: state
            .get_viewer_annotations()
            .iter()
            .map(|a| Annotation {
                kind: match a.kind {
                    1 => AnnotationKind::Circle,
                    2 => AnnotationKind::Arrow,
                    3 => AnnotationKind::Point,
                    _ => AnnotationKind::Text,
                },
                x: a.x,
                y: a.y,
                end_x: a.end_x,
                end_y: a.end_y,
                text: a.text.to_string(),
            })
            .collect(),
        retained: state.get_viewer_retained(),
        updated_at: String::new(),
    }
}
fn model(review: &PhotoReview) -> ModelRc<AnnotationData> {
    ModelRc::new(VecModel::from(
        review
            .annotations
            .iter()
            .map(|a| AnnotationData {
                kind: match a.kind {
                    AnnotationKind::Circle => 1,
                    AnnotationKind::Arrow => 2,
                    AnnotationKind::Point => 3,
                    AnnotationKind::Text => 4,
                },
                x: a.x,
                y: a.y,
                end_x: a.end_x,
                end_y: a.end_y,
                text: a.text.clone().into(),
            })
            .collect::<Vec<_>>(),
    ))
}
pub(super) fn reset(win: &MainWindow) {
    let s = win.global::<AppState>();
    s.set_viewer_fit(true);
    s.set_viewer_panel(0);
    s.invoke_show_photo_controls();
    s.set_viewer_loupe(false);
    s.set_viewer_tool(0);
    s.set_viewer_can_undo(false);
    s.set_viewer_original(slint::Image::default());
    s.set_viewer_brightness(0);
    s.set_viewer_contrast(0);
    s.set_viewer_rotation(0);
    s.set_viewer_mirror(false);
    s.set_viewer_show_original(false);
    s.set_viewer_adjusting(false);
    s.set_viewer_guides(false);
    s.set_viewer_guide_x(0.5);
    s.set_viewer_guide_y(0.5);
    s.set_viewer_guide_radius(0.3);
    s.set_viewer_retained(false);
    s.set_viewer_saved_at("".into());
    s.set_viewer_notes("".into());
    s.set_viewer_annotations(ModelRc::default());
    s.set_viewer_annotations_visible(true);
    s.set_viewer_review_dirty(false);
    s.set_viewer_review_load_error(false);
    s.set_viewer_autosave_paused(false);
    s.set_viewer_feedback("".into());
    s.set_viewer_feedback_error(false);
    s.set_comparison_candidates(ModelRc::default());
    s.set_comparison_loading(false);
    s.set_comparison_from("".into());
    s.set_comparison_to("".into());
    s.set_comparison_fit(true);
    s.set_comparison_zoom_ratio(1.0);
    s.set_comparison_offset_x(0.0);
    s.set_comparison_offset_y(0.0);
    s.set_comparison_zoom(100);
    s.set_comparison_pan_x(0.0);
    s.set_comparison_pan_y(0.0);
}
pub(super) fn loaded(win: &MainWindow, review: io::Result<PhotoReview>) {
    let state = win.global::<AppState>();
    state.set_viewer_original(win.get_viewer_image());
    state.set_viewer_device_scale(win.window().scale_factor());
    match review {
        Ok(review) => {
            state.set_viewer_retained(review.retained);
            state.set_viewer_saved_at(
                crate::workflow_controller::local_saved_time(&review.updated_at).into(),
            );
            state.set_viewer_notes(review.notes.clone().into());
            state.set_viewer_annotations(model(&review));
            if !review.updated_at.is_empty() {
                state.set_viewer_feedback(
                    format!("Observations enregistrées · {}", review.updated_at).into(),
                );
            }
        }
        Err(error) => {
            state.set_viewer_review_load_error(true);
            state.set_viewer_feedback_error(true);
            state.set_viewer_feedback(format!("Notes indisponibles : {error}. Rouvrez la photo pour réessayer ; les observations existantes sont conservées.").into());
        }
    }
    // Restore tools only after both the original and its observations are available.
    state.set_viewer_panel(state.get_preferred_photo_panel());
}
/// Defers navigation/closing until the current observations are durably saved.
pub(super) fn defer_leave(
    win: &MainWindow,
    path: Option<(&slint::SharedString, &slint::SharedString)>,
) -> bool {
    let s = win.global::<AppState>();
    if !s.get_viewer_review_dirty() && !s.get_viewer_review_busy() {
        return false;
    }
    s.set_viewer_pending_action(if path.is_some() { 2 } else { 1 });
    if let Some((path, version)) = path {
        s.set_viewer_pending_path(path.clone());
        s.set_viewer_pending_version(version.clone());
    }
    if !s.get_viewer_review_busy() {
        s.invoke_save_viewer_review();
    }
    true
}
fn finish_save(win: &MainWindow, result: io::Result<()>, revision: i32) {
    let s = win.global::<AppState>();
    s.set_viewer_review_busy(false);
    s.set_viewer_feedback_error(result.is_err());
    if let Err(error) = result {
        s.set_viewer_feedback(format!("Enregistrement impossible : {error}. Les observations restent ouvertes ; utilisez Enregistrer pour réessayer.").into());
        s.set_viewer_pending_action(0);
        s.set_viewer_close_app_pending(false);
        // Stop automatic retries while preserving the editable draft.
        s.set_viewer_autosave_paused(true);
        return;
    }
    if s.get_viewer_review_revision() != revision {
        s.set_viewer_review_dirty(true);
        if s.get_viewer_pending_action() != 0 || s.get_viewer_close_app_pending() {
            s.invoke_save_viewer_review();
        }
        return;
    }
    s.set_viewer_review_dirty(false);
    s.set_viewer_saved_at(chrono::Local::now().format("%H:%M").to_string().into());
    s.set_viewer_feedback("Observations enregistrées.".into());
    let action = s.get_viewer_pending_action();
    s.set_viewer_pending_action(0);
    if s.get_viewer_close_app_pending() {
        s.set_viewer_close_app_pending(false);
        win.window()
            .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    } else if action == 1 {
        s.invoke_close_viewer();
    } else if action == 2 {
        s.invoke_open_capture_file(s.get_viewer_pending_path(), s.get_viewer_pending_version());
    }
}
type UndoHistory = std::rc::Rc<std::cell::RefCell<(String, Vec<PhotoReview>)>>;
fn remember(win: &MainWindow, history: &UndoHistory) {
    let state = win.global::<AppState>();
    let mut history = history.borrow_mut();
    if history.0 != state.get_viewer_path().as_str() || !state.get_viewer_can_undo() {
        history.0 = state.get_viewer_path().to_string();
        history.1.clear();
    }
    if history.1.len() >= 32 {
        history.1.remove(0);
    }
    history.1.push(snapshot(win));
    state.set_viewer_can_undo(true);
}
fn bind_review(window: &MainWindow, runtime: &AppRuntime) {
    let history: UndoHistory = std::rc::Rc::default();
    let weak = window.as_weak();
    window
        .global::<AppState>()
        .on_viewer_review_edited(move || {
            if let Some(win) = weak.upgrade() {
                let s = win.global::<AppState>();
                s.set_viewer_review_revision(s.get_viewer_review_revision().wrapping_add(1));
                s.set_viewer_review_dirty(true);
                s.set_viewer_autosave_paused(false);
                s.set_viewer_feedback("Observations modifiées…".into());
            }
        });
    let weak = window.as_weak();
    let history_add = history.clone();
    window
        .global::<AppState>()
        .on_add_viewer_annotation(move |kind, x, y, end_x, end_y| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if s.get_viewer_review_busy()
                || s.get_viewer_review_load_error()
                || !(1..=4).contains(&kind)
            {
                return;
            }
            if kind == 4 && s.get_viewer_annotation_text().trim().is_empty() {
                s.set_viewer_feedback_error(true);
                s.set_viewer_feedback(
                    "Saisissez le texte de l’annotation avant de la placer.".into(),
                );
                return;
            }
            let mut review = snapshot(&win);
            review.annotations.push(Annotation {
                kind: match kind {
                    1 => AnnotationKind::Circle,
                    2 => AnnotationKind::Arrow,
                    3 => AnnotationKind::Point,
                    _ => AnnotationKind::Text,
                },
                x,
                y,
                end_x,
                end_y,
                text: if kind == 4 {
                    s.get_viewer_annotation_text().to_string()
                } else {
                    String::new()
                },
            });
            if let Err(e) = review.validate() {
                s.set_viewer_feedback_error(true);
                s.set_viewer_feedback(e.to_string().into());
                return;
            }
            remember(&win, &history_add);
            s.set_viewer_annotations(model(&review));
            s.invoke_viewer_review_edited();
        });
    let weak = window.as_weak();
    let history_delete = history.clone();
    window
        .global::<AppState>()
        .on_delete_viewer_annotation(move |index| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if s.get_viewer_review_busy() {
                return;
            }
            let mut review = snapshot(&win);
            if let Ok(index) = usize::try_from(index)
                && index < review.annotations.len()
            {
                remember(&win, &history_delete);
                review.annotations.remove(index);
                s.set_viewer_annotations(model(&review));
                s.invoke_viewer_review_edited();
            }
        });
    bind_undo(window, history);
    bind_save(window, runtime);
}
fn bind_undo(window: &MainWindow, history: UndoHistory) {
    let weak = window.as_weak();
    window
        .global::<AppState>()
        .on_undo_viewer_annotation(move || {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let state = win.global::<AppState>();
            if state.get_viewer_review_busy() {
                return;
            }
            let mut history = history.borrow_mut();
            if history.0 == state.get_viewer_path().as_str()
                && let Some(review) = history.1.pop()
            {
                state.set_viewer_annotations(model(&review));
                state.set_viewer_can_undo(!history.1.is_empty());
                state.invoke_viewer_review_edited();
            }
        });
}
fn bind_save(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    window.global::<AppState>().on_save_viewer_review(move || {
        let Some(win) = weak.upgrade() else {
            return;
        };
        let s = win.global::<AppState>();
        if !s.get_viewer_review_dirty() || s.get_viewer_review_busy() || s.get_viewer_is_video() {
            return;
        }
        let revision = s.get_viewer_review_revision();
        let review = snapshot(&win);
        let result = review
            .validate()
            .and_then(|()| CaptureFileVersion::from_token(s.get_viewer_file_version().as_str()));
        let expected = match result {
            Ok(v) => v,
            Err(e) => {
                finish_save(&win, Err(e), revision);
                return;
            }
        };
        s.set_viewer_review_busy(true);
        s.set_viewer_review_load_error(false);
        s.set_viewer_feedback("Enregistrement des observations…".into());
        let path = PathBuf::from(s.get_viewer_path().as_str());
        let weak = weak.clone();
        if !jobs.submit(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let result =
                iriscope_core::library::save_photo_review(&path, &expected, &review, &|| {
                    std::time::Instant::now() >= deadline
                });
            let _ = weak.upgrade_in_event_loop(move |win| finish_save(&win, result, revision));
        }) {
            finish_save(
                &win,
                Err(io::Error::other("Traitement disque en cours. Réessayez.")),
                revision,
            );
        }
    });
}
#[derive(Clone, Copy, Default)]
pub(super) struct DisplaySettings {
    pub brightness: i32,
    pub contrast: i32,
    pub rotation: i32,
    pub mirror: bool,
}
pub(super) fn display_settings(win: &MainWindow) -> DisplaySettings {
    let s = win.global::<AppState>();
    if s.get_viewer_show_original() {
        return DisplaySettings::default();
    }
    DisplaySettings {
        brightness: s.get_viewer_brightness(),
        contrast: s.get_viewer_contrast(),
        rotation: s.get_viewer_rotation(),
        mirror: s.get_viewer_mirror(),
    }
}
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // Inputs are bounded to ±100; output pixels are rounded and clamped to 0..255.
pub(super) fn transform(mut image: image::RgbImage, settings: DisplaySettings) -> image::RgbImage {
    let brightness = settings.brightness.clamp(-100, 100) as f32 * 1.5;
    let contrast = 2.0_f32.powf(settings.contrast.clamp(-100, 100) as f32 / 100.0);
    for value in image.as_mut() {
        *value = ((f32::from(*value) - 127.5) * contrast + 127.5 + brightness)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    let image = iriscope_imaging::rotate_rgb(image, settings.rotation);
    if settings.mirror {
        image::imageops::flip_horizontal(&image)
    } else {
        image
    }
}
pub(super) fn projected_review(
    review: &PhotoReview,
    settings: DisplaySettings,
    dimensions: (u32, u32),
) -> PhotoReview {
    let geometry =
        iriscope_imaging::RotationGeometry::new(dimensions.0, dimensions.1, settings.rotation);
    let point = |x: f32, y: f32| geometry.project(x, y, settings.mirror);
    let mut result = review.clone();
    for annotation in &mut result.annotations {
        (annotation.x, annotation.y) = point(annotation.x, annotation.y);
        (annotation.end_x, annotation.end_y) = point(annotation.end_x, annotation.end_y);
    }
    result
}
fn bind_display(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    window
        .global::<AppState>()
        .on_refresh_viewer_scale(move || {
            if let Some(win) = weak.upgrade() {
                win.global::<AppState>()
                    .set_viewer_device_scale(win.window().scale_factor());
            }
        });
    let renderer = Arc::new(DisplayRenderer {
        weak: window.as_weak(),
        generation: Arc::clone(&runtime.viewer_generation),
        revision: AtomicU64::new(0),
        state: Mutex::new(DisplayRenderQueue::default()),
    });
    let jobs = Arc::clone(&runtime.background_jobs);
    let weak = window.as_weak();
    window
        .global::<AppState>()
        .on_viewer_display_changed(move || {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let state = win.global::<AppState>();
            let Some(pixels) = state.get_viewer_original().to_rgb8() else {
                return;
            };
            let request = DisplayRequest {
                pixels,
                settings: display_settings(&win),
                revision: renderer.revision.fetch_add(1, Ordering::AcqRel) + 1,
                generation: renderer.generation.load(Ordering::Acquire),
            };
            state.set_viewer_adjusting(true);
            let mut queue = renderer
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queue.latest = Some(request);
            if queue.scheduled {
                return;
            }
            queue.scheduled = true;
            drop(queue);
            let worker = Arc::clone(&renderer);
            if !jobs.submit(move || worker.run()) {
                let mut queue = renderer
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                queue.latest = None;
                queue.scheduled = false;
                state.set_viewer_adjusting(false);
                state.set_viewer_feedback_error(true);
                state.set_viewer_feedback(
                    "Affichage indisponible. Réessayez dans un instant.".into(),
                );
            }
        });
}

struct DisplayRequest {
    pixels: slint::SharedPixelBuffer<slint::Rgb8Pixel>,
    settings: DisplaySettings,
    revision: u64,
    generation: u64,
}
#[derive(Default)]
struct DisplayRenderQueue {
    latest: Option<DisplayRequest>,
    scheduled: bool,
}
struct DisplayRenderer {
    weak: slint::Weak<MainWindow>,
    generation: Arc<AtomicU64>,
    revision: AtomicU64,
    state: Mutex<DisplayRenderQueue>,
}
impl DisplayRenderer {
    fn current(&self, request: &DisplayRequest) -> bool {
        self.revision.load(Ordering::Acquire) == request.revision
            && self.generation.load(Ordering::Acquire) == request.generation
    }
    fn run(self: Arc<Self>) {
        loop {
            let request = {
                let mut queue = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(request) = queue.latest.take() else {
                    queue.scheduled = false;
                    return;
                };
                request
            };
            if !self.current(&request) {
                continue;
            }
            let Some(image) = image::RgbImage::from_raw(
                request.pixels.width(),
                request.pixels.height(),
                request.pixels.as_bytes().to_vec(),
            ) else {
                continue;
            };
            let image = transform(image, request.settings);
            let pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(
                image.as_raw(),
                image.width(),
                image.height(),
            );
            let renderer = Arc::clone(&self);
            let _ = self.weak.upgrade_in_event_loop(move |win| {
                if renderer.generation.load(Ordering::Acquire) == request.generation {
                    win.set_viewer_image(slint::Image::from_rgb8(pixels));
                    let state = win.global::<AppState>();
                    // Show intermediate frames during a drag, and only finish when caught up.
                    state.set_viewer_adjusting(!renderer.current(&request));
                    state.set_viewer_pan_x(0.0);
                    state.set_viewer_pan_y(0.0);
                }
            });
        }
    }
}

type ComparisonRow = (
    String,
    String,
    String,
    Option<slint::SharedPixelBuffer<slint::Rgb8Pixel>>,
);

fn comparison_rows(
    path: &std::path::Path,
    dossier: &str,
    eye: &str,
    cancel: &dyn Fn() -> bool,
    from: &str,
    to: &str,
) -> io::Result<Vec<ComparisonRow>> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("Dossier absent."))?;
    let patient = dossier
        .strip_prefix("D-")
        .and_then(|v| v.parse::<u64>().ok());
    let eye = if eye.to_lowercase().contains("gauche") {
        iriscope_core::session::Eye::Left
    } else if eye.to_lowercase().contains("droit") {
        iriscope_core::session::Eye::Right
    } else {
        iriscope_core::session::Eye::Unspecified
    };
    let query = iriscope_core::library::LibraryQuery::parse(
        dossier,
        from,
        to,
        match eye {
            iriscope_core::session::Eye::Left => 1,
            iriscope_core::session::Eye::Right => 2,
            iriscope_core::session::Eye::Unspecified => 3,
        },
        0,
    )
    .map_err(io::Error::other)?;
    let mut candidates =
        iriscope_core::library::try_scan_library_directory_metadata_cancellable(directory, cancel)?;
    candidates.retain(|e| {
        e.kind == iriscope_core::library::CaptureKind::Photo
            && query.matches(e)
            && e.file_path != path
    });
    candidates.sort_by_key(|e| std::cmp::Reverse(e.timestamp()));
    candidates.truncate(200);
    let entries = iriscope_core::library::resolve_library_candidates_cancellable(
        directory,
        &candidates,
        cancel,
    )?;
    Ok::<_, io::Error>(
        entries
            .into_iter()
            .filter(|e| e.patient_id == patient && e.eye == eye)
            .filter_map(|e| {
                let version = e.file_version.as_ref()?;
                let retained =
                    iriscope_core::library::load_photo_review(&e.file_path, version, cancel)
                        .is_ok_and(|r| r.retained);
                Some((
                    e.file_path.to_string_lossy().into_owned(),
                    e.file_version.as_ref()?.token(),
                    format!(
                        "{} {} · {}{}",
                        crate::workflow_controller::french_date(&e.date_str),
                        e.time_str,
                        eye.filename_label(),
                        if retained { " · Retenue" } else { "" }
                    ),
                    crate::workflow_controller::thumbnail(&e.file_path, &e.file_version?, cancel),
                ))
            })
            .collect::<Vec<_>>(),
    )
}
fn bind_comparison(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    let generation = Arc::clone(&runtime.viewer_generation);
    window
        .global::<AppState>()
        .on_find_comparison_photos(move || {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if s.get_comparison_loading() || s.get_viewer_dossier().is_empty() {
                return;
            }
            let path = PathBuf::from(s.get_viewer_path().as_str());
            let dossier = s.get_viewer_dossier().to_string();
            let eye = s.get_viewer_eye_label().to_string();
            let from = s.get_comparison_from().to_string();
            let to = s.get_comparison_to().to_string();
            let id = generation.load(Ordering::Acquire);
            let generation = Arc::clone(&generation);
            let weak = weak.clone();
            s.set_comparison_loading(true);
            if !jobs.submit(move || {
                let cancel = || generation.load(Ordering::Acquire) != id;
                let result = comparison_rows(&path, &dossier, &eye, &cancel, &from, &to);
                let _ = weak.upgrade_in_event_loop(move |win| {
                    if generation.load(Ordering::Acquire) != id {
                        return;
                    }
                    let s = win.global::<AppState>();
                    s.set_comparison_loading(false);
                    match result {
                        Ok(rows) => {
                            if rows.is_empty() {
                                s.set_viewer_feedback(
                                    "Aucune autre photo du même œil dans ce dossier.".into(),
                                );
                            }
                            s.set_comparison_candidates(ModelRc::new(VecModel::from(
                                rows.into_iter()
                                    .map(|(path, version, caption, thumbnail)| {
                                        ComparisonCandidateData {
                                            thumbnail: thumbnail.map_or_else(
                                                slint::Image::default,
                                                slint::Image::from_rgb8,
                                            ),
                                            path: path.into(),
                                            version: version.into(),
                                            caption: caption.into(),
                                        }
                                    })
                                    .collect::<Vec<_>>(),
                            )));
                        }
                        Err(e) => {
                            s.set_viewer_feedback_error(true);
                            s.set_viewer_feedback(e.to_string().into());
                        }
                    }
                });
            }) {
                s.set_comparison_loading(false);
            }
        });
    bind_select_comparison(window, runtime);
}
fn bind_select_comparison(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    let generation = Arc::clone(&runtime.viewer_generation);
    window.global::<AppState>().on_select_comparison_photo(move |path,version,caption| {
        let Some(win)=weak.upgrade() else{return;};let s=win.global::<AppState>();
        if s.get_comparison_loading(){return;}let Ok(expected)=CaptureFileVersion::from_token(version.as_str()) else{return;};
        let id=generation.load(Ordering::Acquire);let generation=Arc::clone(&generation);let weak=weak.clone();s.set_comparison_loading(true);
        if !jobs.submit(move || {
            let result=crate::playback::read_verified_photo_cancellable(std::path::Path::new(path.as_str()),&expected,&||generation.load(Ordering::Acquire)!=id);
            let pixels=result.ok().flatten().map(|(w,h,rgb)|slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(&rgb,w,h));
            let _=weak.upgrade_in_event_loop(move |win| {
                if generation.load(Ordering::Acquire)!=id{return;}let s=win.global::<AppState>();s.set_comparison_loading(false);
                if let Some(pixels)=pixels {s.set_comparison_image(slint::Image::from_rgb8(pixels));s.set_comparison_path(path);s.set_comparison_caption(caption);s.set_comparison_available(true);s.set_comparison_enabled(true);s.set_comparison_fit(true);s.set_comparison_zoom(100);s.set_comparison_pan_x(0.0);s.set_comparison_pan_y(0.0);s.set_viewer_fit(true);s.set_viewer_panel(0);}
                else{s.set_viewer_feedback_error(true);s.set_viewer_feedback("Photo de comparaison illisible ou modifiée. Actualisez la bibliothèque.".into());}
            });
        }){s.set_comparison_loading(false);}
    });
}
pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    bind_review(window, runtime);
    bind_display(window, runtime);
    bind_comparison(window, runtime);
    crate::photo_export::install(window, runtime);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_transform_rotates_and_mirrors_without_mutating_source() {
        let original = image::RgbImage::from_raw(2, 1, vec![10, 20, 30, 200, 210, 220]).unwrap();
        let rotated = transform(
            original.clone(),
            DisplaySettings {
                rotation: 90,
                ..Default::default()
            },
        );
        assert_eq!(rotated.dimensions(), (1, 2));
        assert_eq!(rotated.get_pixel(0, 1).0, [200, 210, 220]);
        let mirrored = transform(
            original.clone(),
            DisplaySettings {
                mirror: true,
                ..Default::default()
            },
        );
        assert_eq!(mirrored.get_pixel(0, 0).0, [200, 210, 220]);
        let modified = transform(
            original.clone(),
            DisplaySettings {
                brightness: 50,
                contrast: 100,
                ..Default::default()
            },
        );
        assert_ne!(modified, original);
        assert_eq!(original.as_raw(), &[10, 20, 30, 200, 210, 220]);
        assert_eq!(
            transform(original.clone(), DisplaySettings::default()),
            original
        );
    }
}
