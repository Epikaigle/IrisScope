use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use iriscope_core::{
    camera::{CameraDescriptor, CameraErrorKind, CameraEvent, CapturedFrame, StreamConfiguration},
    capabilities::{
        CameraControlDescriptor, CameraControlId, CameraControlKind, CameraControlValue,
        PixelFormat,
    },
    capture::LatestFrame,
    library::{
        CaptureKind, LibraryFilter, present_library_items, record_capture_metadata,
        scan_library_directory,
    },
    session::{CaptureSession, Eye},
    settings::{AppSettings, PhysicalButtonBehavior},
    storage::{
        CaptureNamingPolicy, CaptureTimestamp, DEFAULT_FILENAME_TEMPLATE,
        filename_template_preserves_identity, save_new_capture,
    },
    video::{AviMjpegReader, AviMjpegWriter},
};
use iriscope_imaging::{
    convert_bgra8_to_rgb8, convert_yuyv_to_rgb8, decode_image_to_rgb8, decode_mjpeg_to_rgb8,
    encode_rgb8_jpeg, encode_rgb8_png, ensure_jpeg_has_dht, resize_rgb8_to_fit,
};
use slint::{ComponentHandle, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel};

#[cfg(target_os = "linux")]
use iriscope_camera_linux as platform_camera;
#[cfg(target_os = "macos")]
use iriscope_camera_macos as platform_camera;
#[cfg(target_os = "windows")]
use iriscope_camera_windows as platform_camera;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("IrisScope currently supports Linux, Windows, and macOS");

slint::include_modules!();

const DE400_VENDOR_ID: u16 = 0x21cd;
const DE400_PRODUCT_ID: u16 = 0x603b;

fn is_de400(descriptor: &CameraDescriptor) -> bool {
    descriptor
        .usb
        .as_ref()
        .is_some_and(|usb| usb.vendor_id == DE400_VENDOR_ID && usb.product_id == DE400_PRODUCT_ID)
}

fn camera_error_status(kind: CameraErrorKind) -> &'static str {
    match kind {
        CameraErrorKind::PermissionDenied => "Accès caméra refusé",
        CameraErrorKind::DeviceBusy => "DE400 déjà utilisé",
        CameraErrorKind::BackendUnavailable | CameraErrorKind::Backend => "Caméra indisponible",
        CameraErrorKind::DeviceNotFound | CameraErrorKind::Disconnected => "DE400 non détecté",
        CameraErrorKind::TimedOut => "La caméra ne répond pas",
        CameraErrorKind::Unsupported | CameraErrorKind::InvalidConfiguration => {
            "Flux DE400 indisponible"
        }
        _ => "Erreur caméra",
    }
}

enum WorkerCommand {
    SetControl(CameraControlId, CameraControlValue),
    ResetControls,
    Stop,
}

fn settings_file_path() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    if let Ok(base) = std::env::var("APPDATA") {
        return std::path::PathBuf::from(base)
            .join("IrisScope")
            .join("settings.json");
    }

    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("IrisScope")
            .join("settings.json");
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(base) = std::env::var("XDG_CONFIG_HOME") {
            return std::path::PathBuf::from(base)
                .join("IrisScope")
                .join("settings.json");
        }
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home)
                .join(".config")
                .join("IrisScope")
                .join("settings.json");
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("iriscope-settings.json")
}

fn settings_snapshot(settings: &Arc<Mutex<AppSettings>>) -> AppSettings {
    settings
        .lock()
        .map_or_else(|_| AppSettings::default(), |guard| guard.clone())
}

fn persist_settings(settings: &Arc<Mutex<AppSettings>>, path: &std::path::Path) {
    if let Ok(guard) = settings.lock() {
        let _ = guard.save_to_file(path);
    }
}

fn physical_button_mode_index(behavior: PhysicalButtonBehavior) -> i32 {
    match behavior {
        PhysicalButtonBehavior::FollowMode => 0,
        PhysicalButtonBehavior::AlwaysPhoto => 1,
        PhysicalButtonBehavior::AlwaysVideo => 2,
    }
}

fn capture_session_from_window(win: &MainWindow) -> Result<CaptureSession, &'static str> {
    let first_name = win.get_patient_first_name().to_string();
    let last_name = win.get_patient_last_name().to_string();

    if first_name.trim().is_empty() || last_name.trim().is_empty() {
        return Err("Renseignez le prénom et le nom avant la capture.");
    }

    let eye = match win.get_selected_eye() {
        1 => Eye::Left,
        2 => Eye::Right,
        _ => return Err("Sélectionnez l'œil gauche ou droit avant la capture."),
    };

    Ok(CaptureSession::new(first_name, last_name, eye))
}

