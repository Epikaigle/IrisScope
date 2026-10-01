//! Optional WDM camera controls exposed by the activated Media Foundation source.
//!
//! No second capture filter is opened. All COM calls stay on the source's worker.
//! Standard `IKsControl` properties cover sources without the older IAM interfaces.
//! A source without supported properties simply advertises no controls. See:
//! <https://learn.microsoft.com/en-us/windows/win32/directshow/configure-the-video-quality>
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source>

use iriscope_core::{
    camera::{CameraError, CameraErrorKind, CameraResult},
    capabilities::{
        CameraControlDescriptor, CameraControlId, CameraControlKind, CameraControlMenuItem,
        CameraControlValue, StandardCameraControl,
    },
};
use windows::{
    Win32::Media::{
        DirectShow::{
            CameraControl_Exposure, CameraControl_Flags_Auto, CameraControl_Flags_Manual,
            CameraControl_Focus, IAMCameraControl, IAMVideoProcAmp,
            VideoProcAmp_BacklightCompensation, VideoProcAmp_Brightness, VideoProcAmp_Contrast,
            VideoProcAmp_Gain, VideoProcAmp_Gamma, VideoProcAmp_Hue, VideoProcAmp_Saturation,
            VideoProcAmp_Sharpness, VideoProcAmp_WhiteBalance,
        },
        KernelStreaming::{PROPSETID_VIDCAP_CAMERACONTROL, PROPSETID_VIDCAP_VIDEOPROCAMP},
        MediaFoundation::IMFMediaSource,
    },
    core::{GUID, Interface},
};

use crate::{kernel_controls::KernelControls, platform::windows_device_error};

// CameraControlFlags and VideoProcAmpFlags use the same native AUTO/MANUAL bits.
const AUTO: i32 = CameraControl_Flags_Auto.0;
const MANUAL: i32 = CameraControl_Flags_Manual.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Property {
    ProcAmp(i32),
    Camera(i32),
}

impl Property {
    fn kernel_id(self) -> (GUID, u32) {
        match self {
            Self::ProcAmp(id) => (PROPSETID_VIDCAP_VIDEOPROCAMP, id.cast_unsigned()),
            Self::Camera(id) => (PROPSETID_VIDCAP_CAMERACONTROL, id.cast_unsigned()),
        }
    }
}

#[derive(Clone, Copy)]
enum Transport {
    Iam,
    Kernel,
}

#[derive(Clone, Copy, Debug)]
struct PropertyRange {
    minimum: i32,
    maximum: i32,
    step: i32,
    default: i32,
    modes: i32,
    // GetRange reports supported modes, not a default mode. Preserve the mode
    // observed when opening rather than inventing an automatic/manual default.
    initial_mode: i32,
    writable: bool,
}

impl PropertyRange {
    fn accepts(self, value: i64) -> bool {
        value >= i64::from(self.minimum)
            && value <= i64::from(self.maximum)
            && self.step > 0
            && (value - i64::from(self.minimum)) % i64::from(self.step) == 0
    }

    fn is_valid(self) -> bool {
        self.minimum <= self.maximum
            && self.accepts(i64::from(self.default))
            && self.modes & (AUTO | MANUAL) != 0
            && matches!(self.initial_mode, AUTO | MANUAL)
            && self.modes & self.initial_mode != 0
    }
}

#[derive(Clone, Copy, Debug)]
enum ValueMapping {
    Number,
    AutomaticToggle,
    ExposureMode,
}

struct ControlBinding {
    descriptor: CameraControlDescriptor,
    property: Property,
    mapping: ValueMapping,
}

/// Interfaces and metadata belonging solely to the camera's COM worker thread.
pub(super) struct NativeControls {
    proc_amp: Option<IAMVideoProcAmp>,
    camera: Option<IAMCameraControl>,
    kernel: Option<KernelControls>,
    properties: Vec<(Property, PropertyRange, Transport)>,
    controls: Vec<ControlBinding>,
}

