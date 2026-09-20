use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use iriscope_core::{
    camera::{CameraEvent, CapturedFrame, StreamConfiguration},
    capabilities::{CameraControlId, CameraControlValue, PixelFormat, StandardCameraControl},
    capture::LatestFrame,
    library::{CaptureKind, LibraryFilter, present_library_items, scan_library_directory},
    session::{CaptureSession, Eye},
    settings::AppSettings,
    storage::{CaptureNamingPolicy, CaptureTimestamp, save_new_capture},
    video::AviMjpegWriter,
};
use iriscope_imaging::{
    apply_transforms, convert_yuyv_to_rgb8, decode_mjpeg_to_rgb8, ensure_jpeg_has_dht,
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

enum WorkerCommand {
    SetControl(CameraControlId, CameraControlValue),
    ResetControls,
    Stop,
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

    let target = devices
        .iter()
        .find(|d| {
            d.usb
                .as_ref()
                .is_some_and(|u| u.vendor_id == 0x21cd && u.product_id == 0x603b)
        })
        .unwrap_or(&devices[0]);

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

fn load_library_items(
    dir: &std::path::Path,
    active_session: &CaptureSession,
) -> Vec<LibraryItemData> {
    let raw_entries = scan_library_directory(dir);
    let presented = present_library_items(&raw_entries, active_session, LibraryFilter::All);
    presented
        .into_iter()
        .enumerate()
        .map(|(idx, item)| LibraryItemData {
            date_time: item.date_time.into(),
            eye_label: item.eye_label.into(),
            file_path: item.file_path.to_string_lossy().to_string().into(),
            id: idx.to_string().into(),
            is_current_session: item.is_current_session,
            is_video: matches!(item.kind, CaptureKind::Video),
            title: item.display_title.into(),
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let main_window = MainWindow::new()?;
    let settings = Arc::new(AppSettings::default());
    let latest_frame = Arc::new(LatestFrame::new());
    let is_recording = Arc::new(AtomicBool::new(false));
    let video_writer: Arc<Mutex<Option<AviMjpegWriter>>> = Arc::new(Mutex::new(None));
    let active_session = Arc::new(Mutex::new(CaptureSession::default()));
    let recording_start: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let last_toast_time: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let frozen_frame: Arc<Mutex<Option<CapturedFrame>>> = Arc::new(Mutex::new(None));

    let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>();

    // Spawn camera capture worker thread
    let main_weak = main_window.as_weak();
    let latest_frame_clone = Arc::clone(&latest_frame);
    let is_rec_clone = Arc::clone(&is_recording);
    let video_writer_clone = Arc::clone(&video_writer);
    let rec_start_clone = Arc::clone(&recording_start);
    let toast_clone = Arc::clone(&last_toast_time);

    thread::spawn(move || {
        let mut backend = platform_camera::create_backend();
        loop {
            let devices = backend.enumerate_devices().unwrap_or_default();
            if devices.is_empty() {
                let _ = main_weak.upgrade_in_event_loop(|win| {
                    win.set_camera_connected(false);
                    win.set_is_streaming(false);
                    win.set_status_text("DE400 non détecté".into());
                });
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            // Find DE400 or use first camera
            let target = devices
                .iter()
                .find(|d| {
                    d.usb
                        .as_ref()
                        .is_some_and(|u| u.vendor_id == 0x21cd && u.product_id == 0x603b)
                })
                .unwrap_or(&devices[0])
                .clone();

            let usb_info = target.usb.as_ref().map_or_else(
                || "N/A".to_string(),
                |u| format!("{:04x}:{:04x}", u.vendor_id, u.product_id),
            );

            let Ok(mut device) = backend.open(&target.id) else {
                thread::sleep(Duration::from_millis(500));
                continue;
            };

            let Some((pref_mode, pref_fps)) = device.capabilities().preferred_mode() else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            let (pref_mode, pref_fps) = (pref_mode.clone(), pref_fps);

            let config = StreamConfiguration {
                pixel_format: pref_mode.pixel_format.clone(),
                resolution: pref_mode.resolution,
                frame_rate: pref_fps,
            };

            if device.start_stream(&config).is_err() {
                thread::sleep(Duration::from_secs(1));
                continue;
            }

            let _ = main_weak.upgrade_in_event_loop({
                let dev_name = target.display_name.clone();
                let dev_id = target.id.to_string();
                let backend_name = backend.kind().to_string();
                let usb_text = usb_info.clone();
                let format_text = config.pixel_format.to_string();
                let res_text = config.resolution.to_string();
                let fps_text = format!("{:.2} fps", config.frame_rate.frames_per_second());

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
                            let _ = device.stop_stream();
                            return;
                        }
                    }
                }

                match device.next_event(Duration::from_millis(500)) {
                    Ok(CameraEvent::Frame(frame)) => {
                        frame_count = frame_count.saturating_add(1);
                        stat_frames += 1;

                        // Lossless video recording if active
                        if is_rec_clone.load(Ordering::Relaxed)
                            && let Ok(mut writer_guard) = video_writer_clone.lock()
                            && let Some(writer) = writer_guard.as_mut()
                        {
                            let _ = writer.write_frame(&frame.data);
                        }

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
                            _ => None,
                        };
                        let decode_dur = decode_start.elapsed();

                        if let Some((width, height, raw_rgb)) = rgb_opt {
                            let _ = main_weak.upgrade_in_event_loop(move |win| {
                                if win.get_is_frozen() {
                                    return;
                                }

                                let rot = u32::try_from(win.get_rotation_angle().max(0))
                                    .unwrap_or_default();
                                let mir_h = win.get_mirror_horizontal();
                                let mir_v = win.get_mirror_vertical();

                                let (out_w, out_h, final_rgb) =
                                    apply_transforms(&raw_rgb, width, height, rot, mir_h, mir_v);

                                let pixel_buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                                    &final_rgb, out_w, out_h,
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
                        // DE400 button press
                        let _ = main_weak.upgrade_in_event_loop(|win| {
                            win.invoke_trigger_capture();
                        });
                    }
                    Ok(CameraEvent::Disconnected) => {
                        let _ = device.stop_stream();
                        break;
                    }
                    _ => {}
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
    refresh_lib_for_win(&main_window, &settings.capture_directory, &active_session);

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

        let first = win.get_patient_first_name().to_string();
        let last = win.get_patient_last_name().to_string();
        let eye = match win.get_selected_eye() {
            1 => Eye::Left,
            2 => Eye::Right,
            _ => Eye::Unspecified,
        };

        let session = CaptureSession::new(&first, &last, eye);
        if let Ok(mut guard) = session_cap.lock() {
            *guard = session.clone();
        }
        let timestamp = CaptureTimestamp::now();
        let policy = CaptureNamingPolicy::new(&settings_cap.filename_template);
        let ext = match frame.pixel_format {
            PixelFormat::Mjpeg => "jpg",
            _ => "bin",
        };
        let file_name = policy.filename(&session, timestamp, ext);

        let data_to_save = match frame.pixel_format {
            PixelFormat::Mjpeg => ensure_jpeg_has_dht(&frame.data),
            _ => std::borrow::Cow::Borrowed(&frame.data[..]),
        };

        match save_new_capture(
            &settings_cap.capture_directory,
            &file_name,
            data_to_save.as_ref(),
        ) {
            Ok(saved_path) => {
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
                refresh_lib_for_win(&win, &settings_cap.capture_directory, &session_cap);
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

    main_window.on_toggle_recording(move || {
        let Some(win) = weak_rec.upgrade() else {
            return;
        };
        let currently_recording = is_rec.load(Ordering::Relaxed);

        if currently_recording {
            // Stop recording
            is_rec.store(false, Ordering::Relaxed);
            if let Ok(mut g) = rec_start_rec.lock() {
                *g = None;
            }
            win.set_is_recording(false);
            if let Ok(mut writer_guard) = video_writer_rec.lock()
                && let Some(mut writer) = writer_guard.take()
            {
                let _ = writer.finish();
            }
            win.set_last_capture_message("✓ Vidéo enregistrée".into());
            win.set_show_last_capture(true);
            if let Ok(mut g) = toast_rec.lock() {
                *g = Some(Instant::now());
            }
            refresh_lib_for_win(&win, &settings_rec.capture_directory, &session_rec);
        } else {
            // Start recording
            let first = win.get_patient_first_name().to_string();
            let last = win.get_patient_last_name().to_string();
            let eye = match win.get_selected_eye() {
                1 => Eye::Left,
                2 => Eye::Right,
                _ => Eye::Unspecified,
            };

            let session = CaptureSession::new(&first, &last, eye);
            let timestamp = CaptureTimestamp::now();
            let policy = CaptureNamingPolicy::new(&settings_rec.filename_template);
            let file_name = policy.filename(&session, timestamp, "avi");
            let file_path = settings_rec.capture_directory.join(&file_name);

            if let Some(parent) = file_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            match AviMjpegWriter::create(&file_path, 1280, 1024, 8) {
                Ok(writer) => {
                    if let Ok(mut writer_guard) = video_writer_rec.lock() {
                        *writer_guard = Some(writer);
                    }
                    is_rec.store(true, Ordering::Relaxed);
                    if let Ok(mut g) = rec_start_rec.lock() {
                        *g = Some(Instant::now());
                    }
                    win.set_is_recording(true);
                    win.set_recording_duration("00:00".into());
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
        refresh_lib_for_win(&win, &settings_clear.capture_directory, &session_clear);
    });

    // Refresh library callback
    let weak_refresh = main_window.as_weak();
    let settings_refresh = Arc::clone(&settings);
    let session_refresh = Arc::clone(&active_session);
    main_window.on_refresh_library(move || {
        let Some(win) = weak_refresh.upgrade() else {
            return;
        };
        refresh_lib_for_win(&win, &settings_refresh.capture_directory, &session_refresh);
    });

    // Open capture file
    main_window.on_open_capture_file(move |file_path_str| {
        let path = std::path::Path::new(file_path_str.as_str());
        if path.exists() {
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("explorer").arg(path).spawn();
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(path).spawn();
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

    // Camera control adjustments
    let cmd_tx_ctrl = cmd_tx.clone();
    main_window.on_set_control_requested(move |name, value| {
        let id = match name.as_str() {
            "brightness" => CameraControlId::Standard(StandardCameraControl::Brightness),
            "contrast" => CameraControlId::Standard(StandardCameraControl::Contrast),
            "saturation" => CameraControlId::Standard(StandardCameraControl::Saturation),
            "hue" => CameraControlId::Standard(StandardCameraControl::Hue),
            "gamma" => CameraControlId::Standard(StandardCameraControl::Gamma),
            "sharpness" => CameraControlId::Standard(StandardCameraControl::Sharpness),
            "white_balance_temperature" => {
                CameraControlId::Standard(StandardCameraControl::WhiteBalanceManual)
            }
            "power_line_frequency" => {
                CameraControlId::Standard(StandardCameraControl::PowerLineFrequency)
            }
            _ => CameraControlId::PlatformSpecific(name.to_string()),
        };

        #[allow(clippy::cast_possible_truncation)]
        let val = CameraControlValue::Integer(value.round() as i64);
        let _ = cmd_tx_ctrl.send(WorkerCommand::SetControl(id, val));
    });

    let cmd_tx_wb = cmd_tx.clone();
    main_window.on_set_white_balance_auto_requested(move |is_auto| {
        let id = CameraControlId::Standard(StandardCameraControl::WhiteBalanceAutomatic);
        let val = CameraControlValue::Boolean(is_auto);
        let _ = cmd_tx_wb.send(WorkerCommand::SetControl(id, val));
    });

    let cmd_tx_reset = cmd_tx.clone();
    main_window.on_reset_camera_controls(move || {
        let _ = cmd_tx_reset.send(WorkerCommand::ResetControls);
    });

    // Capture directory opener
    let settings_open = Arc::clone(&settings);
    main_window.on_open_capture_directory(move || {
        let dir = &settings_open.capture_directory;
        let _ = std::fs::create_dir_all(dir);
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(dir).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(dir).spawn();
    });

    #[cfg(target_os = "linux")]
    spawn_hardware_button_listener(main_window.as_weak());

    main_window.run()?;
    let _ = cmd_tx.send(WorkerCommand::Stop);
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_hardware_button_listener(weak_win: slint::Weak<MainWindow>) {
    thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let Ok(mut child) = Command::new("dmesg")
            .arg("-w")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };

        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let reader = BufReader::new(stdout);
        let mut last_trigger = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .unwrap_or_else(Instant::now);

        for line in reader.lines().map_while(Result::ok) {
            let is_button_event = (line.contains("Button") && line.contains("pressed"))
                || line.contains("KEY_CAMERA")
                || (line.contains("Digital Microscope") && line.contains("button"));

            if is_button_event && last_trigger.elapsed() >= Duration::from_millis(600) {
                last_trigger = Instant::now();
                let _ = weak_win.upgrade_in_event_loop(|win| {
                    win.invoke_trigger_capture();
                });
            }
        }
    });
}
