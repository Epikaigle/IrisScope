use crate::ui::{AppState, MainWindow};
use iriscope_core::camera::{CapturedFrame, StreamConfiguration};
use iriscope_core::capabilities::{CameraCapabilities, PixelFormat};
use iriscope_core::session::{CaptureSession, Eye};
use iriscope_core::settings::{AppSettings, PhysicalButtonBehavior, VideoQualityPreference};
use iriscope_imaging::{
    convert_bgra8_to_rgb8, convert_nv12_to_rgb8, convert_yuyv_to_rgb8, decode_mjpeg_to_rgb8,
};
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};

pub(super) fn settings_file_path() -> std::path::PathBuf {
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

pub(super) fn settings_snapshot(settings: &Arc<Mutex<AppSettings>>) -> AppSettings {
    settings
        .lock()
        .map_or_else(|_| AppSettings::default(), |guard| guard.clone())
}

pub(super) fn persist_settings(
    settings: &Arc<Mutex<AppSettings>>,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let snapshot = settings
        .lock()
        .map_err(|_| std::io::Error::other("settings lock unavailable"))?
        .clone();
    snapshot.save_to_file(path)
}

pub(super) fn settings_error(win: &MainWindow, message: &str) {
    win.set_settings_feedback_is_error(true);
    win.set_settings_feedback(message.into());
}

pub(super) fn physical_button_mode_index(behavior: PhysicalButtonBehavior) -> i32 {
    match behavior {
        PhysicalButtonBehavior::FollowMode => 0,
        PhysicalButtonBehavior::AlwaysPhoto => 1,
        PhysicalButtonBehavior::AlwaysVideo => 2,
    }
}

pub(super) const fn video_quality_index(quality: VideoQualityPreference) -> i32 {
    match quality {
        VideoQualityPreference::Best => 0,
        VideoQualityPreference::Balanced => 1,
        VideoQualityPreference::Smooth => 2,
    }
}

pub(super) const fn video_quality_from_index(index: i32) -> Option<VideoQualityPreference> {
    match index {
        0 => Some(VideoQualityPreference::Best),
        1 => Some(VideoQualityPreference::Balanced),
        2 => Some(VideoQualityPreference::Smooth),
        _ => None,
    }
}

pub(super) fn capture_session_from_window(
    win: &MainWindow,
) -> Result<CaptureSession, &'static str> {
    let first_name = win.get_patient_first_name().trim().to_owned();
    let last_name = win.get_patient_last_name().trim().to_owned();
    let patient_id = win.get_patient_id().to_string();
    let patient_id = match (first_name.is_empty(), last_name.is_empty()) {
        (true, true) => None,
        (false, false) => Some(
            patient_id
                .parse::<u64>()
                .ok()
                .filter(|id| *id != 0)
                .ok_or("Sélectionnez ou créez un dossier patient avant la capture.")?,
        ),
        _ => return Err("Renseignez le prénom et le nom, ou laissez les deux champs vides."),
    };
    let eye = match win.get_selected_eye() {
        1 => Eye::Left,
        2 => Eye::Right,
        _ => return Err("Sélectionnez l'œil gauche ou droit avant la capture."),
    };

    let mut session = CaptureSession::new(first_name, last_name, eye);
    session.set_patient_id(patient_id);
    Ok(session)
}

pub(super) fn dispatch_hardware_button(win: &MainWindow, behavior: PhysicalButtonBehavior) {
    match behavior {
        PhysicalButtonBehavior::FollowMode => {
            if win.get_is_video_mode() {
                win.global::<AppState>().invoke_toggle_recording();
            } else {
                win.global::<AppState>().invoke_trigger_capture();
            }
        }
        PhysicalButtonBehavior::AlwaysPhoto => win.global::<AppState>().invoke_trigger_capture(),
        PhysicalButtonBehavior::AlwaysVideo => win.global::<AppState>().invoke_toggle_recording(),
    }
}

