//! Bounded, authenticated helper RPC and camera-supplied control capabilities.
use iriscope_core::{
    camera::{CameraError, CameraErrorKind, CameraResult},
    capabilities::{
        CameraControlDescriptor, CameraControlId, CameraControlKind, CameraControlMenuItem,
        CameraControlValue, StandardCameraControl,
    },
};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) fn socket_path(stream: &Path) -> PathBuf {
    let mut path = stream.as_os_str().to_owned();
    path.push(".control");
    path.into()
}
fn control_identity(number: i32) -> Option<(CameraControlId, &'static str)> {
    use StandardCameraControl as C;
    let (standard, name) = match number {
        1 => (C::Brightness, "Luminosité"),
        2 => (C::Contrast, "Contraste"),
        3 => (C::Saturation, "Saturation"),
        4 => (C::Hue, "Teinte"),
        5 => (C::Sharpness, "Netteté"),
        6 => (C::Gamma, "Gamma"),
        7 => (C::WhiteBalanceManual, "Balance des blancs"),
        8 => (C::WhiteBalanceAutomatic, "Balance des blancs automatique"),
        9 => (C::PowerLineFrequency, "Anti-scintillement"),
        10 => (C::Exposure, "Exposition"),
        11 => (C::ExposureMode, "Mode d’exposition"),
        12 => {
            return Some((
                CameraControlId::PlatformSpecific("uvc-gain".to_owned()),
                "Gain",
            ));
        }
        13 => {
            return Some((
                CameraControlId::PlatformSpecific("uvc-backlight".to_owned()),
                "Compensation du contre-jour",
            ));
        }
        _ => return None,
    };
    Some((CameraControlId::Standard(standard), name))
}
pub(super) fn control_number(id: &CameraControlId) -> CameraResult<i32> {
    (1..=13)
        .find(|number| control_identity(*number).is_some_and(|(candidate, _)| &candidate == id))
        .ok_or_else(|| invalid("unknown control"))
}
fn invalid(message: impl Into<String>) -> CameraError {
    CameraError::new(
        CameraErrorKind::InvalidConfiguration,
        format!("DE400 USB controls: {}", message.into()),
    )
}
pub(super) fn parse(data: &[u8]) -> CameraResult<Vec<CameraControlDescriptor>> {
    if data.len() > 13 * 32 || data.len() % 32 != 0 {
        return Err(invalid("invalid descriptor size"));
    }
    let mut descriptors = Vec::new();
    for block in data.chunks_exact(32) {
        let values: Vec<i32> = block
            .chunks_exact(4)
            .map(|bytes| i32::from_le_bytes(bytes.try_into().expect("fixed word")))
            .collect();
        let (id, name) =
            control_identity(values[0]).ok_or_else(|| invalid("unknown descriptor"))?;
        let [minimum, maximum, step, default, current] =
            [values[2], values[3], values[4], values[5], values[6]].map(i64::from);
        if minimum >= maximum
            || !matches!(values[7], 0 | 1)
            || descriptors
                .iter()
                .any(|d: &CameraControlDescriptor| d.id == id)
        {
            return Err(invalid("invalid or duplicate descriptor"));
        }
        let kind = match values[1] {
            0 if step > 0 => CameraControlKind::Integer {
                minimum,
                maximum,
                step,
                default,
                unit: None,
            },
            1 if minimum == 0 && maximum == 1 => CameraControlKind::Boolean {
                default: default != 0,
            },
            2 if matches!(values[0], 9 | 11) && minimum >= 0 && maximum <= 8 && step > 0 => {
                let labels = if values[0] == 9 {
                    vec![
                        (0, "Désactivé"),
                        (1, "50 Hz"),
                        (2, "60 Hz"),
                        (3, "Automatique"),
                    ]
                } else {
                    vec![
                        (1, "Manuelle"),
                        (2, "Automatique"),
                        (4, "Priorité à la vitesse"),
                        (8, "Priorité à l’ouverture"),
                    ]
                };
                let items: Vec<_> = labels
                    .into_iter()
                    .filter(|(value, _)| step & (1 << value) != 0)
                    .map(|(value, label)| CameraControlMenuItem {
                        value,
                        label: label.to_owned(),
                    })
                    .collect();
                if items.len() < 2 {
                    return Err(invalid("invalid menu"));
                }
                CameraControlKind::Menu { items, default }
            }
            _ => return Err(invalid("invalid descriptor kind")),
        };
        if ![default, current]
            .into_iter()
            .all(|value| valid_value(&kind, value))
        {
            return Err(invalid("invalid camera-reported value"));
        }
        descriptors.push(CameraControlDescriptor {
            id,
            name: name.to_owned(),
            kind,
            read_only: values[7] != 0,
        });
    }
    Ok(descriptors)
}
fn valid_value(kind: &CameraControlKind, value: i64) -> bool {
    match kind {
        CameraControlKind::Integer {
            minimum,
            maximum,
            step,
            ..
        } => value >= *minimum && value <= *maximum && (value - minimum) % step == 0,
        CameraControlKind::Boolean { .. } => matches!(value, 0 | 1),
        CameraControlKind::Menu { items, .. } => items.iter().any(|item| item.value == value),
        _ => false,
    }
}
pub(super) fn encode(
    descriptor: &CameraControlDescriptor,
    value: &CameraControlValue,
) -> CameraResult<i32> {
    let number = match (&descriptor.kind, value) {
        (CameraControlKind::Integer { .. }, CameraControlValue::Integer(value))
        | (CameraControlKind::Menu { .. }, CameraControlValue::Menu(value)) => *value,
        (CameraControlKind::Boolean { .. }, CameraControlValue::Boolean(value)) => {
            i64::from(*value)
        }
        _ => return Err(invalid("wrong value type")),
    };
    if descriptor.read_only || !valid_value(&descriptor.kind, number) {
        return Err(invalid("value is read-only or outside the camera range"));
    }
    i32::try_from(number).map_err(|_| invalid("value too large"))
}
pub(super) fn decode(
    descriptor: &CameraControlDescriptor,
    value: i32,
) -> CameraResult<CameraControlValue> {
    if !valid_value(&descriptor.kind, i64::from(value)) {
        return Err(invalid("unexpected camera value"));
    }
    Ok(match descriptor.kind {
        CameraControlKind::Boolean { .. } => CameraControlValue::Boolean(value != 0),
        CameraControlKind::Menu { .. } => CameraControlValue::Menu(i64::from(value)),
        _ => CameraControlValue::Integer(i64::from(value)),
    })
}
pub(super) fn request(path: &Path, operation: i32, number: i32, value: i32) -> CameraResult<i32> {
    let run = || -> std::io::Result<(i32, i32)> {
        let mut socket = UnixStream::connect(path)?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        for word in [operation, number, value] {
            socket.write_all(&word.to_le_bytes())?;
        }
        let mut reply = [0; 8];
        socket.read_exact(&mut reply)?;
        Ok((
            i32::from_le_bytes(reply[..4].try_into().expect("fixed word")),
            i32::from_le_bytes(reply[4..].try_into().expect("fixed word")),
        ))
    };
    let (status, actual) = run().map_err(|e| {
        CameraError::new(
            CameraErrorKind::BackendUnavailable,
            format!("DE400 USB control request: {e}"),
        )
    })?;
    if status != 0 {
        return Err(CameraError::new(
            CameraErrorKind::BackendUnavailable,
            format!("DE400 refused USB control {number} ({status})"),
        ));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(words: [i32; 8]) -> Vec<u8> {
        words.into_iter().flat_map(i32::to_le_bytes).collect()
    }
    #[test]
    fn preserves_signed_ranges_and_rejects_bad_steps_before_usb() {
        let controls = parse(&descriptor([1, 0, -64, 64, 2, 0, -4, 0])).unwrap();
        assert_eq!(
            encode(&controls[0], &CameraControlValue::Integer(-62)).unwrap(),
            -62
        );
        assert!(encode(&controls[0], &CameraControlValue::Integer(-63)).is_err());
        assert!(encode(&controls[0], &CameraControlValue::Integer(66)).is_err());
        assert!(encode(&controls[0], &CameraControlValue::Boolean(true)).is_err());
    }
    #[test]
    fn refuses_duplicate_and_invalid_capabilities() {
        let bytes = descriptor([2, 0, 0, 64, 1, 32, 32, 0]);
        assert!(parse(&[bytes.as_slice(), bytes.as_slice()].concat()).is_err());
        for words in [
            [1, 0, 0, 64, 0, 32, 32, 0],
            [8, 1, 0, 1, 1, 0, 2, 0],
            [14, 0, 0, 64, 1, 32, 32, 0],
        ] {
            assert!(parse(&descriptor(words)).is_err());
        }
    }
    #[test]
    fn maps_only_supported_exposure_modes() {
        let controls = parse(&descriptor([11, 2, 1, 8, 258, 8, 1, 0])).unwrap();
        assert_eq!(
            encode(&controls[0], &CameraControlValue::Menu(1)).unwrap(),
            1
        );
        assert!(encode(&controls[0], &CameraControlValue::Menu(2)).is_err());
        assert_eq!(
            decode(&controls[0], 8).unwrap(),
            CameraControlValue::Menu(8)
        );
    }
}