impl NativeControls {
    pub(super) fn discover(source: &IMFMediaSource) -> Self {
        let mut controls = Self {
            proc_amp: source.cast().ok(),
            camera: source.cast().ok(),
            kernel: KernelControls::discover(source),
            properties: Vec::new(),
            controls: Vec::new(),
        };

        for (native, standard, name) in [
            (
                VideoProcAmp_Brightness.0,
                StandardCameraControl::Brightness,
                "Luminosité",
            ),
            (
                VideoProcAmp_Contrast.0,
                StandardCameraControl::Contrast,
                "Contraste",
            ),
            (VideoProcAmp_Hue.0, StandardCameraControl::Hue, "Teinte"),
            (
                VideoProcAmp_Saturation.0,
                StandardCameraControl::Saturation,
                "Saturation",
            ),
            (
                VideoProcAmp_Sharpness.0,
                StandardCameraControl::Sharpness,
                "Netteté",
            ),
            (VideoProcAmp_Gamma.0, StandardCameraControl::Gamma, "Gamma"),
            (
                VideoProcAmp_WhiteBalance.0,
                StandardCameraControl::WhiteBalanceManual,
                "Balance des blancs",
            ),
        ] {
            controls.discover_number(
                Property::ProcAmp(native),
                CameraControlId::Standard(standard),
                name,
                None,
            );
        }
        for (native, id, name) in [
            (VideoProcAmp_Gain.0, "windows.video-proc-amp.gain", "Gain"),
            (
                VideoProcAmp_BacklightCompensation.0,
                "windows.video-proc-amp.backlight",
                "Compensation du contre-jour",
            ),
        ] {
            controls.discover_number(
                Property::ProcAmp(native),
                CameraControlId::PlatformSpecific(id.to_owned()),
                name,
                None,
            );
        }
        controls.discover_number(
            Property::Camera(CameraControl_Exposure.0),
            CameraControlId::Standard(StandardCameraControl::Exposure),
            "Exposition",
            Some("log2(s)"),
        );
        controls.discover_number(
            Property::Camera(CameraControl_Focus.0),
            CameraControlId::Standard(StandardCameraControl::Focus),
            "Mise au point",
            None,
        );
        controls.discover_automatic_modes();
        controls
    }

    pub(super) fn descriptors(&self) -> Vec<CameraControlDescriptor> {
        self.controls
            .iter()
            .map(|binding| binding.descriptor.clone())
            .collect()
    }

    fn discover_number(
        &mut self,
        property: Property,
        id: CameraControlId,
        name: &str,
        unit: Option<&str>,
    ) {
        let Ok((range, transport)) = self.read_range(property) else {
            return;
        };
        if !range.is_valid() {
            return;
        }
        self.properties.push((property, range, transport));
        self.controls.push(ControlBinding {
            descriptor: CameraControlDescriptor {
                id,
                name: name.to_owned(),
                kind: CameraControlKind::Integer {
                    minimum: i64::from(range.minimum),
                    maximum: i64::from(range.maximum),
                    step: i64::from(range.step),
                    default: i64::from(range.default),
                    unit: unit.map(str::to_owned),
                },
                read_only: !range.writable || range.modes & MANUAL == 0,
            },
            property,
            mapping: ValueMapping::Number,
        });
    }

    fn discover_automatic_modes(&mut self) {
        for &(property, range, _) in &self.properties {
            // A one-mode device has no switch to present to the user.
            if !range.writable || range.modes & (AUTO | MANUAL) != (AUTO | MANUAL) {
                continue;
            }
            let (id, name, mapping, kind) = match property {
                Property::ProcAmp(native) if native == VideoProcAmp_WhiteBalance.0 => (
                    StandardCameraControl::WhiteBalanceAutomatic,
                    "Balance des blancs automatique",
                    ValueMapping::AutomaticToggle,
                    CameraControlKind::Boolean {
                        default: range.initial_mode == AUTO,
                    },
                ),
                Property::Camera(native) if native == CameraControl_Exposure.0 => (
                    StandardCameraControl::ExposureMode,
                    "Mode d'exposition",
                    ValueMapping::ExposureMode,
                    CameraControlKind::Menu {
                        items: vec![
                            CameraControlMenuItem {
                                value: i64::from(MANUAL),
                                label: "Manuelle".to_owned(),
                            },
                            CameraControlMenuItem {
                                value: i64::from(AUTO),
                                label: "Automatique".to_owned(),
                            },
                        ],
                        default: i64::from(range.initial_mode),
                    },
                ),
                _ => continue,
            };
            self.controls.push(ControlBinding {
                descriptor: CameraControlDescriptor {
                    id: CameraControlId::Standard(id),
                    name: name.to_owned(),
                    kind,
                    read_only: false,
                },
                property,
                mapping,
            });
        }
    }

