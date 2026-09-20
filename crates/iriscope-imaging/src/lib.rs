//! Image decoding and processing for `IrisScope`.

use std::{error::Error, fmt};

use image::{ColorType, ImageEncoder, imageops::FilterType};
use zune_jpeg::{
    JpegDecoder,
    zune_core::{bytestream::ZCursor, colorspace::ColorSpace},
};

/// Error produced during image decoding or processing.
#[derive(Debug)]
pub enum ImagingError {
    /// JPEG decoding error.
    JpegDecode(String),
    /// Frame buffer size does not match expected resolution.
    InvalidBufferSize,
    /// Generic image decoding error.
    ImageDecode(String),
    /// Image encoding error.
    ImageEncode(String),
}

impl fmt::Display for ImagingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JpegDecode(msg) => write!(f, "JPEG decode error: {msg}"),
            Self::InvalidBufferSize => write!(f, "Invalid image buffer size"),
            Self::ImageDecode(msg) => write!(f, "Image decode error: {msg}"),
            Self::ImageEncode(msg) => write!(f, "Image encode error: {msg}"),
        }
    }
}

impl Error for ImagingError {}

// Standard JPEG Annex K default Huffman tables (Luminance DC, Chrominance DC, Luminance AC, Chrominance AC).
// Many UVC cameras omit these tables in MJPEG mode to reduce frame payload size.
const DEFAULT_DHT: &[u8] = &[
    0xFF, 0xC4, 0x01, 0xA2, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A,
    0x0B, 0x01, 0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x10, 0x00,
    0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01,
    0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22,
    0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24,
    0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29,
    0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A,
    0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A,
    0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A,
    0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8,
    0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6,
    0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3,
    0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9,
    0xFA, 0x11, 0x00, 0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01,
    0x02, 0x77, 0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07,
    0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33,
    0x52, 0xF0, 0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19,
    0x1A, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46,
    0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66,
    0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85,
    0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3,
    0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA,
    0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8,
    0xD9, 0xDA, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6,
    0xF7, 0xF8, 0xF9, 0xFA,
];

/// Ensures a JPEG frame has standard Annex K Huffman tables (DHT).
///
/// Many UVC cameras omit DHT markers to reduce transmission overhead.
/// Standalone JPEG decoders and photo viewers require DHT to be present.
#[must_use]
pub fn ensure_jpeg_has_dht(jpeg_data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if jpeg_data.windows(2).any(|w| w == [0xFF, 0xC4]) {
        std::borrow::Cow::Borrowed(jpeg_data)
    } else if let Some(pos) = jpeg_data.windows(2).position(|w| w == [0xFF, 0xDA]) {
        let mut buf = Vec::with_capacity(jpeg_data.len() + DEFAULT_DHT.len());
        buf.extend_from_slice(&jpeg_data[..pos]);
        buf.extend_from_slice(DEFAULT_DHT);
        buf.extend_from_slice(&jpeg_data[pos..]);
        std::borrow::Cow::Owned(buf)
    } else {
        std::borrow::Cow::Borrowed(jpeg_data)
    }
}

/// Decodes raw MJPEG bytes into an RGB8 byte buffer using SIMD acceleration.
///
/// Returns `(width, height, rgb_bytes)`.
///
/// # Errors
///
/// Returns an [`ImagingError::JpegDecode`] if the JPEG bitstream is invalid or corrupted.
#[allow(clippy::match_same_arms)]
pub fn decode_mjpeg_to_rgb8(jpeg_data: &[u8]) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    let patched_data = ensure_jpeg_has_dht(jpeg_data);

    let mut decoder = JpegDecoder::new(ZCursor::new(patched_data.as_ref()));
    let pixels = decoder
        .decode()
        .map_err(|err| ImagingError::JpegDecode(format!("{err:?}")))?;

    let info = decoder
        .info()
        .ok_or_else(|| ImagingError::JpegDecode("missing JPEG header metadata".to_string()))?;

    let width = u32::from(info.width);
    let height = u32::from(info.height);

    // Ensure output is RGB8
    let rgb_bytes = match decoder.output_colorspace() {
        Some(ColorSpace::RGB) | None => pixels,
        Some(ColorSpace::BGR) => {
            let mut rgb = pixels;
            for chunk in rgb.chunks_exact_mut(3) {
                chunk.swap(0, 2);
            }
            rgb
        }
        Some(ColorSpace::RGBA) => {
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for chunk in pixels.chunks_exact(4) {
                rgb.extend_from_slice(&chunk[..3]);
            }
            rgb
        }
        _ => pixels,
    };

    Ok((width, height, rgb_bytes))
}

