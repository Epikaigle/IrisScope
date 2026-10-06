//! Local display preferences, independent of patient records and source files.
use crate::{
    runtime::AppRuntime,
    ui::{AppState, MainWindow},
};
use iriscope_core::settings::InterfacePreferences;
use slint::ComponentHandle;
use std::sync::Arc;

fn apply(window: &MainWindow, preferences: &InterfacePreferences) {
    let state = window.global::<AppState>();
    state.set_interface_ready(false);
    state.set_remember_layout(preferences.remember_layout);
    state.set_sidebar_visible(preferences.sidebar_visible);
    state.set_camera_panel_width(preferences.camera_panel_width);
    state.set_viewer_panel_width(preferences.viewer_panel_width);
    state.set_consultation_panel_width(preferences.consultation_panel_width);
    state.set_preferred_photo_panel(preferences.photo_panel);
    state.set_library_view(preferences.library_view);
    state.set_thumbnail_size(preferences.thumbnail_size);
    state.set_advanced_settings_expanded(preferences.advanced_settings_expanded);
    state.set_presentation_mode(preferences.presentation_mode);
    state.set_image_only(preferences.image_only);
    state.set_photo_controls_visible(true);
    state.set_interface_ready(true);
}

fn snapshot(window: &MainWindow) -> InterfacePreferences {
    let state = window.global::<AppState>();
    let mut preferences = InterfacePreferences {
        remember_layout: state.get_remember_layout(),
        sidebar_visible: state.get_sidebar_visible(),
        camera_panel_width: state.get_camera_panel_width(),
        viewer_panel_width: state.get_viewer_panel_width(),
        consultation_panel_width: state.get_consultation_panel_width(),
        photo_panel: state.get_preferred_photo_panel(),
        library_view: state.get_library_view(),
        thumbnail_size: state.get_thumbnail_size(),
        advanced_settings_expanded: state.get_advanced_settings_expanded(),
        presentation_mode: state.get_presentation_mode(),
        image_only: state.get_image_only(),
    };
    preferences.normalize();
    preferences.initial_layout()
}

pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    let mut preferences = crate::app_helpers::settings_snapshot(&runtime.settings).interface;
    preferences.normalize();
    apply(window, &preferences.initial_layout());
    let weak = window.as_weak();
    let settings = Arc::clone(&runtime.settings);
    let mailbox = Arc::clone(&runtime.camera_settings_save);
    window.global::<AppState>().on_interface_edited(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if !window.global::<AppState>().get_interface_ready() {
            return;
        }
        let preferences = snapshot(&window);
        let changed = {
            let mut settings = settings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if settings.interface == preferences {
                false
            } else {
                settings.interface = preferences;
                true
            }
        };
        if changed {
            mailbox.mark_dirty();
        }
    });
    let weak = window.as_weak();
    window.global::<AppState>().on_reset_interface(move || {
        if let Some(window) = weak.upgrade() {
            apply(&window, &InterfacePreferences::default());
            window.global::<AppState>().set_viewer_panel(0);
            window.global::<AppState>().invoke_interface_edited();
            window.set_settings_feedback_is_error(false);
            window.set_settings_feedback(
                "Disposition réinitialisée. Les captures et les notes sont conservées.".into(),
            );
        }
    });
    let weak = window.as_weak();
    window.global::<AppState>().on_toggle_presentation(move || {
        if let Some(window) = weak.upgrade() {
            let state = window.global::<AppState>();
            let enabled = !state.get_presentation_mode();
            if enabled {
                state.set_presentation_return_tab(state.get_current_tab());
            }
            state.set_presentation_mode(enabled);
            state.set_viewer_tool(0);
            state.set_library_map_open(false);
            state.set_calendar_open(false);
            state.set_image_controls_open(false);
            if enabled {
                state.set_patient_edit_open(false);
                state.set_current_tab(0);
                if state.get_consultation_open() {
                    state.invoke_close_consultation();
                }
            } else {
                state.set_current_tab(state.get_presentation_return_tab());
            }
            state.invoke_preview_tab_changed(
                state.get_current_tab() == 0
                    && !state.get_viewer_open()
                    && !state.get_consultation_open(),
            );
            state.invoke_show_photo_controls();
            state.invoke_show_viewport_controls();
            state.invoke_interface_edited();
        }
    });
}