    pub(super) fn get(&self, id: &CameraControlId) -> CameraResult<CameraControlValue> {
        let binding = self.binding(id)?;
        let (value, flags) = self.read_property(binding.property)?;
        match binding.mapping {
            ValueMapping::Number if self.range(binding.property)?.accepts(i64::from(value)) => {
                Ok(CameraControlValue::Integer(i64::from(value)))
            }
            ValueMapping::Number => Err(CameraError::new(
                CameraErrorKind::Backend,
                "camera returned a control value outside its advertised range",
            )),
            ValueMapping::AutomaticToggle => match flags & (AUTO | MANUAL) {
                AUTO => Ok(CameraControlValue::Boolean(true)),
                MANUAL => Ok(CameraControlValue::Boolean(false)),
                _ => Err(CameraError::new(
                    CameraErrorKind::Backend,
                    "camera returned an invalid automatic control mode",
                )),
            },
            ValueMapping::ExposureMode => match flags & (AUTO | MANUAL) {
                AUTO => Ok(CameraControlValue::Menu(i64::from(AUTO))),
                MANUAL => Ok(CameraControlValue::Menu(i64::from(MANUAL))),
                _ => Err(CameraError::new(
                    CameraErrorKind::Backend,
                    "camera returned an invalid exposure mode",
                )),
            },
        }
    }

    pub(super) fn set(&self, id: &CameraControlId, value: &CameraControlValue) -> CameraResult<()> {
        let binding = self.binding(id)?;
        if binding.descriptor.read_only {
            return Err(CameraError::new(
                CameraErrorKind::Unsupported,
                "camera control is read-only",
            ));
        }
        let range = self.range(binding.property)?;
        let (native_value, mode) = match (binding.mapping, value) {
            (ValueMapping::Number, CameraControlValue::Integer(value)) if range.accepts(*value) => {
                (i32::try_from(*value).map_err(|_| invalid_value())?, MANUAL)
            }
            (ValueMapping::AutomaticToggle, CameraControlValue::Boolean(automatic)) => {
                let (current, _) = self.read_property(binding.property)?;
                (
                    if range.accepts(i64::from(current)) {
                        current
                    } else {
                        range.default
                    },
                    if *automatic { AUTO } else { MANUAL },
                )
            }
            (ValueMapping::ExposureMode, CameraControlValue::Menu(mode))
                if *mode == i64::from(AUTO) || *mode == i64::from(MANUAL) =>
            {
                let (current, _) = self.read_property(binding.property)?;
                (
                    if range.accepts(i64::from(current)) {
                        current
                    } else {
                        range.default
                    },
                    i32::try_from(*mode).map_err(|_| invalid_value())?,
                )
            }
            _ => return Err(invalid_value()),
        };
        self.write_property(binding.property, native_value, mode)
    }

