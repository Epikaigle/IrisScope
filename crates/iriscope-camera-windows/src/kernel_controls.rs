//! Standard camera properties routed through the Media Foundation source.
//!
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source>
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ksproxy/nf-ksproxy-ikscontrol-ksproperty>

use std::{mem::size_of, ptr};

use iriscope_core::camera::{CameraError, CameraErrorKind, CameraResult};
use windows::{
    Win32::Media::{
        KernelStreaming::{
            IKsControl, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0, KSPROPERTY_BOUNDS_LONG,
            KSPROPERTY_DESCRIPTION, KSPROPERTY_MEMBER_FLAG_DEFAULT, KSPROPERTY_MEMBER_RANGES,
            KSPROPERTY_MEMBER_STEPPEDRANGES, KSPROPERTY_MEMBER_VALUES, KSPROPERTY_MEMBERSHEADER,
            KSPROPERTY_STEPPING_LONG, KSPROPERTY_TYPE_BASICSUPPORT, KSPROPERTY_TYPE_GET,
            KSPROPERTY_TYPE_SET, KSPROPERTY_VIDEOPROCAMP_S, KSPROPTYPESETID_General,
        },
        MediaFoundation::IMFMediaSource,
    },
    core::{GUID, Interface},
};

use crate::platform::windows_device_error;

const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;

pub(super) struct KernelControls(IKsControl);

#[derive(Clone, Copy, Debug)]
pub(super) struct KernelRange {
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub default: Option<i32>,
    pub writable: bool,
}

impl KernelControls {
    pub(super) fn discover(source: &IMFMediaSource) -> Option<Self> {
        source.cast().ok().map(Self)
    }

    pub(super) fn range(&self, set: GUID, id: u32) -> CameraResult<KernelRange> {
        let request = identifier(set, id, KSPROPERTY_TYPE_BASICSUPPORT);
        let mut description = KSPROPERTY_DESCRIPTION::default();
        let mut returned = 0;
        // SAFETY: The COM interface belongs to the camera worker and all buffers
        // have their advertised lengths. A header-sized BASICSUPPORT query
        // reports DescriptionSize without allocating any driver-chosen size.
        unsafe {
            self.0.KsProperty(
                &raw const request,
                size_u32::<KSIDENTIFIER>(),
                (&raw mut description).cast(),
                size_u32::<KSPROPERTY_DESCRIPTION>(),
                &raw mut returned,
            )
        }
        .map_err(|error| {
            windows_device_error("reading kernel camera control description", &error)
        })?;
        if returned < size_u32::<KSPROPERTY_DESCRIPTION>() {
            return Err(invalid_description());
        }
        let length =
            usize::try_from(description.DescriptionSize).map_err(|_| invalid_description())?;
        if !(size_of::<KSPROPERTY_DESCRIPTION>()..=MAX_DESCRIPTION_BYTES).contains(&length) {
            return Err(invalid_description());
        }
        let mut bytes = vec![0_u8; length];
        returned = 0;
        // SAFETY: The allocated output is exactly the validated description size.
        unsafe {
            self.0.KsProperty(
                &raw const request,
                size_u32::<KSIDENTIFIER>(),
                bytes.as_mut_ptr().cast(),
                description.DescriptionSize,
                &raw mut returned,
            )
        }
        .map_err(|error| windows_device_error("reading kernel camera control range", &error))?;
        let returned = usize::try_from(returned).map_err(|_| invalid_description())?;
        if returned > bytes.len() {
            return Err(invalid_description());
        }
        parse_range(&bytes[..returned]).ok_or_else(invalid_description)
    }

    pub(super) fn get(&self, set: GUID, id: u32) -> CameraResult<(i32, i32, i32)> {
        let request = KSPROPERTY_VIDEOPROCAMP_S {
            Property: identifier(set, id, KSPROPERTY_TYPE_GET),
            ..KSPROPERTY_VIDEOPROCAMP_S::default()
        };
        let mut output = request;
        let mut returned = 0;
        // SAFETY: Both legacy camera property sets use this header/value/flags/
        // capabilities layout. Input and output remain separate valid buffers.
        unsafe {
            self.0.KsProperty(
                &raw const request.Property,
                size_u32::<KSPROPERTY_VIDEOPROCAMP_S>(),
                (&raw mut output).cast(),
                size_u32::<KSPROPERTY_VIDEOPROCAMP_S>(),
                &raw mut returned,
            )
        }
        .map_err(|error| windows_device_error("reading kernel camera control", &error))?;
        if returned < size_u32::<KSPROPERTY_VIDEOPROCAMP_S>() {
            return Err(invalid_description());
        }
        Ok((
            output.Value,
            output.Flags.cast_signed(),
            output.Capabilities.cast_signed(),
        ))
    }

