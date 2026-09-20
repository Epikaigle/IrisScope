#[cfg(target_os = "linux")]
use iriscope_camera_linux as platform_camera;
#[cfg(target_os = "macos")]
use iriscope_camera_macos as platform_camera;
#[cfg(target_os = "windows")]
use iriscope_camera_windows as platform_camera;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("IrisScope currently supports Linux, Windows, and macOS");

fn main() {
    println!(
        "IrisScope workspace initialized ({})",
        platform_camera::BACKEND_NAME
    );
}