pub(super) fn decode_camera_frame_to_rgb8(frame: &CapturedFrame) -> Option<(u32, u32, Vec<u8>)> {
    match frame.pixel_format {
        PixelFormat::Mjpeg => decode_mjpeg_to_rgb8(&frame.data).ok(),
        PixelFormat::Yuyv => {
            let rgb =
                convert_yuyv_to_rgb8(&frame.data, frame.resolution.width, frame.resolution.height);
            (!rgb.is_empty()).then_some((frame.resolution.width, frame.resolution.height, rgb))
        }
        PixelFormat::Bgra8 => {
            convert_bgra8_to_rgb8(&frame.data, frame.resolution.width, frame.resolution.height)
                .ok()
                .map(|rgb| (frame.resolution.width, frame.resolution.height, rgb))
        }
        PixelFormat::Nv12 => {
            convert_nv12_to_rgb8(&frame.data, frame.resolution.width, frame.resolution.height)
                .ok()
                .map(|rgb| (frame.resolution.width, frame.resolution.height, rgb))
        }
        _ => None,
    }
}

pub(super) fn ranked_stream_configurations(
    capabilities: &CameraCapabilities,
) -> Vec<StreamConfiguration> {
    capabilities
        .ranked_modes()
        .into_iter()
        .filter(|(mode, _)| {
            matches!(
                mode.pixel_format,
                PixelFormat::Mjpeg | PixelFormat::Yuyv | PixelFormat::Nv12 | PixelFormat::Bgra8
            )
        })
        .map(|(mode, frame_rate)| StreamConfiguration {
            pixel_format: mode.pixel_format.clone(),
            resolution: mode.resolution,
            frame_rate,
        })
        .collect()
}

pub(super) fn stream_configurations_for_quality(
    capabilities: &CameraCapabilities,
    quality: VideoQualityPreference,
) -> Vec<StreamConfiguration> {
    let mut candidates = ranked_stream_configurations(capabilities);
    let preferred_index = match quality {
        VideoQualityPreference::Best => None,
        VideoQualityPreference::Balanced => candidates.first().and_then(|best| {
            let maximum_pixels = best.resolution.pixel_count() * 3 / 4;
            candidates
                .iter()
                .position(|mode| mode.resolution.pixel_count() <= maximum_pixels)
        }),
        VideoQualityPreference::Smooth => candidates
            .iter()
            .enumerate()
            .max_by_key(|(index, mode)| {
                (
                    mode.frame_rate,
                    mode.resolution.pixel_count(),
                    std::cmp::Reverse(*index),
                )
            })
            .map(|(index, _)| index),
    };
    if let Some(index) = preferred_index {
        let preferred = candidates.remove(index);
        candidates.insert(0, preferred);
    }
    candidates
}

#[cfg(test)]
mod video_quality_tests {
    use iriscope_core::capabilities::{
        CameraCapabilities, CameraMode, FrameRate, PixelFormat, Resolution,
    };

    use super::{VideoQualityPreference, stream_configurations_for_quality};

    #[test]
    pub(super) fn quality_choices_select_resolution_or_frame_rate_as_requested() {
        let modes =
            [(1280, 1024, 8), (800, 600, 15), (640, 480, 30)].map(|(width, height, fps)| {
                CameraMode {
                    pixel_format: PixelFormat::Mjpeg,
                    resolution: Resolution::new(width, height),
                    frame_rates: vec![FrameRate::new(fps, 1).expect("valid frame rate")],
                }
            });
        let capabilities = CameraCapabilities {
            modes: modes.to_vec(),
            controls: Vec::new(),
        };
        for (quality, expected_width) in [
            (VideoQualityPreference::Best, 1280),
            (VideoQualityPreference::Balanced, 800),
            (VideoQualityPreference::Smooth, 640),
        ] {
            let selected = stream_configurations_for_quality(&capabilities, quality);
            assert_eq!(selected[0].resolution.width, expected_width);
            assert_eq!(selected.len(), 3);
        }
    }

    #[test]
    fn unsupported_pixel_formats_are_not_attempted() {
        let capabilities = CameraCapabilities {
            modes: vec![
                CameraMode {
                    pixel_format: PixelFormat::Other("GREY".to_owned()),
                    resolution: Resolution::new(1920, 1080),
                    frame_rates: vec![FrameRate::new(30, 1).expect("valid frame rate")],
                },
                CameraMode {
                    pixel_format: PixelFormat::Mjpeg,
                    resolution: Resolution::new(1280, 1024),
                    frame_rates: vec![FrameRate::new(8, 1).expect("valid frame rate")],
                },
            ],
            controls: Vec::new(),
        };

        let candidates =
            stream_configurations_for_quality(&capabilities, VideoQualityPreference::Best);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pixel_format, PixelFormat::Mjpeg);
    }
}