    pub(super) fn set(&self, set: GUID, id: u32, value: i32, flags: i32) -> CameraResult<()> {
        let request = KSPROPERTY_VIDEOPROCAMP_S {
            Property: identifier(set, id, KSPROPERTY_TYPE_SET),
            Value: value,
            Flags: flags.cast_unsigned(),
            Capabilities: 0,
        };
        let mut output = request;
        let mut returned = 0;
        // SAFETY: The caller validated the value and mode against BASICSUPPORT
        // and capabilities. Buffers have the exact native property layout.
        unsafe {
            self.0.KsProperty(
                &raw const request.Property,
                size_u32::<KSPROPERTY_VIDEOPROCAMP_S>(),
                (&raw mut output).cast(),
                size_u32::<KSPROPERTY_VIDEOPROCAMP_S>(),
                &raw mut returned,
            )
        }
        .map_err(|error| windows_device_error("setting kernel camera control", &error))
    }
}

fn identifier(set: GUID, id: u32, flags: u32) -> KSIDENTIFIER {
    KSIDENTIFIER {
        Anonymous: KSIDENTIFIER_0 {
            Anonymous: KSIDENTIFIER_0_0 {
                Set: set,
                Id: id,
                Flags: flags,
            },
        },
    }
}

fn size_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("fixed Windows property structures fit in u32")
}

fn invalid_description() -> CameraError {
    CameraError::new(
        CameraErrorKind::Unsupported,
        "camera driver returned an unsupported or invalid control description",
    )
}

