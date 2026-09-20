#[cfg(target_os = "linux")]
use iriscope_camera_linux as platform_camera;
#[cfg(target_os = "macos")]
use iriscope_camera_macos as platform_camera;
#[cfg(target_os = "windows")]
use iriscope_camera_windows as platform_camera;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("IrisScope currently supports Linux, Windows, and macOS");

fn main() {
    let mut backend = platform_camera::create_backend();
    println!("IrisScope camera detection ({})", backend.kind());

    match backend.enumerate_devices() {
        Ok(devices) if devices.is_empty() => println!("No video capture device detected"),
        Ok(devices) => {
            for device in devices {
                if let Some(usb) = device.usb {
                    println!(
                        "Camera: {} [{}] USB {:04x}:{:04x}",
                        device.display_name, device.id, usb.vendor_id, usb.product_id
                    );
                } else {
                    println!("Camera: {} [{}]", device.display_name, device.id);
                }
            }
        }
        Err(error) => eprintln!("Camera detection failed: {error}"),
    }
}
