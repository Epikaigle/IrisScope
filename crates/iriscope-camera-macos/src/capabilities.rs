use iriscope_core::capabilities::{CameraMode, FrameRate, PixelFormat};

pub(crate) const fn fourcc(code: [u8; 4]) -> u32 {
    ((code[0] as u32) << 24) | ((code[1] as u32) << 16) | ((code[2] as u32) << 8) | code[3] as u32
}

pub(crate) fn pixel_format_from_ostype(media_subtype: u32) -> PixelFormat {
    match media_subtype {
        value
            if value == fourcc(*b"jpeg")
                || value == fourcc(*b"dmb1")
                || value == fourcc(*b"MJPG")
                || value == fourcc(*b"mjpg") =>
        {
            PixelFormat::Mjpeg
        }
        value if value == fourcc(*b"yuvs") || value == fourcc(*b"YUY2") => PixelFormat::Yuyv,
        value
            if value == fourcc(*b"420v")
                || value == fourcc(*b"420f")
                || value == fourcc(*b"NV12") =>
        {
            PixelFormat::Nv12
        }
        value if value == fourcc(*b"BGRA") => PixelFormat::Bgra8,
        value => PixelFormat::Other(ostype_name(value)),
    }
}

pub(crate) fn frame_rate_from_duration_parts(
    duration_value: i64,
    duration_timescale: i32,
) -> Option<FrameRate> {
    let numerator = u32::try_from(duration_timescale).ok()?;
    let denominator = u32::try_from(duration_value).ok()?;
    FrameRate::new(numerator, denominator)
}

pub(crate) fn merge_mode(modes: &mut Vec<CameraMode>, mut candidate: CameraMode) {
    if let Some(existing) = modes.iter_mut().find(|mode| {
        mode.pixel_format == candidate.pixel_format && mode.resolution == candidate.resolution
    }) {
        existing.frame_rates.append(&mut candidate.frame_rates);
        sort_and_deduplicate_frame_rates(&mut existing.frame_rates);
    } else {
        sort_and_deduplicate_frame_rates(&mut candidate.frame_rates);
        modes.push(candidate);
    }
}

fn sort_and_deduplicate_frame_rates(frame_rates: &mut Vec<FrameRate>) {
    frame_rates.sort_by(|left, right| {
        left.frames_per_second()
            .total_cmp(&right.frames_per_second())
    });
    frame_rates.dedup();
}

fn ostype_name(value: u32) -> String {
    let bytes = value.to_be_bytes();
    if bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        format!("OSType 0x{value:08X}")
    }
}

#[cfg(test)]
mod tests {
    use iriscope_core::capabilities::{CameraMode, FrameRate, PixelFormat, Resolution};

    use super::{fourcc, frame_rate_from_duration_parts, merge_mode, pixel_format_from_ostype};

    #[test]
    fn maps_common_avfoundation_pixel_formats() {
        assert_eq!(
            pixel_format_from_ostype(fourcc(*b"dmb1")),
            PixelFormat::Mjpeg
        );
        assert_eq!(
            pixel_format_from_ostype(fourcc(*b"yuvs")),
            PixelFormat::Yuyv
        );
        assert_eq!(
            pixel_format_from_ostype(fourcc(*b"420v")),
            PixelFormat::Nv12
        );
        assert_eq!(
            pixel_format_from_ostype(fourcc(*b"420f")),
            PixelFormat::Nv12
        );
        assert_eq!(
            pixel_format_from_ostype(fourcc(*b"BGRA")),
            PixelFormat::Bgra8
        );
        assert_eq!(
            pixel_format_from_ostype(fourcc(*b"2vuy")),
            PixelFormat::Other("2vuy".to_owned())
        );
    }

    #[test]
    fn turns_exact_cm_time_duration_into_frame_rate() {
        let frame_rate = frame_rate_from_duration_parts(1_001, 30_000)
            .expect("a positive CMTime duration should produce a frame rate");

        assert_eq!(frame_rate.numerator(), 30_000);
        assert_eq!(frame_rate.denominator(), 1_001);
        assert!(frame_rate_from_duration_parts(0, 30_000).is_none());
        assert!(frame_rate_from_duration_parts(1, -30_000).is_none());
    }

    #[test]
    fn merges_duplicate_formats_and_range_boundaries() {
        let resolution = Resolution::new(1280, 1024);
        let mut modes = Vec::new();

        merge_mode(
            &mut modes,
            CameraMode {
                pixel_format: PixelFormat::Mjpeg,
                resolution,
                frame_rates: vec![
                    FrameRate::new(30, 1).expect("valid frame rate"),
                    FrameRate::new(5, 1).expect("valid frame rate"),
                ],
            },
        );
        merge_mode(
            &mut modes,
            CameraMode {
                pixel_format: PixelFormat::Mjpeg,
                resolution,
                frame_rates: vec![
                    FrameRate::new(30, 1).expect("valid frame rate"),
                    FrameRate::new(60, 1).expect("valid frame rate"),
                ],
            },
        );

        assert_eq!(modes.len(), 1);
        assert_eq!(
            modes[0].frame_rates,
            vec![
                FrameRate::new(5, 1).expect("valid frame rate"),
                FrameRate::new(30, 1).expect("valid frame rate"),
                FrameRate::new(60, 1).expect("valid frame rate"),
            ]
        );
    }
}
