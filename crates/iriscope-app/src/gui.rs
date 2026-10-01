use crate::library_worker::refresh_library_in_background;
use crate::runtime::AppRuntime;
use crate::ui::MainWindow;
use crate::{
    camera_controller, capture_controller, library_controller, patient_controller,
    settings_controller, viewer_controller,
};
use slint::ComponentHandle;
use std::sync::Arc;

/// Composes controllers and owns their runtime through shutdown.
///
/// # Errors
/// Returns interface initialization or event-loop errors.
pub fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let main_window = MainWindow::new()?;
    let (settings, settings_path) = settings_controller::load_settings(&main_window)?;
    let mut runtime = AppRuntime::new(settings, settings_path);
    runtime.start_workers(&main_window);
    install_controllers(&main_window, &runtime);
    main_window.set_library_loading(true);
    refresh_library_in_background(
        &runtime.library_mailbox,
        &runtime.settings,
        &runtime.active_session,
    );
    library_controller::refresh_pending_recording_count(
        main_window.as_weak(),
        Arc::clone(&runtime.settings),
        &runtime.background_jobs,
    );
    runtime.start_references(&main_window);
    let result = main_window.run();
    runtime.shutdown();
    result?;
    Ok(())
}

pub(crate) fn install_controllers(main_window: &MainWindow, runtime: &AppRuntime) {
    patient_controller::install(main_window, runtime);
    library_controller::install(main_window, runtime);
    capture_controller::install(main_window, runtime);
    viewer_controller::install(main_window, runtime);
    camera_controller::install(main_window, runtime);
    settings_controller::install(main_window, runtime);
    runtime.install_close_guard(main_window);
}
