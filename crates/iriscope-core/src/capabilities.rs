//! Camera capability models exposed consistently by every native backend.

use std::{cmp::Ordering, fmt};

/// Pixel layout or compressed format delivered by a camera.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum PixelFormat {
    /// A complete JPEG image for every frame.
    Mjpeg,
    /// Packed YUV 4:2:2 in YUYV/YUY2 byte order.
    Yuyv,
    /// Semi-planar YUV 4:2:0.
    Nv12,
    /// Eight-bit blue, green, red, and alpha channels.
    Bgra8,
    /// A backend format not yet represented by a shared variant.
    Other(String),
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mjpeg => formatter.write_str("MJPEG"),
            Self::Yuyv => formatter.write_str("YUYV"),
            Self::Nv12 => formatter.write_str("NV12"),
            Self::Bgra8 => formatter.write_str("BGRA8"),
            Self::Other(name) => formatter.write_str(name),
        }
    }
}

/// Width and height of a camera frame in pixels.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Resolution {
    /// Horizontal pixel count.
    pub width: u32,
    /// Vertical pixel count.
    pub height: u32,
}

impl Resolution {
    /// Creates a resolution.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Returns the total number of pixels in a frame.
    #[must_use]
    pub fn pixel_count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}×{}", self.width, self.height)
    }
}

/// Exact frame rate expressed as frames per second.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

impl FrameRate {
    /// Creates a frame rate from a non-zero rational value.
    #[must_use]
    pub const fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            None
        } else {
            let divisor = greatest_common_divisor(numerator, denominator);
            Some(Self {
                numerator: numerator / divisor,
                denominator: denominator / divisor,
            })
        }
    }

    /// Returns the fraction numerator.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Returns the fraction denominator.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    /// Returns the frame rate as a floating-point value for diagnostics.
    #[must_use]
    pub fn frames_per_second(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }
}

impl Ord for FrameRate {
    fn cmp(&self, other: &Self) -> Ordering {
        (u64::from(self.numerator) * u64::from(other.denominator))
            .cmp(&(u64::from(other.numerator) * u64::from(self.denominator)))
    }
}

impl PartialOrd for FrameRate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// A pixel format and resolution with all advertised frame rates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraMode {
    /// Format delivered by the camera.
    pub pixel_format: PixelFormat,
    /// Frame dimensions.
    pub resolution: Resolution,
    /// Frame rates advertised for this format and resolution.
    pub frame_rates: Vec<FrameRate>,
}

impl CameraMode {
    /// Reports whether this mode advertises the requested frame rate.
    #[must_use]
    pub fn supports_frame_rate(&self, frame_rate: FrameRate) -> bool {
        self.frame_rates.contains(&frame_rate)
    }
}

/// A camera control with a shared meaning across platforms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum StandardCameraControl {
    /// Image brightness.
    Brightness,
    /// Image contrast.
    Contrast,
    /// Color saturation.
    Saturation,
    /// Color hue.
    Hue,
    /// Automatic white balance toggle.
    WhiteBalanceAutomatic,
    /// Manual white balance value.
    WhiteBalanceManual,
    /// Gamma correction.
    Gamma,
    /// Power-line anti-flicker mode.
    PowerLineFrequency,
    /// Image sharpness.
    Sharpness,
    /// Automatic or manual exposure mode.
    ExposureMode,
    /// Manual exposure value.
    Exposure,
    /// Software focus when supported.
    Focus,
    /// Software illumination when supported.
    Illumination,
}

/// Stable identifier used to read or update a camera control.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CameraControlId {
    /// A control with shared semantics.
    Standard(StandardCameraControl),
    /// A native control that has no shared mapping yet.
    PlatformSpecific(String),
    /// A UVC extension unit selector.
    UvcExtension {
        /// Extension unit number.
        unit: u8,
        /// Control selector number.
        selector: u8,
        /// Extension unit GUID when the backend can provide it.
        guid: Option<String>,
    },
}

/// One selectable item exposed by a menu control.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraControlMenuItem {
    /// Native numeric value sent to the camera.
    pub value: i64,
    /// Human-readable value supplied by the backend.
    pub label: String,
}

/// Value constraints for a camera control.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CameraControlKind {
    /// A bounded integer value.
    Integer {
        /// Minimum accepted value.
        minimum: i64,
        /// Maximum accepted value.
        maximum: i64,
        /// Smallest supported increment.
        step: i64,
        /// Default value reported by the camera.
        default: i64,
        /// Unit reported by the API, if any.
        unit: Option<String>,
    },
    /// A boolean toggle.
    Boolean {
        /// Default value reported by the camera.
        default: bool,
    },
    /// A value selected from a finite set.
    Menu {
        /// Available values.
        items: Vec<CameraControlMenuItem>,
        /// Default native value reported by the camera.
        default: i64,
    },
}