/// Decodes a supported still-image file (JPEG or PNG) into RGB8.
///
/// # Errors
///
/// Returns `ImagingError::ImageDecode` when the input cannot be decoded.
pub fn decode_image_to_rgb8(image_data: &[u8]) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    let decoded = image::load_from_memory(image_data)
        .map_err(|error| ImagingError::ImageDecode(error.to_string()))?;
    let rgb = decoded.to_rgb8();
    let width = rgb.width();
    let height = rgb.height();
    Ok((width, height, rgb.into_raw()))
}

/// Resizes an RGB8 image to fit inside a square thumbnail while preserving aspect ratio.
///
/// # Errors
///
/// Returns `ImagingError::InvalidBufferSize` when the RGB buffer does not match
/// the supplied dimensions.
pub fn resize_rgb8_to_fit(
    rgb: &[u8],
    width: u32,
    height: u32,
    max_dimension: u32,
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    let image = image::RgbImage::from_raw(width, height, rgb.to_vec())
        .ok_or(ImagingError::InvalidBufferSize)?;
    let resized = image::DynamicImage::ImageRgb8(image)
        .resize(max_dimension, max_dimension, FilterType::Triangle)
        .to_rgb8();
    let out_width = resized.width();
    let out_height = resized.height();
    Ok((out_width, out_height, resized.into_raw()))
}

/// Converts packed BGRA8 bytes into packed RGB8 bytes.
///
/// # Errors
///
/// Returns `ImagingError::InvalidBufferSize` when the source buffer is smaller
/// than the dimensions require or the calculated frame size overflows.
pub fn convert_bgra8_to_rgb8(
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ImagingError> {
    let pixel_count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| ImagingError::InvalidBufferSize)?;
    let expected = pixel_count
        .checked_mul(4)
        .ok_or(ImagingError::InvalidBufferSize)?;
    if bgra.len() < expected {
        return Err(ImagingError::InvalidBufferSize);
    }

    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for pixel in bgra[..expected].chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    Ok(rgb)
}

/// Encodes an RGB8 frame as a lossless PNG.
///
/// # Errors
///
/// Returns an imaging error when the RGB buffer size is invalid or encoding fails.
pub fn encode_rgb8_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImagingError> {
    let expected = usize::try_from(u64::from(width) * u64::from(height) * 3)
        .map_err(|_| ImagingError::InvalidBufferSize)?;
    if rgb.len() != expected {
        return Err(ImagingError::InvalidBufferSize);
    }

    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(rgb, width, height, ColorType::Rgb8.into())
        .map_err(|error| ImagingError::ImageEncode(format!("{error:?}")))?;
    Ok(encoded)
}

/// Converts packed YUYV (YUY2) 4:2:2 bytes into packed RGB8 bytes.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn convert_yuyv_to_rgb8(yuyv: &[u8], width: u32, height: u32) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut rgb = vec![0_u8; pixel_count * 3];

    let macropixel_count = pixel_count / 2;
    for i in 0..macropixel_count {
        let src_idx = i * 4;
        if src_idx + 3 >= yuyv.len() {
            break;
        }

        let y0 = f32::from(yuyv[src_idx]);
        let u = f32::from(yuyv[src_idx + 1]) - 128.0;
        let y1 = f32::from(yuyv[src_idx + 2]);
        let v = f32::from(yuyv[src_idx + 3]) - 128.0;

        // Pixel 0
        let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g0 = (y0 - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 255.0) as u8;
        let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

        // Pixel 1
        let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g1 = (y1 - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 255.0) as u8;
        let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

        let dst0 = i * 6;
        if dst0 + 5 < rgb.len() {
            rgb[dst0] = r0;
            rgb[dst0 + 1] = g0;
            rgb[dst0 + 2] = b0;
            rgb[dst0 + 3] = r1;
            rgb[dst0 + 4] = g1;
            rgb[dst0 + 5] = b1;
        }
    }

    rgb
}