    pub(super) fn reset(&self) -> CameraResult<()> {
        if self.properties.is_empty() {
            return Err(unavailable());
        }
        let mut first_error = None;
        for &(property, range, _) in &self.properties {
            if !range.writable {
                continue;
            }
            // Restore each underlying property once. Automatic toggles share
            // the same native property as their numeric value.
            if let Err(error) = self.write_property(property, range.default, range.initial_mode) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn binding(&self, id: &CameraControlId) -> CameraResult<&ControlBinding> {
        self.controls
            .iter()
            .find(|binding| binding.descriptor.id == *id)
            .ok_or_else(unavailable)
    }

    fn range(&self, property: Property) -> CameraResult<PropertyRange> {
        self.properties
            .iter()
            .find_map(|&(id, range, _)| (id == property).then_some(range))
            .ok_or_else(unavailable)
    }

    fn read_range(&self, property: Property) -> CameraResult<(PropertyRange, Transport)> {
        if let Ok(range) = self.read_iam_range(property)
            && range.is_valid()
        {
            return Ok((range, Transport::Iam));
        }
        let kernel = self.kernel.as_ref().ok_or_else(unavailable)?;
        let (set, id) = property.kernel_id();
        let native = kernel.range(set, id)?;
        let (current, flags, modes) = kernel.get(set, id)?;
        let range = PropertyRange {
            minimum: native.minimum,
            maximum: native.maximum,
            step: native.step,
            // Some drivers supply only range metadata. In that case reset
            // restores the value observed on opening rather than a guessed
            // factory default. The automatic mode uses the same baseline.
            default: native.default.unwrap_or(current),
            modes,
            initial_mode: flags & (AUTO | MANUAL),
            writable: native.writable,
        };
        Ok((range, Transport::Kernel))
    }

    fn read_iam_range(&self, property: Property) -> CameraResult<PropertyRange> {
        let mut range = PropertyRange {
            minimum: 0,
            maximum: 0,
            step: 0,
            default: 0,
            modes: 0,
            initial_mode: 0,
            writable: true,
        };
        // SAFETY: Interfaces and output pointers are valid on the owning COM worker.
        let result = unsafe {
            match property {
                Property::ProcAmp(id) => self.proc_amp.as_ref().ok_or_else(unavailable)?.GetRange(
                    id,
                    &raw mut range.minimum,
                    &raw mut range.maximum,
                    &raw mut range.step,
                    &raw mut range.default,
                    &raw mut range.modes,
                ),
                Property::Camera(id) => self.camera.as_ref().ok_or_else(unavailable)?.GetRange(
                    id,
                    &raw mut range.minimum,
                    &raw mut range.maximum,
                    &raw mut range.step,
                    &raw mut range.default,
                    &raw mut range.modes,
                ),
            }
        };
        result.map_err(|error| windows_device_error("reading camera control range", &error))?;
        let (_, flags) = self.read_iam_property(property)?;
        range.initial_mode = flags & (AUTO | MANUAL);
        Ok(range)
    }

    fn read_property(&self, property: Property) -> CameraResult<(i32, i32)> {
        let transport = self
            .properties
            .iter()
            .find_map(|&(id, _, transport)| (id == property).then_some(transport))
            .ok_or_else(unavailable)?;
        match transport {
            Transport::Iam => self.read_iam_property(property),
            Transport::Kernel => {
                let (set, id) = property.kernel_id();
                let (value, flags, _) =
                    self.kernel.as_ref().ok_or_else(unavailable)?.get(set, id)?;
                Ok((value, flags))
            }
        }
    }

    fn read_iam_property(&self, property: Property) -> CameraResult<(i32, i32)> {
        let mut value = 0;
        let mut flags = 0;
        // SAFETY: Interfaces and output pointers are valid on the owning COM worker.
        let result = unsafe {
            match property {
                Property::ProcAmp(id) => self.proc_amp.as_ref().ok_or_else(unavailable)?.Get(
                    id,
                    &raw mut value,
                    &raw mut flags,
                ),
                Property::Camera(id) => self.camera.as_ref().ok_or_else(unavailable)?.Get(
                    id,
                    &raw mut value,
                    &raw mut flags,
                ),
            }
        };
        result.map_err(|error| windows_device_error("reading camera control", &error))?;
        Ok((value, flags))
    }

    fn write_property(&self, property: Property, value: i32, flags: i32) -> CameraResult<()> {
        let transport = self
            .properties
            .iter()
            .find_map(|&(id, _, transport)| (id == property).then_some(transport))
            .ok_or_else(unavailable)?;
        if matches!(transport, Transport::Kernel) {
            let (set, id) = property.kernel_id();
            return self
                .kernel
                .as_ref()
                .ok_or_else(unavailable)?
                .set(set, id, value, flags);
        }
        // SAFETY: Interfaces belong to the current worker; value and mode were validated.
        let result = unsafe {
            match property {
                Property::ProcAmp(id) => self
                    .proc_amp
                    .as_ref()
                    .ok_or_else(unavailable)?
                    .Set(id, value, flags),
                Property::Camera(id) => self
                    .camera
                    .as_ref()
                    .ok_or_else(unavailable)?
                    .Set(id, value, flags),
            }
        };
        result.map_err(|error| windows_device_error("setting camera control", &error))
    }
}

fn unavailable() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "the Windows camera source does not expose this control",
    )
}

fn invalid_value() -> CameraError {
    CameraError::new(
        CameraErrorKind::InvalidConfiguration,
        "camera control value has the wrong type or is outside its range/step",
    )
}

#[cfg(test)]
mod tests {
    use super::{AUTO, MANUAL, PropertyRange};

    #[test]
    fn native_control_range_checks_values_without_i32_overflow() {
        let range = PropertyRange {
            minimum: i32::MIN,
            maximum: i32::MAX,
            step: 1,
            default: 0,
            modes: AUTO | MANUAL,
            initial_mode: MANUAL,
            writable: true,
        };
        assert!(range.is_valid());
        assert!(range.accepts(i64::from(i32::MAX)));
        assert!(range.accepts(i64::from(i32::MIN)));
        assert!(!range.accepts(i64::MAX));
        assert!(!range.accepts(i64::MIN));
    }

    #[test]
    fn invalid_driver_ranges_are_not_advertised() {
        let range = PropertyRange {
            minimum: -12,
            maximum: 6,
            step: 2,
            default: -4,
            modes: AUTO | MANUAL,
            initial_mode: AUTO,
            writable: true,
        };
        assert!(range.is_valid());
        assert!(!range.accepts(-3));
        assert!(!PropertyRange { step: 0, ..range }.is_valid());
        assert!(
            !PropertyRange {
                default: -3,
                ..range
            }
            .is_valid()
        );
        assert!(
            !PropertyRange {
                modes: MANUAL,
                ..range
            }
            .is_valid()
        );
        assert!(
            !PropertyRange {
                initial_mode: AUTO | MANUAL,
                ..range
            }
            .is_valid()
        );
    }
}