// All types read here are C-layout scalar/GUID POD structures with no pointers
// or invalid bit patterns. Check every byte boundary before unaligned reads.
fn read_pod<T: Copy>(bytes: &[u8], offset: usize) -> Option<T> {
    let end = offset.checked_add(size_of::<T>())?;
    let bytes = bytes.get(offset..end)?;
    // SAFETY: Bounds checked above. Caller uses only native scalar POD types.
    Some(unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}

fn parse_range(bytes: &[u8]) -> Option<KernelRange> {
    let description = read_pod::<KSPROPERTY_DESCRIPTION>(bytes, 0)?;
    // SAFETY: The property description uses the identifier's Set/Id/Flags layout.
    let value_type = unsafe { description.PropTypeSet.Anonymous.Anonymous };
    // Standard legacy camera values are signed LONG (VT_I4 = 3). Do not
    // reinterpret unsigned or 64-bit range descriptors as signed LONG.
    if value_type.Set != KSPROPTYPESETID_General || value_type.Id != 3 {
        return None;
    }
    let total = usize::try_from(description.DescriptionSize).ok()?;
    if total != bytes.len() || description.AccessFlags & KSPROPERTY_TYPE_GET == 0 {
        return None;
    }
    let mut offset = size_of::<KSPROPERTY_DESCRIPTION>();
    let mut bounds = None;
    let mut default = None;
    // Bound the loop by the bytes available even if MembersListCount is corrupt.
    if usize::try_from(description.MembersListCount).ok()?
        > bytes.len() / size_of::<KSPROPERTY_MEMBERSHEADER>()
    {
        return None;
    }
    for _ in 0..description.MembersListCount {
        let header = read_pod::<KSPROPERTY_MEMBERSHEADER>(bytes, offset)?;
        offset = offset.checked_add(size_of::<KSPROPERTY_MEMBERSHEADER>())?;
        let member_size = usize::try_from(header.MembersSize).ok()?;
        let member_count = usize::try_from(header.MembersCount).ok()?;
        let list_size = member_size.checked_mul(member_count)?;
        let end = offset.checked_add(list_size)?;
        let members = bytes.get(offset..end)?;
        match header.MembersFlags {
            KSPROPERTY_MEMBER_STEPPEDRANGES
                if member_size == size_of::<KSPROPERTY_STEPPING_LONG>() && member_count == 1 =>
            {
                if bounds.is_some() {
                    return None;
                }
                let range = read_pod::<KSPROPERTY_STEPPING_LONG>(members, 0)?;
                // SAFETY: Camera properties carry signed LONG values.
                let signed = unsafe { range.Bounds.Anonymous1 };
                bounds = Some((
                    signed.SignedMinimum,
                    signed.SignedMaximum,
                    i32::try_from(range.SteppingDelta).ok()?,
                ));
            }
            KSPROPERTY_MEMBER_RANGES
                if member_size == size_of::<KSPROPERTY_BOUNDS_LONG>() && member_count == 1 =>
            {
                if bounds.is_some() {
                    return None;
                }
                let range = read_pod::<KSPROPERTY_BOUNDS_LONG>(members, 0)?;
                // SAFETY: Camera properties carry signed LONG values.
                let signed = unsafe { range.Anonymous1 };
                bounds = Some((signed.SignedMinimum, signed.SignedMaximum, 1));
            }
            KSPROPERTY_MEMBER_VALUES
                if header.Flags & KSPROPERTY_MEMBER_FLAG_DEFAULT != 0
                    && member_size == size_of::<i32>()
                    && member_count == 1 =>
            {
                if default.is_some() {
                    return None;
                }
                default = Some(read_pod::<i32>(members, 0)?);
            }
            KSPROPERTY_MEMBER_STEPPEDRANGES | KSPROPERTY_MEMBER_RANGES => return None,
            _ => {}
        }
        offset = end;
    }
    if offset != bytes.len() {
        return None;
    }
    let (minimum, maximum, step) = bounds?;
    (minimum <= maximum && step > 0).then_some(KernelRange {
        minimum,
        maximum,
        step,
        default,
        writable: description.AccessFlags & KSPROPERTY_TYPE_SET != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::KernelStreaming::KSPROPERTY_BOUNDS_LONG_0;

    fn append_pod<T: Copy>(bytes: &mut Vec<u8>, value: &T) {
        // SAFETY: Tests only use initialized C-layout scalar POD fixtures.
        let value = unsafe {
            std::slice::from_raw_parts(ptr::from_ref(value).cast::<u8>(), size_of::<T>())
        };
        bytes.extend_from_slice(value);
    }

    fn description_fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        append_pod(
            &mut bytes,
            &KSPROPERTY_DESCRIPTION {
                AccessFlags: KSPROPERTY_TYPE_GET | KSPROPERTY_TYPE_SET,
                DescriptionSize: u32::try_from(
                    size_of::<KSPROPERTY_DESCRIPTION>()
                        + 2 * size_of::<KSPROPERTY_MEMBERSHEADER>()
                        + size_of::<KSPROPERTY_STEPPING_LONG>()
                        + size_of::<i32>(),
                )
                .unwrap(),
                MembersListCount: 2,
                PropTypeSet: identifier(KSPROPTYPESETID_General, 3, 0),
                ..KSPROPERTY_DESCRIPTION::default()
            },
        );
        append_pod(
            &mut bytes,
            &KSPROPERTY_MEMBERSHEADER {
                MembersFlags: KSPROPERTY_MEMBER_STEPPEDRANGES,
                MembersSize: size_u32::<KSPROPERTY_STEPPING_LONG>(),
                MembersCount: 1,
                Flags: 0,
            },
        );
        let mut stepped = KSPROPERTY_STEPPING_LONG {
            SteppingDelta: 2,
            ..Default::default()
        };
        stepped.Bounds.Anonymous1 = KSPROPERTY_BOUNDS_LONG_0 {
            SignedMinimum: -12,
            SignedMaximum: 6,
        };
        append_pod(&mut bytes, &stepped);
        append_pod(
            &mut bytes,
            &KSPROPERTY_MEMBERSHEADER {
                MembersFlags: KSPROPERTY_MEMBER_VALUES,
                MembersSize: size_u32::<i32>(),
                MembersCount: 1,
                Flags: KSPROPERTY_MEMBER_FLAG_DEFAULT,
            },
        );
        append_pod(&mut bytes, &-4_i32);
        bytes
    }

    #[test]
    fn reads_signed_range_steps_and_reported_default() {
        let range = parse_range(&description_fixture()).expect("valid native description");
        assert_eq!(
            (range.minimum, range.maximum, range.step, range.default),
            (-12, 6, 2, Some(-4))
        );
        assert!(range.writable);
    }

    #[test]
    fn malformed_driver_description_is_rejected() {
        let bytes = description_fixture();
        for end in 0..bytes.len() {
            assert!(parse_range(&bytes[..end]).is_none());
        }
        let mut invalid = bytes.clone();
        invalid[32..36].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(parse_range(&invalid).is_none());
        let mut invalid = bytes;
        let member_size_offset = size_of::<KSPROPERTY_DESCRIPTION>() + 4;
        invalid[member_size_offset..member_size_offset + 4]
            .copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(parse_range(&invalid).is_none());
    }
}
