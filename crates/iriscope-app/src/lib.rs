#[cfg(target_os = "linux")]
pub(crate) use iriscope_camera_linux as platform_camera;
#[cfg(target_os = "macos")]
pub(crate) use iriscope_camera_macos as platform_camera;
#[cfg(target_os = "windows")]
pub(crate) use iriscope_camera_windows as platform_camera;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("IrisScope currently supports Linux, Windows, and macOS");

mod app_helpers;
mod background;
mod camera_controller;
mod camera_queue;
mod capture_controller;
mod config;
mod controls_ui;
mod diagnostic;
mod gui;
mod library_controller;
mod library_ui;
mod library_worker;
mod patient_controller;
mod patient_ui;
mod photo_worker;
mod playback;
mod recording_worker;
mod reference_worker;
mod runtime;
mod settings_controller;
#[doc(hidden)]
pub mod test_support;
pub mod ui;
mod viewer_controller;

pub use diagnostic::run_diagnose;
pub use gui::run_gui;