/// Runtime value read from or written to a camera control.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CameraControlValue {
    /// Integer value.
    Integer(i64),
    /// Boolean value.
    Boolean(bool),
    /// Native value selected from a menu.
    Menu(i64),
}

/// Metadata required to present and edit a camera control.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraControlDescriptor {
    /// Backend-stable identifier.
    pub id: CameraControlId,
    /// Human-readable name supplied by the backend.
    pub name: String,
    /// Value type and limits reported by the camera.
    pub kind: CameraControlKind,
    /// Whether the backend reports this value as read-only.
    pub read_only: bool,
}

/// Complete capabilities discovered from one connected camera.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CameraCapabilities {
    /// Advertised streaming modes.
    pub modes: Vec<CameraMode>,
    /// Advertised image and device controls.
    pub controls: Vec<CameraControlDescriptor>,
}

impl CameraCapabilities {
    /// Finds an advertised format and resolution.
    #[must_use]
    pub fn find_mode(
        &self,
        pixel_format: &PixelFormat,
        resolution: Resolution,
    ) -> Option<&CameraMode> {
        self.modes
            .iter()
            .find(|mode| mode.pixel_format == *pixel_format && mode.resolution == resolution)
    }

    /// Selects the strongest advertised mode before runtime measurements are available.
    ///
    /// Resolution is ranked first, followed by advertised frame rate and a deterministic
    /// format preference. Runtime benchmarks may replace this initial choice later.
    #[must_use]
    pub fn preferred_mode(&self) -> Option<(&CameraMode, FrameRate)> {
        self.modes
            .iter()
            .filter_map(|mode| {
                mode.frame_rates
                    .iter()
                    .copied()
                    .max()
                    .map(|frame_rate| (mode, frame_rate))
            })
            .max_by_key(|(mode, frame_rate)| {
                (
                    mode.resolution.pixel_count(),
                    *frame_rate,
                    pixel_format_preference(&mode.pixel_format),
                )
            })
    }
}

const fn pixel_format_preference(pixel_format: &PixelFormat) -> u8 {
    match pixel_format {
        PixelFormat::Mjpeg => 4,
        PixelFormat::Yuyv => 3,
        PixelFormat::Bgra8 => 2,
        PixelFormat::Nv12 => 1,
        PixelFormat::Other(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{CameraCapabilities, CameraMode, FrameRate, PixelFormat, Resolution};

    #[test]
    fn frame_rate_preserves_fractional_values() {
        let frame_rate = FrameRate::new(25, 4).expect("25/4 is a valid frame rate");

        assert!((frame_rate.frames_per_second() - 6.25).abs() < f64::EPSILON);
        assert_eq!(frame_rate.numerator(), 25);
        assert_eq!(frame_rate.denominator(), 4);
        assert!(FrameRate::new(0, 1).is_none());
        assert!(FrameRate::new(1, 0).is_none());
        assert_eq!(
            FrameRate::new(16, 2),
            FrameRate::new(8, 1),
            "equivalent rates should normalize to the same value"
        );
    }

    #[test]
    fn capabilities_find_an_exact_mode() {
        let resolution = Resolution::new(1280, 1024);
        let frame_rate = FrameRate::new(8, 1).expect("8 fps is valid");
        let capabilities = CameraCapabilities {
            modes: vec![CameraMode {
                pixel_format: PixelFormat::Mjpeg,
                resolution,
                frame_rates: vec![frame_rate],
            }],
            controls: Vec::new(),
        };

        let mode = capabilities
            .find_mode(&PixelFormat::Mjpeg, resolution)
            .expect("the MJPEG mode should be present");

        assert!(mode.supports_frame_rate(frame_rate));
        assert_eq!(resolution.pixel_count(), 1_310_720);
    }

    #[test]
    fn preferred_mode_prioritizes_resolution_then_frame_rate() {
        let high_resolution = Resolution::new(1280, 1024);
        let capabilities = CameraCapabilities {
            modes: vec![
                CameraMode {
                    pixel_format: PixelFormat::Yuyv,
                    resolution: high_resolution,
                    frame_rates: vec![FrameRate::new(3, 1).expect("valid frame rate")],
                },
                CameraMode {
                    pixel_format: PixelFormat::Mjpeg,
                    resolution: high_resolution,
                    frame_rates: vec![FrameRate::new(8, 1).expect("valid frame rate")],
                },
                CameraMode {
                    pixel_format: PixelFormat::Mjpeg,
                    resolution: Resolution::new(640, 480),
                    frame_rates: vec![FrameRate::new(30, 1).expect("valid frame rate")],
                },
            ],
            controls: Vec::new(),
        };

        let (mode, frame_rate) = capabilities
            .preferred_mode()
            .expect("one mode should be preferred");

        assert_eq!(mode.resolution, high_resolution);
        assert_eq!(mode.pixel_format, PixelFormat::Mjpeg);
        assert_eq!(frame_rate, FrameRate::new(8, 1).expect("valid frame rate"));
    }
}