/// Applies display-only transformations (rotation, horizontal and vertical flips).
#[must_use]
pub fn apply_transforms(
    pixels: &[u8],
    width: u32,
    height: u32,
    rotation_degrees: u32,
    mirror_h: bool,
    mirror_v: bool,
) -> (u32, u32, Vec<u8>) {
    if rotation_degrees == 0 && !mirror_h && !mirror_v {
        return (width, height, pixels.to_vec());
    }

    let Some(image_buffer) = image::RgbImage::from_raw(width, height, pixels.to_vec()) else {
        return (width, height, pixels.to_vec());
    };

    let mut transformed = match rotation_degrees % 360 {
        90 => image::imageops::rotate90(&image_buffer),
        180 => image::imageops::rotate180(&image_buffer),
        270 => image::imageops::rotate270(&image_buffer),
        _ => image_buffer,
    };

    if mirror_h {
        transformed = image::imageops::flip_horizontal(&transformed);
    }
    if mirror_v {
        transformed = image::imageops::flip_vertical(&transformed);
    }

    let out_w = transformed.width();
    let out_h = transformed.height();
    (out_w, out_h, transformed.into_raw())
}

/// Generates a JPEG thumbnail scaled to fit within `max_dimension`.
///
/// # Errors
///
/// Returns an [`ImagingError`] if the input buffer cannot be decoded or re-encoded.
pub fn create_thumbnail_jpeg(
    rgb: &[u8],
    width: u32,
    height: u32,
    max_dimension: u32,
) -> Result<Vec<u8>, ImagingError> {
    let img = image::RgbImage::from_raw(width, height, rgb.to_vec())
        .ok_or(ImagingError::InvalidBufferSize)?;

    let dynamic = image::DynamicImage::ImageRgb8(img);
    let resized = dynamic.resize(max_dimension, max_dimension, FilterType::Triangle);

    let mut jpeg_bytes = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 80);
    encoder
        .write_image(
            resized.as_bytes(),
            resized.width(),
            resized.height(),
            ColorType::Rgb8.into(),
        )
        .map_err(|err| ImagingError::ImageEncode(format!("{err:?}")))?;

    Ok(jpeg_bytes)
}

#[cfg(test)]
mod tests {
    use super::{apply_transforms, convert_yuyv_to_rgb8};

    #[test]
    fn converts_yuyv_black_and_white() {
        // Two pixels: black (Y=0, U=128, Y=0, V=128)
        let yuyv = [0, 128, 0, 128];
        let rgb = convert_yuyv_to_rgb8(&yuyv, 2, 1);
        assert_eq!(rgb.len(), 6);
        assert_eq!(rgb, [0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn transforms_preserve_dimensions_on_180() {
        let pixels = vec![0_u8; 100 * 50 * 3];
        let (w, h, out) = apply_transforms(&pixels, 100, 50, 180, false, false);
        assert_eq!(w, 100);
        assert_eq!(h, 50);
        assert_eq!(out.len(), pixels.len());
    }

    #[test]
    fn transforms_swap_dimensions_on_90() {
        let pixels = vec![0_u8; 100 * 50 * 3];
        let (w, h, out) = apply_transforms(&pixels, 100, 50, 90, false, false);
        assert_eq!(w, 50);
        assert_eq!(h, 100);
        assert_eq!(out.len(), pixels.len());
    }
}
