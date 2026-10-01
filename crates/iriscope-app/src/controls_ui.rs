use crate::camera_queue::CameraSettingsSaveMailbox;
#[cfg(test)]
use crate::camera_queue::run_camera_settings_save_worker;
use crate::ui::{CameraControlUiData, MainWindow};
use iriscope_core::capabilities::{
    CameraControlDescriptor, CameraControlId, CameraControlKind, CameraControlValue,
};
use iriscope_core::settings::{AppSettings, SavedCameraControlValue};
use slint::{Model, ModelRc, VecModel};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct CameraControlRuntimeState {
    pub(super) key: String,
    pub(super) descriptor: CameraControlDescriptor,
    pub(super) value: CameraControlValue,
}

pub(super) fn camera_control_key(id: &CameraControlId) -> String {
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

pub(super) fn default_camera_control_value(kind: &CameraControlKind) -> Option<CameraControlValue> {
    match kind {
        CameraControlKind::Integer { default, .. } => Some(CameraControlValue::Integer(*default)),
        CameraControlKind::Boolean { default } => Some(CameraControlValue::Boolean(*default)),
        CameraControlKind::Menu { default, .. } => Some(CameraControlValue::Menu(*default)),
        _ => None,
    }
}

pub(super) fn saved_camera_control_value(
    value: &CameraControlValue,
) -> Option<SavedCameraControlValue> {
    match value {
        CameraControlValue::Integer(value) => Some(SavedCameraControlValue::Integer(*value)),
        CameraControlValue::Boolean(value) => Some(SavedCameraControlValue::Boolean(*value)),
        CameraControlValue::Menu(value) => Some(SavedCameraControlValue::Menu(*value)),
        _ => None,
    }
}

pub(super) fn compatible_saved_camera_control_value(
    descriptor: &CameraControlDescriptor,
    saved: &SavedCameraControlValue,
) -> Option<CameraControlValue> {
    if descriptor.read_only {
        return None;
    }
    match (&descriptor.kind, saved) {
        (
            CameraControlKind::Integer {
                minimum,
                maximum,
                step,
                ..
            },
            SavedCameraControlValue::Integer(value),
        ) if value >= minimum
            && value <= maximum
            && (i128::from(*value) - i128::from(*minimum)) % i128::from((*step).max(1)) == 0 =>
        {
            Some(CameraControlValue::Integer(*value))
        }
        (CameraControlKind::Boolean { .. }, SavedCameraControlValue::Boolean(value)) => {
            Some(CameraControlValue::Boolean(*value))
        }
        (CameraControlKind::Menu { items, .. }, SavedCameraControlValue::Menu(value))
            if items.iter().any(|item| item.value == *value) =>
        {
            Some(CameraControlValue::Menu(*value))
        }
        _ => None,
    }
}

pub(super) fn remember_camera_control_value(
    settings: &Arc<Mutex<AppSettings>>,
    mailbox: &CameraSettingsSaveMailbox,
    key: &str,
    value: &CameraControlValue,
) {
    let Some(saved) = saved_camera_control_value(value) else {
        return;
    };
    let Ok(mut settings) = settings.lock() else {
        return;
    };
    if settings.camera_control_values.get(key) == Some(&saved) {
        return;
    }
    settings.camera_control_values.insert(key.to_owned(), saved);
    drop(settings);
    mailbox.mark_dirty();
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn camera_control_ui_data(state: &CameraControlRuntimeState) -> CameraControlUiData {
    let mut data = CameraControlUiData {
        key: state.key.clone().into(),
        name: state.descriptor.name.clone().into(),
        kind: 0,
        minimum: 0.0,
        maximum: 1.0,
        value: 0.0,
        value_label: "".into(),
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
            const MAX_EXACT_F32_INT: i64 = 1 << 24;
            data.kind = if [*minimum, *maximum, *value]
                .iter()
                .any(|n| i128::from(*n).abs() > i128::from(MAX_EXACT_F32_INT))
            {
                3
            } else {
                0
            };
            data.minimum = *minimum as f32;
            data.maximum = *maximum as f32;
            data.value = *value as f32;
            data.value_label = value.to_string().into();
            if !data.value.is_finite() {
                data.value = *default as f32;
            }
        }
        (CameraControlKind::Boolean { .. }, CameraControlValue::Boolean(value)) => {
            data.kind = 1;
            data.boolean_value = *value;
            data.value_label = if *value { "Activé" } else { "Désactivé" }.into();
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
            data.value_label = value.to_string().into();
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

pub(super) fn set_camera_control_model(win: &MainWindow, states: &[CameraControlRuntimeState]) {
    let rows = states
        .iter()
        .map(camera_control_ui_data)
        .collect::<Vec<_>>();
    win.set_camera_controls(ModelRc::new(VecModel::from(rows)));
}

pub(super) fn update_camera_control_row(
    win: &MainWindow,
    states: &[CameraControlRuntimeState],
    index: usize,
) {
    let model = win.get_camera_controls();
    if model.row_count() == states.len()
        && model
            .row_data(index)
            .is_some_and(|row| row.key.as_str() == states[index].key)
    {
        model.set_row_data(index, camera_control_ui_data(&states[index]));
    } else {
        set_camera_control_model(win, states);
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn snap_integer_control_value(
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

    if !requested.is_finite() || minimum > maximum {
        return None;
    }
    let step = i128::from(step.max(1));
    let requested = i128::from(requested.round() as i64);
    let clamped = requested.clamp(i128::from(minimum), i128::from(maximum));
    let snapped = i128::from(minimum) + ((clamped - i128::from(minimum)) / step) * step;
    Some(CameraControlValue::Integer(i64::try_from(snapped).ok()?))
}

#[cfg(test)]
mod camera_control_settings_tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
        thread,
        time::SystemTime,
    };

    use iriscope_core::{
        capabilities::{
            CameraControlDescriptor, CameraControlId, CameraControlKind, CameraControlMenuItem,
            CameraControlValue, StandardCameraControl,
        },
        settings::{AppSettings, SavedCameraControlValue},
    };

    use super::{
        CameraSettingsSaveMailbox, compatible_saved_camera_control_value,
        remember_camera_control_value, run_camera_settings_save_worker, snap_integer_control_value,
    };

    #[test]
    pub(super) fn saved_control_must_match_current_camera_capabilities() {
        let mut descriptor = CameraControlDescriptor {
            id: CameraControlId::Standard(StandardCameraControl::Brightness),
            name: "Luminosité".to_owned(),
            kind: CameraControlKind::Integer {
                minimum: 10,
                maximum: 30,
                step: 5,
                default: 20,
                unit: None,
            },
            read_only: false,
        };
        assert_eq!(
            compatible_saved_camera_control_value(
                &descriptor,
                &SavedCameraControlValue::Integer(25)
            ),
            Some(CameraControlValue::Integer(25))
        );
        for saved in [
            SavedCameraControlValue::Integer(31),
            SavedCameraControlValue::Integer(24),
            SavedCameraControlValue::Boolean(true),
        ] {
            assert_eq!(
                compatible_saved_camera_control_value(&descriptor, &saved),
                None
            );
        }
        descriptor.read_only = true;
        assert_eq!(
            compatible_saved_camera_control_value(
                &descriptor,
                &SavedCameraControlValue::Integer(25)
            ),
            None
        );

        descriptor.read_only = false;
        descriptor.kind = CameraControlKind::Menu {
            items: vec![CameraControlMenuItem {
                value: 3,
                label: "Manuel".to_owned(),
            }],
            default: 3,
        };
        assert_eq!(
            compatible_saved_camera_control_value(&descriptor, &SavedCameraControlValue::Menu(3)),
            Some(CameraControlValue::Menu(3))
        );
        assert_eq!(
            compatible_saved_camera_control_value(&descriptor, &SavedCameraControlValue::Menu(4)),
            None
        );
    }

    #[test]
    pub(super) fn integer_control_snapping_handles_extreme_ranges_without_overflow() {
        let descriptor = CameraControlDescriptor {
            id: CameraControlId::Standard(StandardCameraControl::Brightness),
            name: "Entier extrême".to_owned(),
            kind: CameraControlKind::Integer {
                minimum: i64::MIN,
                maximum: i64::MAX,
                step: 3,
                default: 0,
                unit: None,
            },
            read_only: false,
        };
        assert_eq!(snap_integer_control_value(&descriptor, f32::NAN), None);
        assert_eq!(
            snap_integer_control_value(&descriptor, 0.0),
            Some(CameraControlValue::Integer(-2))
        );
    }

    #[test]
    pub(super) fn changed_control_updates_in_memory_before_debounced_save() {
        let settings = Arc::new(Mutex::new(AppSettings::default()));
        let mailbox = CameraSettingsSaveMailbox::default();
        remember_camera_control_value(
            &settings,
            &mailbox,
            "standard:Brightness",
            &CameraControlValue::Integer(42),
        );
        assert_eq!(
            settings
                .lock()
                .expect("settings")
                .camera_control_values
                .get("standard:Brightness"),
            Some(&SavedCameraControlValue::Integer(42))
        );
        assert!(mailbox.state.lock().expect("mailbox").dirty);
    }

    #[test]
    pub(super) fn closing_save_worker_flushes_last_slider_value() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("valid time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_image_settings_{unique}.json"));
        let settings = Arc::new(Mutex::new(AppSettings::default()));
        let mailbox = Arc::new(CameraSettingsSaveMailbox::default());
        let worker = thread::spawn({
            let settings = Arc::clone(&settings);
            let mailbox = Arc::clone(&mailbox);
            let path = path.clone();
            move || run_camera_settings_save_worker(&mailbox, &settings, &path, None)
        });

        for value in [20, 42, 71] {
            remember_camera_control_value(
                &settings,
                &mailbox,
                "standard:Brightness",
                &CameraControlValue::Integer(value),
            );
        }
        mailbox.close();
        worker.join().expect("save worker");
        let restored = AppSettings::load_from_file(&path);
        assert_eq!(
            restored.camera_control_values.get("standard:Brightness"),
            Some(&SavedCameraControlValue::Integer(71))
        );
        fs::remove_file(path).expect("remove settings");
    }
}
