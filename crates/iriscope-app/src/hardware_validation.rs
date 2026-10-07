//! Bounded validation of the actual camera through production UI and disk workers.

use crate::{
    app_helpers::dispatch_hardware_button,
    gui::install_controllers,
    runtime::AppRuntime,
    ui::{AppState, MainWindow},
};
use iriscope_core::{
    library::{
        CaptureKind, LibraryEntry, backup_library, create_patient, restore_library,
        try_scan_library_directory,
    },
    session::Eye,
    settings::{AppSettings, PhysicalButtonBehavior},
    video::AviMjpegReader,
};
use slint::{ComponentHandle, Model};
use std::fmt::Write as _;
use std::{
    cell::RefCell,
    fs, io,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

struct Validation {
    phase: u8,
    started: Instant,
    recording_started: Option<Instant>,
    first_patient: u64,
    second_patient: u64,
    physical: bool,
    result: Option<Result<Vec<LibraryEntry>, String>>,
}

fn create_validation_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn select_patient(window: &MainWindow, id: Option<u64>, eye: Eye) {
    window.set_patient_first_name(if id.is_some() { "Émilie" } else { "" }.into());
    window.set_patient_last_name(if id.is_some() { "Test IrisScope" } else { "" }.into());
    window.set_patient_id(
        id.map_or_else(String::new, |value| value.to_string())
            .into(),
    );
    window.set_selected_eye(if eye == Eye::Left { 1 } else { 2 });
}

impl Validation {
    // Keep the sequential capture stages together so their transitions stay visible.
    #[allow(clippy::too_many_lines)]
    fn advance(
        &mut self,
        window: &MainWindow,
        directory: &Path,
    ) -> Result<Option<Vec<LibraryEntry>>, String> {
        if self.started.elapsed() > Duration::from_secs(if self.physical { 180 } else { 45 }) {
            return Err(format!(
                "Validation timed out at phase {}: {}",
                self.phase,
                window.get_status_text()
            ));
        }
        if self.phase == 0 {
            if !window.get_is_streaming() {
                return Ok(None);
            }
            println!(
                "HARDWARE_BUTTON_STATUS {}",
                window.global::<AppState>().get_hardware_button_status()
            );
            println!(
                "CAMERA_CONTROLS_UI {}",
                window
                    .get_camera_controls()
                    .iter()
                    .map(|control| control.name.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            select_patient(window, Some(self.first_patient), Eye::Left);
            window.set_is_video_mode(false);
            println!(
                "BUTTON_PHOTO_READY dossier={} eye=Gauche",
                self.first_patient
            );
            if !self.physical {
                dispatch_hardware_button(window, PhysicalButtonBehavior::FollowMode);
            }
            self.phase = 1;
            return Ok(None);
        }
        let entries = match try_scan_library_directory(directory) {
            Ok(entries) => entries,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) =>
            {
                // Capture publication removes its temporary files and journals.
                // Retry an in-flight snapshot on the next bounded timer tick.
                println!("VALIDATION_SCAN_RETRY {error}");
                return Ok(None);
            }
            Err(error) => return Err(error.to_string()),
        };
        let photos = if self.physical { 2 } else { 3 };
        match self.phase {
            1 if entries.len() == 1 => {
                select_patient(window, Some(self.second_patient), Eye::Right);
                println!(
                    "BUTTON_SECOND_PHOTO_READY dossier={} eye=Droit",
                    self.second_patient
                );
                if !self.physical {
                    dispatch_hardware_button(window, PhysicalButtonBehavior::AlwaysPhoto);
                    dispatch_hardware_button(window, PhysicalButtonBehavior::AlwaysPhoto);
                }
                self.phase = 2;
            }
            2 if entries.len() == photos => {
                select_patient(window, Some(self.first_patient), Eye::Right);
                window.set_is_video_mode(true);
                println!(
                    "BUTTON_VIDEO_START_READY dossier={} eye=Droit",
                    self.first_patient
                );
                if !self.physical {
                    dispatch_hardware_button(window, PhysicalButtonBehavior::FollowMode);
                }
                self.phase = 3;
            }
            3 if window.get_is_recording() => {
                let started = *self.recording_started.get_or_insert_with(|| {
                    println!("BUTTON_VIDEO_STOP_READY recording=true");
                    Instant::now()
                });
                if !self.physical && started.elapsed() >= Duration::from_secs(4) {
                    // Stop remains available even if the selected capture mode changed.
                    window.set_is_video_mode(false);
                    dispatch_hardware_button(window, PhysicalButtonBehavior::FollowMode);
                    self.phase = 4;
                }
            }
            3 if self.physical
                && self.recording_started.is_some()
                && !window.get_recording_finalizing() =>
            {
                self.phase = 4;
            }
            4 if entries.len() == photos + 1 && !window.get_recording_finalizing() => {
                if self.physical {
                    return Ok(Some(entries));
                }
                select_patient(window, None, Eye::Right);
                dispatch_hardware_button(window, PhysicalButtonBehavior::AlwaysPhoto);
                self.phase = 5;
            }
            5 if entries.len() == photos + 2 => return Ok(Some(entries)),
            _ => {}
        }
        Ok(None)
    }
}

/// Validates captures in a new, isolated directory, leaving personal settings intact.
/// With `physical`, the operator performs two photo presses and video start/stop.
/// # Errors
/// Returns camera, interface, storage, decoding, association or timeout failures.
#[allow(clippy::too_many_lines)]
pub fn run_hardware_validation(
    output: &Path,
    physical: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if output.exists() && fs::read_dir(output)?.next().is_some() {
        return Err("Validation requires an empty directory; existing files are preserved".into());
    }
    create_validation_directory(output)?;
    let directory = output.join("captures");
    create_validation_directory(&directory)?;
    let first = create_patient(&directory, "Émilie", "Test IrisScope")?;
    let second = create_patient(&directory, "Émilie", "Test IrisScope")?;
    let window = MainWindow::new()?;
    let mut runtime = AppRuntime::new(
        AppSettings {
            capture_directory: directory.clone(),
            ..AppSettings::default()
        },
        output.join("settings.json"),
    );
    install_controllers(&window, &runtime);
    runtime.start_workers(&window);
    let state = Rc::new(RefCell::new(Validation {
        phase: 0,
        started: Instant::now(),
        recording_started: None,
        first_patient: first.id,
        second_patient: second.id,
        physical,
        result: None,
    }));
    let timer = slint::Timer::default();
    let current = Rc::clone(&state);
    let weak = window.as_weak();
    let captures = directory.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(window) = weak.upgrade() else { return };
            let result = current.borrow_mut().advance(&window, &captures);
            match result {
                Ok(None) => {}
                Ok(Some(entries)) => {
                    current.borrow_mut().result = Some(Ok(entries));
                    let _ = slint::quit_event_loop();
                }
                Err(error) => {
                    current.borrow_mut().result = Some(Err(error));
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    println!(
        "VALIDATION_START physical={physical} output={}",
        output.display()
    );
    let event_loop = window.run();
    timer.stop();
    let button_status = window.global::<AppState>().get_hardware_button_status();
    let control_names = window
        .get_camera_controls()
        .iter()
        .map(|control| control.name.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    runtime.shutdown();
    event_loop?;
    let entries = state
        .borrow_mut()
        .result
        .take()
        .ok_or("Validation closed before completion")?
        .map_err(io::Error::other)?;
    let mut report = format!(
        "IrisScope {} — {} {} hardware validation\nPhysical button: {physical}\n{button_status}\nCamera controls in UI: {control_names}\nDossiers: {}, {} (same names, distinct identities)\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        first.dossier_number,
        second.dossier_number
    );
    let mut left = 0;
    let mut right = 0;
    let mut videos = 0;
    let mut anonymous = 0;
    for entry in &entries {
        match (entry.kind, entry.patient_id, entry.eye) {
            (CaptureKind::Photo, Some(id), Eye::Left) if id == first.id => left += 1,
            (CaptureKind::Photo, Some(id), Eye::Right) if id == second.id => right += 1,
            (CaptureKind::Video, Some(id), Eye::Right) if id == first.id => videos += 1,
            (CaptureKind::Photo, None, Eye::Right) if !physical => anonymous += 1,
            _ => return Err("A real capture was assigned to the wrong dossier or eye".into()),
        }
        let filename = entry
            .file_path
            .file_name()
            .ok_or("Missing capture name")?
            .to_string_lossy();
        if !filename.contains(entry.eye.filename_label())
            || !filename.contains(&entry.date_str)
            || !filename.contains(&entry.time_str.replace(':', "-"))
        {
            return Err("A capture filename is missing its eye, date or time".into());
        }
        if entry.kind == CaptureKind::Photo {
            let bytes = fs::read(&entry.file_path)?;
            let (width, height, _) = iriscope_imaging::decode_image_to_rgb8(&bytes)?;
            writeln!(
                report,
                "PHOTO {filename}: {width}x{height}, dossier={:?}, eye={:?}",
                entry.patient_id, entry.eye
            )?;
        } else {
            let mut video = AviMjpegReader::open(&entry.file_path)?;
            if video.frame_count() < 2 {
                return Err("The actual camera video contains fewer than two frames".into());
            }
            for index in 0..video.frame_count() {
                iriscope_imaging::decode_mjpeg_to_rgb8(&video.read_frame(index)?)?;
            }
            writeln!(
                report,
                "VIDEO {filename}: {} readable frames, {:.2} fps, dossier={:?}, eye={:?}",
                video.frame_count(),
                video.frame_rate().frames_per_second(),
                entry.patient_id,
                entry.eye
            )?;
        }
    }
    let expected = if physical { (1, 1, 1, 0) } else { (1, 2, 1, 1) };
    if (left, right, videos, anonymous) != expected {
        return Err("Unexpected capture counts or dossier associations".into());
    }
    if fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .any(|item| item.file_name().to_string_lossy().ends_with(".part"))
    {
        return Err("A video was not finalized".into());
    }
    let exports = output.join("exports");
    create_validation_directory(&exports)?;
    let video = entries
        .iter()
        .find(|entry| entry.kind == CaptureKind::Video)
        .ok_or("Missing actual camera video")?;
    let original = fs::read(&video.file_path)?;
    let destination = exports
        .join(
            video
                .file_path
                .file_name()
                .ok_or("Missing video filename")?,
        )
        .with_extension("mp4");
    let export = crate::video_export::export_mp4(
        &video.file_path,
        video.file_version.as_ref().ok_or("Missing video version")?,
        &destination,
        &|| false,
        &mut |_| {},
    );
    match export {
        Ok(path) => writeln!(report, "MP4 export: {}", path.display())?,
        Err(error) if crate::video_export::encoder_is_missing(&error) => {
            writeln!(
                report,
                "NOT TESTED: MP4 export requires optional FFmpeg. Camera captures and backup validation continue."
            )?;
        }
        Err(error) => return Err(error.into()),
    }
    if fs::read(&video.file_path)? != original {
        return Err("MP4 export changed the original AVI".into());
    }
    writeln!(report, "Original AVI preserved")?;
    let backups = output.join("backups");
    let restored = output.join("restored");
    create_validation_directory(&backups)?;
    create_validation_directory(&restored)?;
    let backup = backup_library(&directory, &backups, &|| false)?;
    let restored = restore_library(&backup.directory, &restored, &|| false)?;
    let recovered = try_scan_library_directory(&restored.directory)?;
    if recovered.len() != entries.len()
        || entries.iter().any(|entry| {
            !recovered.iter().any(|copy| {
                copy.file_path.file_name() == entry.file_path.file_name()
                    && copy.patient_id == entry.patient_id
                    && copy.eye == entry.eye
            })
        })
    {
        return Err("Backup restoration lost a capture or its dossier association".into());
    }
    for entry in &entries {
        if fs::read(&entry.file_path)?
            != fs::read(
                restored
                    .directory
                    .join(entry.file_path.file_name().ok_or("Missing filename")?),
            )?
        {
            return Err("A restored capture differs from the actual camera original".into());
        }
    }
    writeln!(
        report,
        "Backup and restoration: {} captures, identical media bytes and dossier associations",
        recovered.len()
    )?;
    report.push_str(
        "PASS: names, both eyes, homonyms, decoding, finalized video and library associations\n",
    );
    fs::write(output.join("validation-report.txt"), &report)?;
    println!("{report}");
    Ok(())
}