fn dispatch_hardware_button(win: &MainWindow, behavior: PhysicalButtonBehavior) {
    match behavior {
        PhysicalButtonBehavior::FollowMode => {
            if win.get_is_video_mode() {
                win.invoke_toggle_recording();
            } else {
                win.invoke_trigger_capture();
            }
        }
        PhysicalButtonBehavior::AlwaysPhoto => win.invoke_trigger_capture(),
        PhysicalButtonBehavior::AlwaysVideo => win.invoke_toggle_recording(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--diagnose") {
        run_diagnose();
        return Ok(());
    }

    run_gui()
}

#[allow(clippy::too_many_lines)]
fn run_diagnose() {
    let mut backend = platform_camera::create_backend();
    println!("IrisScope Camera Diagnostic ({})", backend.kind());

    let devices = match backend.enumerate_devices() {
        Ok(devs) => devs,
        Err(err) => {
            eprintln!("Failed to enumerate cameras: {err}");
            std::process::exit(1);
        }
    };

    if devices.is_empty() {
        println!("No video capture device detected.");
        return;
    }

    for (index, device) in devices.iter().enumerate() {
        println!("\nDevice #{index}: {}", device.display_name);
        println!("  ID: {}", device.id);
        if let Some(usb) = &device.usb {
            println!(
                "  USB: VID {:04x}, PID {:04x}",
                usb.vendor_id, usb.product_id
            );
            if let Some(serial) = &usb.serial_number {
                println!("  Serial: {serial}");
            }
            if let Some(rev) = &usb.hardware_revision {
                println!("  Revision: {rev}");
            }
        }
    }

    let Some(target) = devices.iter().find(|device| is_de400(device)) else {
        println!("\nFirefly DE400 not detected.");
        return;
    };

    println!("\nOpening device: {} [{}]", target.display_name, target.id);
    let mut dev = match backend.open(&target.id) {
        Ok(dev) => dev,
        Err(err) => {
            eprintln!("Failed to open camera: {err}");
            std::process::exit(1);
        }
    };

    let caps = dev.capabilities();
    println!("\nModes available:");
    for mode in &caps.modes {
        let fps_list: Vec<String> = mode
            .frame_rates
            .iter()
            .map(|f| format!("{:.2} fps", f.frames_per_second()))
            .collect();
        println!(
            "  Format: {}, Resolution: {}, Rates: [{}]",
            mode.pixel_format,
            mode.resolution,
            fps_list.join(", ")
        );
    }

    println!("\nControls available:");
    for ctrl in &caps.controls {
        println!(
            "  Control: {} ({})",
            ctrl.name,
            if ctrl.read_only {
                "Read-only"
            } else {
                "Read-Write"
            }
        );
    }

    let Some((pref_mode, pref_fps)) = caps.preferred_mode() else {
        println!("No usable mode found.");
        return;
    };

    println!(
        "\nSelected preferred mode: {} at {} with {:.2} fps",
        pref_mode.pixel_format,
        pref_mode.resolution,
        pref_fps.frames_per_second()
    );
    let config = StreamConfiguration {
        pixel_format: pref_mode.pixel_format.clone(),
        resolution: pref_mode.resolution,
        frame_rate: pref_fps,
    };

    println!("Starting stream...");
    if let Err(err) = dev.start_stream(&config) {
        eprintln!("Failed to start stream: {err}");
        std::process::exit(1);
    }

    println!("Stream started successfully. Capturing 3 test frames...");
    let start_time = Instant::now();
    let mut captured = 0;
    while captured < 3 && start_time.elapsed() < Duration::from_secs(5) {
        match dev.next_event(Duration::from_millis(1500)) {
            Ok(CameraEvent::Frame(frame)) => {
                captured += 1;
                println!(
                    "  Frame #{}: {} bytes (seq {}, timestamp {:?})",
                    captured,
                    frame.data.len(),
                    frame.sequence_number,
                    frame.timestamp
                );
                match decode_mjpeg_to_rgb8(&frame.data) {
                    Ok((w, h, rgb)) => {
                        println!(
                            "  ✓ Successfully decoded to RGB8: {w}×{h} ({} bytes)",
                            rgb.len()
                        );
                    }
                    Err(err) => {
                        eprintln!("  ✗ Decode error: {err}");
                    }
                }
            }
            Ok(other) => {
                println!("  Event: {other:?}");
            }
            Err(err) => {
                eprintln!("  Capture error: {err}");
                break;
            }
        }
    }

    let elapsed = start_time.elapsed();
    let _ = dev.stop_stream();
    println!(
        "Stopped stream. Captured {captured} frames in {:.2}s",
        elapsed.as_secs_f64()
    );
}

fn load_full_image(path: &std::path::Path) -> Option<slint::Image> {
    let bytes = std::fs::read(path).ok()?;
    let (width, height, rgb) = decode_image_to_rgb8(&bytes).ok()?;
    let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
    Some(slint::Image::from_rgb8(pixels))
}

fn apply_reference_images(win: &MainWindow, settings: &AppSettings) {
    if let Some(path) = settings.iridology_map_path.as_deref()
        && let Some(image) = load_full_image(path)
    {
        win.set_iridology_map_image(image);
        win.set_has_iridology_map(true);
    } else {
        win.set_has_iridology_map(false);
    }

    if let Some(path) = settings.iridology_symbols_path.as_deref()
        && let Some(image) = load_full_image(path)
    {
        win.set_iridology_symbols_image(image);
        win.set_has_iridology_symbols(true);
    } else {
        win.set_has_iridology_symbols(false);
    }
}

fn load_thumbnail(path: &std::path::Path) -> Option<slint::Image> {
    let bytes = std::fs::read(path).ok()?;
    let (width, height, rgb) = decode_image_to_rgb8(&bytes).ok()?;
    let (thumb_width, thumb_height, thumbnail) =
        resize_rgb8_to_fit(&rgb, width, height, 240).ok()?;
    let pixels =
        SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&thumbnail, thumb_width, thumb_height);
    Some(slint::Image::from_rgb8(pixels))
}

fn load_library_items(
    dir: &std::path::Path,
    active_session: &CaptureSession,
) -> Vec<LibraryItemData> {
    let raw_entries = scan_library_directory(dir);
    let presented = present_library_items(&raw_entries, active_session, LibraryFilter::All);
    presented
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            let thumbnail = if matches!(item.kind, CaptureKind::Photo) {
                load_thumbnail(&item.file_path)
            } else {
                None
            };

            LibraryItemData {
                date_time: item.date_time.into(),
                eye_label: item.eye_label.into(),
                file_path: item.file_path.to_string_lossy().to_string().into(),
                has_thumbnail: thumbnail.is_some(),
                id: idx.to_string().into(),
                is_current_session: item.is_current_session,
                is_video: matches!(item.kind, CaptureKind::Video),
                thumbnail: thumbnail.unwrap_or_default(),
                title: item.display_title.into(),
            }
        })
        .collect()
}

#[derive(Clone)]
struct CameraControlRuntimeState {
    key: String,
    descriptor: CameraControlDescriptor,
    value: CameraControlValue,
}

fn camera_control_key(id: &CameraControlId) -> String {
    match id {
        CameraControlId::Standard(control) => format!("standard:{control:?}"),
        CameraControlId::PlatformSpecific(name) => format!("platform:{name}"),
        CameraControlId::UvcExtension {
            unit,
            selector,
            guid,
        } => format!(
            "uvc:{unit}:{selector}:{}",
            guid.as_deref().unwrap_or_default()
        ),
        _ => format!("unknown:{id:?}"),
    }
}

