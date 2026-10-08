//! Offscreen rendering avoids native event-loop thread and monitor-size limits.
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{rc::Rc, time::Instant};

struct SoftwarePlatform(Instant);

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn duration_since_start(&self) -> std::time::Duration {
        self.0.elapsed()
    }
}

pub fn init() {
    slint::platform::set_platform(Box::new(SoftwarePlatform(Instant::now())))
        .expect("initialize the offscreen software renderer");
}