fn default_camera_control_value(kind: &CameraControlKind) -> Option<CameraControlValue> {
    match kind {
        CameraControlKind::Integer { default, .. } => Some(CameraControlValue::Integer(*default)),
        CameraControlKind::Boolean { default } => Some(CameraControlValue::Boolean(*default)),
        CameraControlKind::Menu { default, .. } => Some(CameraControlValue::Menu(*default)),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn camera_control_ui_data(state: &CameraControlRuntimeState) -> CameraControlUiData {
    let mut data = CameraControlUiData {
        key: state.key.clone().into(),
        name: state.descriptor.name.clone().into(),
        kind: 0,
        minimum: 0.0,
        maximum: 1.0,
        value: 0.0,
        boolean_value: false,
        menu_label: "".into(),
        read_only: state.descriptor.read_only,
    };

    match (&state.descriptor.kind, &state.value) {
        (
            CameraControlKind::Integer {
                minimum,
                maximum,
                default,
                ..
            },
            CameraControlValue::Integer(value),
        ) => {
            data.kind = 0;
            data.minimum = *minimum as f32;
            data.maximum = *maximum as f32;
            data.value = *value as f32;
            if !data.value.is_finite() {
                data.value = *default as f32;
            }
        }
        (CameraControlKind::Boolean { default }, CameraControlValue::Boolean(value)) => {
            data.kind = 1;
            data.boolean_value = *value;
            if state.descriptor.read_only {
                data.boolean_value = *default;
            }
        }
        (CameraControlKind::Menu { items, default }, CameraControlValue::Menu(value)) => {
            data.kind = 2;
            let active = items
                .iter()
                .find(|item| item.value == *value)
                .or_else(|| items.iter().find(|item| item.value == *default));
            data.menu_label = active
                .map_or_else(|| value.to_string(), |item| item.label.clone())
                .into();
            data.value = *value as f32;
        }
        (kind, _) => {
            if let Some(fallback) = default_camera_control_value(kind) {
                return camera_control_ui_data(&CameraControlRuntimeState {
                    key: state.key.clone(),
                    descriptor: state.descriptor.clone(),
                    value: fallback,
                });
            }
            data.read_only = true;
        }
    }

    data
}

fn set_camera_control_model(win: &MainWindow, states: &[CameraControlRuntimeState]) {
    let rows = states
        .iter()
        .map(camera_control_ui_data)
        .collect::<Vec<_>>();
    win.set_camera_controls(ModelRc::new(VecModel::from(rows)));
}

#[allow(clippy::cast_possible_truncation)]
fn snap_integer_control_value(
    descriptor: &CameraControlDescriptor,
    requested: f32,
) -> Option<CameraControlValue> {
    let CameraControlKind::Integer {
        minimum,
        maximum,
        step,
        ..
    } = descriptor.kind
    else {
        return None;
    };

    let step = step.max(1);
    let requested = requested.round() as i64;
    let clamped = requested.clamp(minimum, maximum);
    let snapped = minimum + ((clamped - minimum) / step) * step;
    Some(CameraControlValue::Integer(snapped))
}

fn finalize_recording(
    is_recording: &AtomicBool,
    video_writer: &Mutex<Option<AviMjpegWriter>>,
    recording_start: &Mutex<Option<Instant>>,
) -> bool {
    let was_recording = is_recording.swap(false, Ordering::Relaxed);

    if let Ok(mut start) = recording_start.lock() {
        *start = None;
    }

    if let Ok(mut writer_guard) = video_writer.lock()
        && let Some(mut writer) = writer_guard.take()
    {
        let _ = writer.finish();
    }

    was_recording
}

#[allow(clippy::too_many_lines)]
fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let main_window = MainWindow::new()?;
    let settings_path = settings_file_path();
    let mut loaded_settings = AppSettings::load_from_file(&settings_path);
    if !filename_template_preserves_identity(&loaded_settings.filename_template) {
        DEFAULT_FILENAME_TEMPLATE.clone_into(&mut loaded_settings.filename_template);
    }
    let _ = loaded_settings.save_to_file(&settings_path);
    main_window.set_settings_capture_directory(
        loaded_settings
            .capture_directory
            .to_string_lossy()
            .to_string()
            .into(),
    );
    main_window.set_settings_filename_template(loaded_settings.filename_template.clone().into());
    main_window.set_settings_button_mode(physical_button_mode_index(
        loaded_settings.physical_button_behavior,
    ));
    main_window.set_settings_iridology_map_path(
        loaded_settings
            .iridology_map_path
            .as_deref()
            .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
            .into(),
    );
    main_window.set_settings_iridology_symbols_path(
        loaded_settings
            .iridology_symbols_path
            .as_deref()
            .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
            .into(),
    );
    apply_reference_images(&main_window, &loaded_settings);
    let settings = Arc::new(Mutex::new(loaded_settings));
    let latest_frame = Arc::new(LatestFrame::new());
    let active_stream_configuration: Arc<Mutex<Option<StreamConfiguration>>> =
        Arc::new(Mutex::new(None));
    let is_recording = Arc::new(AtomicBool::new(false));
    let video_writer: Arc<Mutex<Option<AviMjpegWriter>>> = Arc::new(Mutex::new(None));
    let active_session = Arc::new(Mutex::new(CaptureSession::default()));
    let recording_start: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let last_toast_time: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let frozen_frame: Arc<Mutex<Option<CapturedFrame>>> = Arc::new(Mutex::new(None));
    let viewer_generation = Arc::new(AtomicU64::new(0));
    let viewer_video_playing = Arc::new(AtomicBool::new(false));
    let camera_controls: Arc<Mutex<Vec<CameraControlRuntimeState>>> =
        Arc::new(Mutex::new(Vec::new()));

    let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>();

    // Spawn camera capture worker thread
    let main_weak = main_window.as_weak();
    let latest_frame_clone = Arc::clone(&latest_frame);
    let is_rec_clone = Arc::clone(&is_recording);
    let video_writer_clone = Arc::clone(&video_writer);
    let rec_start_clone = Arc::clone(&recording_start);
    let toast_clone = Arc::clone(&last_toast_time);
    let active_stream_configuration_worker = Arc::clone(&active_stream_configuration);
    let camera_controls_worker = Arc::clone(&camera_controls);
    let settings_worker = Arc::clone(&settings);

    thread::spawn(move || {
        let mut backend = platform_camera::create_backend();
        loop {
            let devices = match backend.enumerate_devices() {
                Ok(devices) => devices,
                Err(error) => {
                    let status = camera_error_status(error.kind());
                    let _ = main_weak.upgrade_in_event_loop(move |win| {
                        win.set_camera_connected(false);
                        win.set_is_streaming(false);
                        win.set_status_text(status.into());
                    });
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            if devices.is_empty() {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let Some(target) = devices.iter().find(|device| is_de400(device)).cloned() else {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            };

            let usb_info = target.usb.as_ref().map_or_else(
                || "N/A".to_string(),
                |u| format!("{:04x}:{:04x}", u.vendor_id, u.product_id),
            );

            let mut device = match backend.open(&target.id) {
                Ok(device) => device,
                Err(error) => {
                    let status = camera_error_status(error.kind());
                    let _ = main_weak.upgrade_in_event_loop(move |win| {
                        win.set_camera_connected(false);
                        win.set_is_streaming(false);
                        win.set_status_text(status.into());
                    });
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let discovered_controls = device
                .capabilities()
                .controls
                .iter()
                .cloned()
                .filter_map(|descriptor| {
                    let fallback = default_camera_control_value(&descriptor.kind)?;
                    let value = device.control_value(&descriptor.id).unwrap_or(fallback);
                    Some(CameraControlRuntimeState {
                        key: camera_control_key(&descriptor.id),
                        descriptor,
                        value,
                    })
                })
                .collect::<Vec<_>>();
            if let Ok(mut controls) = camera_controls_worker.lock() {
                controls.clone_from(&discovered_controls);
            }

            let candidates = device
                .capabilities()
                .ranked_modes()
                .into_iter()
                .map(|(mode, frame_rate)| StreamConfiguration {
                    pixel_format: mode.pixel_format.clone(),
                    resolution: mode.resolution,
                    frame_rate,
                })
                .collect::<Vec<_>>();

            let mut active_config = None;
            for candidate in candidates {
                if device.start_stream(&candidate).is_ok() {
                    active_config = Some(candidate);
                    break;
                }
            }

            let Some(config) = active_config else {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("Flux DE400 indisponible".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            if let Ok(mut active) = active_stream_configuration_worker.lock() {
                *active = Some(config.clone());
            }

            let _ = main_weak.upgrade_in_event_loop({
                let dev_name = target.display_name.clone();
                let dev_id = target.id.to_string();
                let backend_name = backend.kind().to_string();
                let usb_text = usb_info.clone();
                let format_text = config.pixel_format.to_string();
                let res_text = config.resolution.to_string();
                let fps_text = format!("{:.2} fps", config.frame_rate.frames_per_second());
                let controls = discovered_controls.clone();

                move |win| {
                    win.set_camera_connected(true);
                    win.set_is_streaming(true);
                    win.set_status_text("DE400 Connecté".into());

                    let mut diag = win.get_diagnostics();
                    diag.device_name = dev_name.into();
                    diag.device_id = dev_id.into();
                    diag.backend_name = backend_name.into();
                    diag.usb_info = usb_text.into();
                    diag.active_format = format_text.into();
                    diag.active_resolution = res_text.into();
                    diag.active_fps = fps_text.into();
                    win.set_diagnostics(diag);
                    set_camera_control_model(&win, &controls);
                }
            });

            // Streaming loop
            let mut frame_count: u64 = 0;
            let mut last_stat_time = Instant::now();
            let mut stat_frames = 0;

            loop {
                // Check commands
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        WorkerCommand::SetControl(id, val) => {
                            let _ = device.set_control_value(&id, &val);
                        }
                        WorkerCommand::ResetControls => {
                            let _ = device.reset_controls();
                        }
                        WorkerCommand::Stop => {
                            finalize_recording(
                                &is_rec_clone,
                                &video_writer_clone,
                                &rec_start_clone,
                            );
                            let _ = device.stop_stream();
                            return;
                        }
                    }
                }

                match device.next_event(Duration::from_millis(500)) {
                    Ok(CameraEvent::Frame(frame)) => {
                        frame_count = frame_count.saturating_add(1);
                        stat_frames += 1;

                        latest_frame_clone.publish(frame.clone());

                        // Decode frame for Slint
                        let decode_start = Instant::now();
                        let rgb_opt = match frame.pixel_format {
                            PixelFormat::Mjpeg => decode_mjpeg_to_rgb8(&frame.data).ok(),
                            PixelFormat::Yuyv => {
                                let rgb = convert_yuyv_to_rgb8(
                                    &frame.data,
                                    frame.resolution.width,
                                    frame.resolution.height,
                                );
                                Some((frame.resolution.width, frame.resolution.height, rgb))
                            }
                            PixelFormat::Bgra8 => convert_bgra8_to_rgb8(
                                &frame.data,
                                frame.resolution.width,
                                frame.resolution.height,
                            )
                            .ok()
                            .map(|rgb| (frame.resolution.width, frame.resolution.height, rgb)),
                            _ => None,
                        };
                        let decode_dur = decode_start.elapsed();

                        if is_rec_clone.load(Ordering::Relaxed)
                            && let Ok(mut writer_guard) = video_writer_clone.lock()
                            && let Some(writer) = writer_guard.as_mut()
                        {
                            match frame.pixel_format {
                                PixelFormat::Mjpeg => {
                                    let jpeg = ensure_jpeg_has_dht(&frame.data);
                                    let _ = writer.write_frame(jpeg.as_ref());
                                }
                                _ => {
                                    if let Some((width, height, rgb)) = rgb_opt.as_ref()
                                        && let Ok(jpeg) = encode_rgb8_jpeg(rgb, *width, *height, 95)
                                    {
                                        let _ = writer.write_frame(&jpeg);
                                    }
                                }
                            }
                        }

                        if let Some((width, height, raw_rgb)) = rgb_opt {
                            let _ = main_weak.upgrade_in_event_loop(move |win| {
                                if win.get_is_frozen() {
                                    return;
                                }

                                let pixel_buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                                    &raw_rgb, width, height,
                                );
                                win.set_live_frame(slint::Image::from_rgb8(pixel_buffer));
                            });
                        }

                        // Update FPS & Timer metrics every 1s
                        if last_stat_time.elapsed() >= Duration::from_secs(1) {
                            let fps =
                                f64::from(stat_frames) / last_stat_time.elapsed().as_secs_f64();
                            last_stat_time = Instant::now();
                            stat_frames = 0;

                            let rec_dur_str =
                                rec_start_clone.lock().ok().and_then(|g| *g).map(|st| {
                                    let el = st.elapsed().as_secs();
                                    format!("{:02}:{:02}", el / 60, el % 60)
                                });

                            let should_hide_toast = toast_clone
                                .lock()
                                .ok()
                                .and_then(|g| *g)
                                .is_some_and(|st| st.elapsed() >= Duration::from_secs(4));

                            if should_hide_toast && let Ok(mut g) = toast_clone.lock() {
                                *g = None;
                            }

                            let _ = main_weak.upgrade_in_event_loop(move |win| {
                                let mut diag = win.get_diagnostics();
                                diag.measured_fps = format!("{fps:.2} fps").into();
                                diag.decode_time_ms =
                                    format!("{:.1} ms", decode_dur.as_secs_f64() * 1000.0).into();
                                diag.frame_count = i32::try_from(frame_count).unwrap_or(i32::MAX);
                                win.set_diagnostics(diag);

                                if let Some(dur) = rec_dur_str {
                                    win.set_recording_duration(dur.into());
                                }
                                if should_hide_toast {
                                    win.set_show_last_capture(false);
                                }
                            });
                        }
                    }
                    Ok(CameraEvent::HardwareButtonPressed) => {
                        let behavior = settings_snapshot(&settings_worker).physical_button_behavior;
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            dispatch_hardware_button(&win, behavior);
                        });
                    }
                    Ok(CameraEvent::Disconnected) => {
                        let recording_was_active = finalize_recording(
                            &is_rec_clone,
                            &video_writer_clone,
                            &rec_start_clone,
                        );
                        let _ = device.stop_stream();
                        if let Ok(mut active) = active_stream_configuration_worker.lock() {
                            *active = None;
                        }
                        if let Ok(mut controls) = camera_controls_worker.lock() {
                            controls.clear();
                        }
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            win.set_camera_connected(false);
                            win.set_is_streaming(false);
                            win.set_is_recording(false);
                            win.set_recording_duration("00:00".into());
                            win.set_status_text(
                                "DE400 déconnecté — reconnexion en cours...".into(),
                            );
                            if recording_was_active {
                                win.set_last_capture_message(
                                    "Vidéo arrêtée et finalisée après déconnexion caméra.".into(),
                                );
                                win.set_show_last_capture(true);
                            }
                            win.set_camera_controls(ModelRc::new(VecModel::from(Vec::new())));
                        });
                        break;
                    }
                    Err(error) if error.kind() == CameraErrorKind::TimedOut => {}
                    Err(error) => {
                        let error_kind = error.kind();
                        let recording_was_active = finalize_recording(
                            &is_rec_clone,
                            &video_writer_clone,
                            &rec_start_clone,
                        );
                        let _ = device.stop_stream();
                        if let Ok(mut active) = active_stream_configuration_worker.lock() {
                            *active = None;
                        }
                        if let Ok(mut controls) = camera_controls_worker.lock() {
                            controls.clear();
                        }
                        let status = if matches!(
                            error_kind,
                            CameraErrorKind::Disconnected | CameraErrorKind::DeviceNotFound
                        ) {
                            "DE400 déconnecté — reconnexion en cours...".to_owned()
                        } else {
                            format!("{} — reconnexion en cours...", camera_error_status(error_kind))
                        };
                        let _ = main_weak.upgrade_in_event_loop(move |win| {
                            win.set_camera_connected(false);
                            win.set_is_streaming(false);
                            win.set_is_recording(false);
                            win.set_recording_duration("00:00".into());
                            win.set_status_text(status.into());
                            if recording_was_active {
                                win.set_last_capture_message(
                                    "Vidéo arrêtée et finalisée après interruption caméra.".into(),
                                );
                                win.set_show_last_capture(true);
                            }
                            win.set_camera_controls(ModelRc::new(VecModel::from(Vec::new())));
                        });
                        thread::sleep(Duration::from_millis(100));
                        break;
                    }
                    Ok(_) => {}
                }
            }
        }
    });

    // Hook callbacks
    let refresh_lib_for_win =
        |win: &MainWindow, dir: &std::path::Path, sess: &Arc<Mutex<CaptureSession>>| {
            let cur_session = sess
                .lock()
                .map_or_else(|_| CaptureSession::default(), |g| g.clone());
            let items = load_library_items(dir, &cur_session);
            win.set_library_items(ModelRc::new(VecModel::from(items)));
        };

    // Initial library population
    let initial_capture_directory = settings_snapshot(&settings).capture_directory;
    refresh_lib_for_win(&main_window, &initial_capture_directory, &active_session);

    let weak = main_window.as_weak();
    let latest_frame_cap = Arc::clone(&latest_frame);
    let frozen_frame_cap = Arc::clone(&frozen_frame);
    let settings_cap = Arc::clone(&settings);
    let session_cap = Arc::clone(&active_session);
    let toast_cap = Arc::clone(&last_toast_time);

    main_window.on_trigger_capture(move || {
        let Some(win) = weak.upgrade() else { return };
        let frame_opt = if win.get_is_frozen() {
            frozen_frame_cap
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .or_else(|| latest_frame_cap.snapshot())
        } else {
            latest_frame_cap.snapshot()
        };

        let Some(frame) = frame_opt else {
            eprintln!("[IrisScope] Capture demandée mais aucune frame caméra disponible");
            return;
        };

        let session = match capture_session_from_window(&win) {
            Ok(session) => session,
            Err(message) => {
                win.set_last_capture_message(message.into());
                win.set_show_last_capture(true);
                return;
            }
        };
        if let Ok(mut guard) = session_cap.lock() {
            *guard = session.clone();
        }
        let timestamp = CaptureTimestamp::now();
        let capture_settings = settings_snapshot(&settings_cap);
        let policy = CaptureNamingPolicy::new(&capture_settings.filename_template);
        let (ext, data_to_save) = match frame.pixel_format {
            PixelFormat::Mjpeg => ("jpg", ensure_jpeg_has_dht(&frame.data)),
            PixelFormat::Yuyv => {
                let rgb = convert_yuyv_to_rgb8(
                    &frame.data,
                    frame.resolution.width,
                    frame.resolution.height,
                );
                let png =
                    match encode_rgb8_png(&rgb, frame.resolution.width, frame.resolution.height) {
                        Ok(png) => png,
                        Err(error) => {
                            win.set_last_capture_message(
                                format!("Erreur de conversion photo : {error}").into(),
                            );
                            win.set_show_last_capture(true);
                            return;
                        }
                    };
                ("png", std::borrow::Cow::Owned(png))
            }
            PixelFormat::Bgra8 => {
                let rgb = match convert_bgra8_to_rgb8(
                    &frame.data,
                    frame.resolution.width,
                    frame.resolution.height,
                ) {
                    Ok(rgb) => rgb,
                    Err(error) => {
                        win.set_last_capture_message(
                            format!("Erreur de conversion photo : {error}").into(),
                        );
                        win.set_show_last_capture(true);
                        return;
                    }
                };
                let png =
                    match encode_rgb8_png(&rgb, frame.resolution.width, frame.resolution.height) {
                        Ok(png) => png,
                        Err(error) => {
                            win.set_last_capture_message(
                                format!("Erreur de conversion photo : {error}").into(),
                            );
                            win.set_show_last_capture(true);
                            return;
                        }
                    };
                ("png", std::borrow::Cow::Owned(png))
            }
            _ => {
                win.set_last_capture_message(
                    "Le format caméra actif ne peut pas encore être enregistré en photo.".into(),
                );
                win.set_show_last_capture(true);
                return;
            }
        };
        let file_name = policy.filename(&session, timestamp, ext);

        match save_new_capture(
            &capture_settings.capture_directory,
            &file_name,
            data_to_save.as_ref(),
        ) {
            Ok(saved_path) => {
                if let Err(error) = record_capture_metadata(
                    &capture_settings.capture_directory,
                    &saved_path,
                    &session,
                    CaptureKind::Photo,
                    timestamp,
                ) {
                    eprintln!("[IrisScope] Index bibliothèque non mis à jour : {error}");
                }

                let display_stem = saved_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(&file_name);

                // 1. Shutter flash effect
                win.set_shutter_flash_opacity(0.85);
                slint::Timer::single_shot(Duration::from_millis(50), {
                    let weak_flash = win.as_weak();
                    move || {
                        if let Some(w) = weak_flash.upgrade() {
                            w.set_shutter_flash_opacity(0.0);
                        }
                    }
                });

                // 2. Play shutter sound in background
                thread::spawn(|| {
                    let sound_path = "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga";
                    if std::path::Path::new(sound_path).exists() {
                        let _ = std::process::Command::new("paplay")
                            .arg(sound_path)
                            .status()
                            .or_else(|_| {
                                std::process::Command::new("pw-play")
                                    .arg(sound_path)
                                    .status()
                            });
                    }
                });

                // 3. Update session photo counter
                let new_count = win.get_session_photo_count() + 1;
                win.set_session_photo_count(new_count);

                // 4. Update thumbnail preview card
                if let Ok((tw, th, raw_rgb)) = decode_mjpeg_to_rgb8(&data_to_save) {
                    let pixel_buffer =
                        SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&raw_rgb, tw, th);
                    win.set_last_capture_thumbnail(slint::Image::from_rgb8(pixel_buffer));
                    win.set_has_last_capture(true);
                    win.set_last_capture_file_name(display_stem.into());
                    win.set_last_capture_path(saved_path.to_string_lossy().to_string().into());
                }

                // 5. Toast notification
                win.set_last_capture_message(
                    format!("✓ Photo #{new_count} enregistrée : {display_stem}").into(),
                );
                win.set_show_last_capture(true);
                if let Ok(mut g) = toast_cap.lock() {
                    *g = Some(Instant::now());
                }
                refresh_lib_for_win(&win, &capture_settings.capture_directory, &session_cap);
            }
            Err(err) => {
                win.set_last_capture_message(format!("Erreur d'enregistrement : {err}").into());
                win.set_show_last_capture(true);
            }
        }
    });

    // Recording callback
    let weak_rec = main_window.as_weak();
    let is_rec = Arc::clone(&is_recording);
    let video_writer_rec = Arc::clone(&video_writer);
    let settings_rec = Arc::clone(&settings);
    let session_rec = Arc::clone(&active_session);
    let rec_start_rec = Arc::clone(&recording_start);
    let toast_rec = Arc::clone(&last_toast_time);
    let active_stream_configuration_rec = Arc::clone(&active_stream_configuration);
    let latest_frame_rec = Arc::clone(&latest_frame);

    main_window.on_toggle_recording(move || {
        let Some(win) = weak_rec.upgrade() else {
            return;
        };
        let currently_recording = is_rec.load(Ordering::Relaxed);
        let recording_settings = settings_snapshot(&settings_rec);

        if currently_recording {
            // Stop recording
            finalize_recording(&is_rec, &video_writer_rec, &rec_start_rec);
            win.set_is_recording(false);
            win.set_recording_duration("00:00".into());
            win.set_last_capture_message("✓ Vidéo enregistrée".into());
            win.set_show_last_capture(true);
            if let Ok(mut g) = toast_rec.lock() {
                *g = Some(Instant::now());
            }
            refresh_lib_for_win(&win, &recording_settings.capture_directory, &session_rec);
        } else {
            // Start recording
            let session = match capture_session_from_window(&win) {
                Ok(session) => session,
                Err(message) => {
                    win.set_last_capture_message(message.into());
                    win.set_show_last_capture(true);
                    return;
                }
            };
            if let Ok(mut current_session) = session_rec.lock() {
                *current_session = session.clone();
            }

            let Some(configuration) = active_stream_configuration_rec
                .lock()
                .ok()
                .and_then(|active| active.clone())
            else {
                win.set_last_capture_message("Erreur vidéo : aucun flux caméra actif".into());
                win.set_show_last_capture(true);
                return;
            };

            let Some(current_frame) = latest_frame_rec.snapshot() else {
                win.set_last_capture_message(
                    "Erreur vidéo : aucune frame caméra disponible".into(),
                );
                win.set_show_last_capture(true);
                return;
            };
            let timestamp = CaptureTimestamp::now();
            let policy = CaptureNamingPolicy::new(&recording_settings.filename_template);
            let file_name = policy.filename(&session, timestamp, "avi");

            match AviMjpegWriter::create_unique(
                &recording_settings.capture_directory,
                &file_name,
                current_frame.resolution.width,
                current_frame.resolution.height,
                configuration.frame_rate,
            ) {
                Ok((writer, saved_path)) => {
                    if let Err(error) = record_capture_metadata(
                        &recording_settings.capture_directory,
                        &saved_path,
                        &session,
                        CaptureKind::Video,
                        timestamp,
                    ) {
                        eprintln!("[IrisScope] Index bibliothèque non mis à jour : {error}");
                    }

                    if let Ok(mut writer_guard) = video_writer_rec.lock() {
                        *writer_guard = Some(writer);
                    }
                    is_rec.store(true, Ordering::Relaxed);
                    if let Ok(mut g) = rec_start_rec.lock() {
                        *g = Some(Instant::now());
                    }
                    win.set_is_recording(true);
                    win.set_recording_duration("00:00".into());
                    win.set_last_capture_path(saved_path.to_string_lossy().to_string().into());
                    if let Some(name) = saved_path.file_name().and_then(|value| value.to_str()) {
                        win.set_last_capture_file_name(name.into());
                    }
                }
                Err(err) => {
                    win.set_last_capture_message(format!("Erreur vidéo : {err}").into());
                    win.set_show_last_capture(true);
                }
            }
        }
    });

    // Session clearing
    let weak_session = main_window.as_weak();
    let session_clear = Arc::clone(&active_session);
    let settings_clear = Arc::clone(&settings);
    main_window.on_clear_session(move || {
        let Some(win) = weak_session.upgrade() else {
            return;
        };
        win.set_patient_first_name("".into());
        win.set_patient_last_name("".into());
        win.set_selected_eye(0);
        win.set_session_photo_count(0);
        win.set_has_last_capture(false);
        if let Ok(mut s) = session_clear.lock() {
            s.clear();
        }
        let directory = settings_snapshot(&settings_clear).capture_directory;
        refresh_lib_for_win(&win, &directory, &session_clear);
    });

    // Refresh library callback
    let weak_refresh = main_window.as_weak();
    let settings_refresh = Arc::clone(&settings);
    let session_refresh = Arc::clone(&active_session);
    main_window.on_refresh_library(move || {
        let Some(win) = weak_refresh.upgrade() else {
            return;
        };
        let directory = settings_snapshot(&settings_refresh).capture_directory;
        refresh_lib_for_win(&win, &directory, &session_refresh);
    });

    // Open photos and IrisScope MJPEG videos inside the private in-app viewer.
    let weak_viewer = main_window.as_weak();
    let viewer_generation_open = Arc::clone(&viewer_generation);
    let viewer_playing_open = Arc::clone(&viewer_video_playing);
    main_window.on_open_capture_file(move |file_path_str| {
        let Some(win) = weak_viewer.upgrade() else {
            return;
        };
        let path = std::path::PathBuf::from(file_path_str.as_str());
        if !path.exists() {
            return;
        }

        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if matches!(extension.as_str(), "jpg" | "jpeg" | "png") {
            viewer_generation_open.fetch_add(1, Ordering::Relaxed);
            viewer_playing_open.store(false, Ordering::Relaxed);

            if let Ok(bytes) = std::fs::read(&path)
                && let Ok((width, height, rgb)) = decode_image_to_rgb8(&bytes)
            {
                let pixels = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
                win.set_viewer_image(slint::Image::from_rgb8(pixels));
                win.set_viewer_is_video(false);
                win.set_viewer_video_playing(false);
                win.set_viewer_open(true);
            }
            return;
        }

        if extension != "avi" {
            win.set_last_capture_message(
                "Ce format vidéo n'est pas encore lisible dans IrisScope.".into(),
            );
            win.set_show_last_capture(true);
            return;
        }

        let Ok(mut reader) = AviMjpegReader::open(&path) else {
            win.set_last_capture_message("Vidéo AVI illisible ou non compatible.".into());
            win.set_show_last_capture(true);
            return;
        };

        let generation = viewer_generation_open
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        viewer_playing_open.store(true, Ordering::Relaxed);
        win.set_viewer_is_video(true);
        win.set_viewer_video_playing(true);
        win.set_viewer_open(true);

        let weak_playback = weak_viewer.clone();
        let generation_state = Arc::clone(&viewer_generation_open);
        let playing_state = Arc::clone(&viewer_playing_open);

        thread::spawn(move || {
            let frame_count = reader.frame_count();
            let fps = reader.frame_rate().frames_per_second().max(0.5);
            let frame_duration = Duration::from_secs_f64(1.0 / fps);
            let mut frame_index = 0_usize;

            while generation_state.load(Ordering::Relaxed) == generation {
                if !playing_state.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }

                let Ok(jpeg) = reader.read_frame(frame_index) else {
                    break;
                };
                let jpeg = ensure_jpeg_has_dht(&jpeg);

                if let Ok((width, height, rgb)) = decode_mjpeg_to_rgb8(&jpeg) {
                    let _ = weak_playback.upgrade_in_event_loop(move |viewer| {
                        let pixels =
                            SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&rgb, width, height);
                        viewer.set_viewer_image(slint::Image::from_rgb8(pixels));
                    });
                }

                frame_index = (frame_index + 1) % frame_count;
                thread::sleep(frame_duration);
            }
        });
    });

    let viewer_generation_close = Arc::clone(&viewer_generation);
    let viewer_playing_close = Arc::clone(&viewer_video_playing);
    let weak_close_viewer = main_window.as_weak();
    main_window.on_close_viewer(move || {
        viewer_generation_close.fetch_add(1, Ordering::Relaxed);
        viewer_playing_close.store(false, Ordering::Relaxed);
        if let Some(win) = weak_close_viewer.upgrade() {
            win.set_viewer_is_video(false);
            win.set_viewer_video_playing(false);
        }
    });

    let viewer_playing_toggle = Arc::clone(&viewer_video_playing);
    let weak_toggle_viewer = main_window.as_weak();
    main_window.on_toggle_viewer_video(move || {
        let playing = !viewer_playing_toggle.load(Ordering::Relaxed);
        viewer_playing_toggle.store(playing, Ordering::Relaxed);
        if let Some(win) = weak_toggle_viewer.upgrade() {
            win.set_viewer_video_playing(playing);
        }
    });

    // Freeze frame
    let weak_freeze = main_window.as_weak();
    let latest_frame_freeze = Arc::clone(&latest_frame);
    let frozen_frame_freeze = Arc::clone(&frozen_frame);
    main_window.on_toggle_freeze(move || {
        let Some(win) = weak_freeze.upgrade() else {
            return;
        };
        let was_frozen = win.get_is_frozen();
        let new_frozen = !was_frozen;
        win.set_is_frozen(new_frozen);
        if new_frozen {
            if let Ok(mut g) = frozen_frame_freeze.lock() {
                *g = latest_frame_freeze.snapshot();
            }
        } else if let Ok(mut g) = frozen_frame_freeze.lock() {
            *g = None;
        }
    });

    // Camera controls discovered dynamically from the active backend.
    let cmd_tx_value = cmd_tx.clone();
    let controls_value = Arc::clone(&camera_controls);
    let weak_value = main_window.as_weak();
    main_window.on_set_camera_control_value(move |key, requested| {
        let Some(win) = weak_value.upgrade() else {
            return;
        };
        let Ok(mut controls) = controls_value.lock() else {
            return;
        };
        let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
            return;
        };
        if state.descriptor.read_only {
            return;
        }
        let Some(value) = snap_integer_control_value(&state.descriptor, requested) else {
            return;
        };
        state.value = value.clone();
        let _ = cmd_tx_value.send(WorkerCommand::SetControl(
            state.descriptor.id.clone(),
            value,
        ));
        set_camera_control_model(&win, &controls);
    });

    let cmd_tx_bool = cmd_tx.clone();
    let controls_bool = Arc::clone(&camera_controls);
    let weak_bool = main_window.as_weak();
    main_window.on_set_camera_control_bool(move |key, requested| {
        let Some(win) = weak_bool.upgrade() else {
            return;
        };
        let Ok(mut controls) = controls_bool.lock() else {
            return;
        };
        let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
            return;
        };
        if state.descriptor.read_only
            || !matches!(state.descriptor.kind, CameraControlKind::Boolean { .. })
        {
            return;
        }
        let value = CameraControlValue::Boolean(requested);
        state.value = value.clone();
        let _ = cmd_tx_bool.send(WorkerCommand::SetControl(
            state.descriptor.id.clone(),
            value,
        ));
        set_camera_control_model(&win, &controls);
    });

    let cmd_tx_menu = cmd_tx.clone();
    let controls_menu = Arc::clone(&camera_controls);
    let weak_menu = main_window.as_weak();
    main_window.on_cycle_camera_control_menu(move |key| {
        let Some(win) = weak_menu.upgrade() else {
            return;
        };
        let Ok(mut controls) = controls_menu.lock() else {
            return;
        };
        let Some(state) = controls.iter_mut().find(|state| state.key == key.as_str()) else {
            return;
        };
        if state.descriptor.read_only {
            return;
        }

        let CameraControlKind::Menu { items, .. } = &state.descriptor.kind else {
            return;
        };
        if items.is_empty() {
            return;
        }

        let current = match state.value {
            CameraControlValue::Menu(value) => value,
            _ => items[0].value,
        };
        let current_index = items
            .iter()
            .position(|item| item.value == current)
            .unwrap_or_default();
        let next = items[(current_index + 1) % items.len()].value;
        let value = CameraControlValue::Menu(next);
        state.value = value.clone();
        let _ = cmd_tx_menu.send(WorkerCommand::SetControl(
            state.descriptor.id.clone(),
            value,
        ));
        set_camera_control_model(&win, &controls);
    });

    let cmd_tx_reset = cmd_tx.clone();
    let controls_reset = Arc::clone(&camera_controls);
    let weak_reset = main_window.as_weak();
    main_window.on_reset_camera_controls(move || {
        let Some(win) = weak_reset.upgrade() else {
            return;
        };
        if let Ok(mut controls) = controls_reset.lock() {
            for state in controls.iter_mut() {
                if let Some(value) = default_camera_control_value(&state.descriptor.kind) {
                    state.value = value;
                }
            }
            set_camera_control_model(&win, &controls);
        }
        let _ = cmd_tx_reset.send(WorkerCommand::ResetControls);
    });

    // Persistent application settings
    let settings_directory = Arc::clone(&settings);
    let settings_path_directory = settings_path.clone();
    let session_directory = Arc::clone(&active_session);
    let weak_directory = main_window.as_weak();
    main_window.on_update_capture_directory(move |value| {
        let Some(win) = weak_directory.upgrade() else {
            return;
        };
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let directory = std::path::PathBuf::from(value);
        if let Ok(mut guard) = settings_directory.lock() {
            guard.capture_directory.clone_from(&directory);
        }
        persist_settings(&settings_directory, &settings_path_directory);
        win.set_settings_capture_directory(directory.to_string_lossy().to_string().into());
        refresh_lib_for_win(&win, &directory, &session_directory);
    });

    let settings_template = Arc::clone(&settings);
    let settings_path_template = settings_path.clone();
    let weak_template = main_window.as_weak();
    main_window.on_update_filename_template(move |value| {
        let Some(win) = weak_template.upgrade() else {
            return;
        };
        let value = value.trim();
        if !filename_template_preserves_identity(value) {
            let current = settings_snapshot(&settings_template).filename_template;
            win.set_settings_filename_template(current.into());
            win.set_last_capture_message(
                "Le modèle doit contenir {prenom}, {nom} et {oeil}.".into(),
            );
            win.set_show_last_capture(true);
            return;
        }
        if let Ok(mut guard) = settings_template.lock() {
            value.clone_into(&mut guard.filename_template);
        }
        persist_settings(&settings_template, &settings_path_template);
        win.set_settings_filename_template(value.into());
    });

    let settings_button = Arc::clone(&settings);
    let settings_path_button = settings_path.clone();
    let weak_button = main_window.as_weak();
    main_window.on_cycle_physical_button_mode(move || {
        let Some(win) = weak_button.upgrade() else {
            return;
        };
        let next = if let Ok(mut guard) = settings_button.lock() {
            guard.physical_button_behavior = match guard.physical_button_behavior {
                PhysicalButtonBehavior::FollowMode => PhysicalButtonBehavior::AlwaysPhoto,
                PhysicalButtonBehavior::AlwaysPhoto => PhysicalButtonBehavior::AlwaysVideo,
                PhysicalButtonBehavior::AlwaysVideo => PhysicalButtonBehavior::FollowMode,
            };
            guard.physical_button_behavior
        } else {
            PhysicalButtonBehavior::FollowMode
        };
        persist_settings(&settings_button, &settings_path_button);
        win.set_settings_button_mode(physical_button_mode_index(next));
    });

    let settings_map = Arc::clone(&settings);
    let settings_path_map = settings_path.clone();
    let weak_map = main_window.as_weak();
    main_window.on_update_iridology_map_path(move |value| {
        let Some(win) = weak_map.upgrade() else {
            return;
        };
        let value = value.trim();
        let path = (!value.is_empty()).then(|| std::path::PathBuf::from(value));
        if let Ok(mut guard) = settings_map.lock() {
            guard.iridology_map_path.clone_from(&path);
        }
        persist_settings(&settings_map, &settings_path_map);
        win.set_settings_iridology_map_path(
            path.as_deref()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
                .into(),
        );
        let snapshot = settings_snapshot(&settings_map);
        apply_reference_images(&win, &snapshot);
    });

    let settings_symbols = Arc::clone(&settings);
    let settings_path_symbols = settings_path.clone();
    let weak_symbols = main_window.as_weak();
    main_window.on_update_iridology_symbols_path(move |value| {
        let Some(win) = weak_symbols.upgrade() else {
            return;
        };
        let value = value.trim();
        let path = (!value.is_empty()).then(|| std::path::PathBuf::from(value));
        if let Ok(mut guard) = settings_symbols.lock() {
            guard.iridology_symbols_path.clone_from(&path);
        }
        persist_settings(&settings_symbols, &settings_path_symbols);
        win.set_settings_iridology_symbols_path(
            path.as_deref()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
                .into(),
        );
        let snapshot = settings_snapshot(&settings_symbols);
        apply_reference_images(&win, &snapshot);
    });

    // Capture directory opener
    let settings_open = Arc::clone(&settings);
    main_window.on_open_capture_directory(move || {
        let dir = settings_snapshot(&settings_open).capture_directory;
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&dir).spawn();
    });

    main_window.run()?;
    let _ = cmd_tx.send(WorkerCommand::Stop);
    Ok(())
}
