//! Image decoding and processing for `IrisScope`.

mod rotation;
pub use rotation::{RotationGeometry, rotate_rgb};

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

const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_ENCODED_IMAGE_BYTES: usize = 32 * 1024 * 1024;

fn image_pixel_count(width: u32, height: u32) -> Result<usize, ImagingError> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(ImagingError::InvalidBufferSize);
    }
    usize::try_from(pixels).map_err(|_| ImagingError::InvalidBufferSize)
}

fn validate_rgb8(rgb: &[u8], width: u32, height: u32) -> Result<(), ImagingError> {
    if image_pixel_count(width, height)?.checked_mul(3) != Some(rgb.len()) {
        return Err(ImagingError::InvalidBufferSize);
    }
    Ok(())
}

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
    if !jpeg_data.starts_with(&[0xFF, 0xD8]) {
        return std::borrow::Cow::Borrowed(jpeg_data);
    }

    // Marker-looking bytes inside APP/COM payloads are ordinary data. Walk the
    // length-delimited header segments and stop before entropy-coded scan data.
    let mut offset = 2;
    while offset < jpeg_data.len() {
        let marker_start = offset;
        if jpeg_data[offset] != 0xFF {
            break;
        }
        while jpeg_data.get(offset) == Some(&0xFF) {
            offset += 1;
        }
        let Some(&marker) = jpeg_data.get(offset) else {
            break;
        };
        offset += 1;

        match marker {
            0xC4 => return std::borrow::Cow::Borrowed(jpeg_data),
            0x00 | 0xD8 | 0xD9 => break,
            0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }

        let Some(length_bytes) = jpeg_data.get(offset..).and_then(|tail| tail.get(..2)) else {
            break;
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        let Some(end) = offset.checked_add(length) else {
            break;
        };
        if length < 2 || end > jpeg_data.len() {
            break;
        }
        if marker == 0xDA {
            let mut buf = Vec::with_capacity(jpeg_data.len() + DEFAULT_DHT.len());
            buf.extend_from_slice(&jpeg_data[..marker_start]);
            buf.extend_from_slice(DEFAULT_DHT);
            buf.extend_from_slice(&jpeg_data[marker_start..]);
            return std::borrow::Cow::Owned(buf);
        }
        offset = end;
    }

    std::borrow::Cow::Borrowed(jpeg_data)
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
    if jpeg_data.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(ImagingError::JpegDecode(
            "JPEG input exceeds the 32 MiB limit".to_string(),
        ));
    }
    let patched_data = ensure_jpeg_has_dht(jpeg_data);

    let mut decoder = JpegDecoder::new(ZCursor::new(patched_data.as_ref()));
    decoder
        .decode_headers()
        .map_err(|err| ImagingError::JpegDecode(format!("{err:?}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| ImagingError::JpegDecode("missing JPEG header metadata".to_string()))?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    if image_pixel_count(width, height).is_err() {
        return Err(ImagingError::JpegDecode(
            "JPEG dimensions exceed the 8192-pixel or 16 megapixel limit".to_string(),
        ));
    }
    let pixels = decoder
        .decode()
        .map_err(|err| ImagingError::JpegDecode(format!("{err:?}")))?;

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
    decode_reference_image_to_rgb8(image_data)
}

/// Decodes a reference image while bounding its dimensions and decoder allocation.
///
/// # Errors
///
/// Returns `ImagingError::ImageDecode` for unsupported, damaged or oversized images.
pub fn decode_reference_image_to_rgb8(
    image_data: &[u8],
) -> Result<(u32, u32, Vec<u8>), ImagingError> {
    use std::io::Cursor;

    if image_data.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(ImagingError::ImageDecode(
            "image input exceeds the 32 MiB limit".to_string(),
        ));
    }
    let reader = image::ImageReader::new(Cursor::new(image_data))
        .with_guessed_format()
        .map_err(|error| ImagingError::ImageDecode(error.to_string()))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| ImagingError::ImageDecode(error.to_string()))?;
    if image_pixel_count(width, height).is_err() {
        return Err(ImagingError::ImageDecode(
            "image dimensions exceed the 8192-pixel or 16 megapixel limit".to_string(),
        ));
    }
    let mut reader = image::ImageReader::new(Cursor::new(image_data))
        .with_guessed_format()
        .map_err(|error| ImagingError::ImageDecode(error.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let rgb = reader
        .decode()
        .map_err(|error| ImagingError::ImageDecode(error.to_string()))?
        .to_rgb8();
    Ok((rgb.width(), rgb.height(), rgb.into_raw()))
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
    validate_rgb8(rgb, width, height)?;
    if max_dimension == 0 || max_dimension > MAX_IMAGE_DIMENSION {
        return Err(ImagingError::InvalidBufferSize);
    }
    let longest_side = u64::from(width.max(height));
    let fitted_side = |side: u32| {
        u32::try_from(
            (u64::from(side) * u64::from(max_dimension) + longest_side / 2) / longest_side,
        )
        .unwrap_or(max_dimension)
        .max(1)
    };
    let out_width = fitted_side(width);
    let out_height = fitted_side(height);
    image_pixel_count(out_width, out_height)?;
    // Borrow the full-resolution source: only the resized result is allocated.
    let image = image::ImageBuffer::<image::Rgb<u8>, &[u8]>::from_raw(width, height, rgb)
        .ok_or(ImagingError::InvalidBufferSize)?;
    let resized = image::imageops::resize(&image, out_width, out_height, FilterType::Triangle);
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
    let pixel_count = image_pixel_count(width, height)?;
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

/// Converts compact NV12 bytes into packed RGB8 bytes.
///
/// The source must contain a tightly packed full-resolution Y plane followed by
/// a tightly packed half-resolution plane of interleaved U and V samples. NV12
/// uses 2x2 chroma subsampling, so both dimensions must be non-zero and even.
///
/// # Errors
///
/// Returns [`ImagingError::InvalidBufferSize`] when a dimension is zero or odd,
/// when the expected frame size overflows, or when the source buffer does not
/// exactly match the supplied dimensions.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn convert_nv12_to_rgb8(nv12: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImagingError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(ImagingError::InvalidBufferSize);
    }

    let pixel_count = image_pixel_count(width, height)?;
    let expected = pixel_count
        .checked_add(pixel_count / 2)
        .ok_or(ImagingError::InvalidBufferSize)?;
    if nv12.len() != expected {
        return Err(ImagingError::InvalidBufferSize);
    }

    let width = usize::try_from(width).map_err(|_| ImagingError::InvalidBufferSize)?;
    let height = usize::try_from(height).map_err(|_| ImagingError::InvalidBufferSize)?;
    let mut rgb = Vec::with_capacity(
        pixel_count
            .checked_mul(3)
            .ok_or(ImagingError::InvalidBufferSize)?,
    );

    for row in 0..height {
        let y_row = row * width;
        let uv_row = pixel_count + (row / 2) * width;
        for column in 0..width {
            let y = f32::from(nv12[y_row + column]);
            let uv_index = uv_row + (column / 2) * 2;
            let u = f32::from(nv12[uv_index]) - 128.0;
            let v = f32::from(nv12[uv_index + 1]) - 128.0;

            rgb.extend_from_slice(&[
                (y + 1.402 * v).clamp(0.0, 255.0) as u8,
                (y - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 255.0) as u8,
                (y + 1.772 * u).clamp(0.0, 255.0) as u8,
            ]);
        }
    }

    Ok(rgb)
}

/// Encodes an RGB8 frame as JPEG.
///
/// This is used only when a camera backend does not provide native MJPEG frames,
/// allowing the same MJPEG AVI container to be used cross-platform.
///
/// # Errors
///
/// Returns an imaging error when the RGB buffer size is invalid or encoding fails.
pub fn encode_rgb8_jpeg(
    rgb: &[u8],
    width: u32,
    height: u32,
    quality: u8,
) -> Result<Vec<u8>, ImagingError> {
    validate_rgb8(rgb, width, height)?;

    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality.clamp(1, 100))
        .write_image(rgb, width, height, ColorType::Rgb8.into())
        .map_err(|error| ImagingError::ImageEncode(format!("{error:?}")))?;
    Ok(encoded)
}

/// Encodes an RGB8 frame as a lossless PNG.
///
/// # Errors
///
/// Returns an imaging error when the RGB buffer size is invalid or encoding fails.
pub fn encode_rgb8_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ImagingError> {
    validate_rgb8(rgb, width, height)?;

    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(rgb, width, height, ColorType::Rgb8.into())
        .map_err(|error| ImagingError::ImageEncode(format!("{error:?}")))?;
    Ok(encoded)
}

/// Converts tightly packed YUYV (YUY2) 4:2:2 bytes into packed RGB8 bytes.
///
/// Returns an empty buffer for zero or odd dimensions, an incomplete frame, or
/// dimensions too large to fit in memory. Trailing source bytes are ignored.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn convert_yuyv_to_rgb8(yuyv: &[u8], width: u32, height: u32) -> Vec<u8> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) {
        return Vec::new();
    }
    let Some((source_length, output_length)) = image_pixel_count(width, height)
        .ok()
        .and_then(|pixels| Some((pixels.checked_mul(2)?, pixels.checked_mul(3)?)))
    else {
        return Vec::new();
    };
    let Some(source) = yuyv.get(..source_length) else {
        return Vec::new();
    };

    let mut rgb = vec![0_u8; output_length];
    for (macropixel, destination) in source.chunks_exact(4).zip(rgb.chunks_exact_mut(6)) {
        let y0 = f32::from(macropixel[0]);
        let u = f32::from(macropixel[1]) - 128.0;
        let y1 = f32::from(macropixel[2]);
        let v = f32::from(macropixel[3]) - 128.0;

        // Pixel 0
        let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g0 = (y0 - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 255.0) as u8;
        let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

        // Pixel 1
        let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g1 = (y1 - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 255.0) as u8;
        let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

        destination.copy_from_slice(&[r0, g0, b0, r1, g1, b1]);
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

    let mut transformed = rotate_rgb(
        image_buffer,
        i32::try_from(rotation_degrees % 360).unwrap_or(0),
    );

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
    let (width, height, resized) = resize_rgb8_to_fit(rgb, width, height, max_dimension)?;
    encode_rgb8_jpeg(&resized, width, height, 80)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{
        ImagingError, apply_transforms, convert_bgra8_to_rgb8, convert_nv12_to_rgb8,
        convert_yuyv_to_rgb8, create_thumbnail_jpeg, decode_image_to_rgb8, decode_mjpeg_to_rgb8,
        decode_reference_image_to_rgb8, encode_rgb8_jpeg, ensure_jpeg_has_dht, resize_rgb8_to_fit,
    };

    fn jpeg_without_dht(jpeg: &[u8]) -> Vec<u8> {
        let mut stripped = jpeg[..2].to_vec();
        let mut offset = 2;
        while offset < jpeg.len() {
            let start = offset;
            assert_eq!(jpeg[offset], 0xFF);
            let marker = jpeg[offset + 1];
            offset += 2;
            let length = usize::from(u16::from_be_bytes([jpeg[offset], jpeg[offset + 1]]));
            let end = offset + length;
            if marker != 0xC4 {
                stripped.extend_from_slice(&jpeg[start..end]);
            }
            offset = end;
            if marker == 0xDA {
                stripped.extend_from_slice(&jpeg[offset..]);
                break;
            }
        }
        stripped
    }

    #[test]
    fn adds_default_dht_to_synthetic_mjpeg_and_decodes_it() {
        let pixels = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let jpeg = encode_rgb8_jpeg(&pixels, 2, 2, 95).expect("encode synthetic JPEG");
        let stripped = jpeg_without_dht(&jpeg);
        assert!(stripped.len() < jpeg.len());

        let patched = ensure_jpeg_has_dht(&stripped);
        assert!(matches!(patched, Cow::Owned(_)));
        assert_eq!(
            decode_mjpeg_to_rgb8(&stripped).unwrap(),
            decode_mjpeg_to_rgb8(&jpeg).unwrap()
        );
    }

    #[test]
    fn rejects_mjpeg_dimensions_before_allocating_pixels() {
        let mut jpeg = encode_rgb8_jpeg(&[42; 12], 2, 2, 80).expect("encode JPEG");
        let sof = jpeg
            .windows(2)
            .position(|bytes| bytes == [0xFF, 0xC0])
            .expect("baseline JPEG frame header");
        jpeg[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(decode_mjpeg_to_rgb8(&jpeg).is_err());
    }

    #[test]
    fn every_image_decoder_rejects_excessive_header_dimensions() {
        let original = encode_rgb8_jpeg(&[42; 12], 2, 2, 80).expect("encode JPEG");
        let sof = original
            .windows(2)
            .position(|bytes| bytes == [0xFF, 0xC0])
            .unwrap();
        for (width, height) in [(8193_u16, 1_u16), (4001, 4000)] {
            let mut jpeg = original.clone();
            jpeg[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
            jpeg[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
            assert!(decode_mjpeg_to_rgb8(&jpeg).is_err());
            assert!(decode_reference_image_to_rgb8(&jpeg).is_err());
            assert!(decode_image_to_rgb8(&jpeg).is_err());
        }
    }

    #[test]
    fn thumbnail_preserves_aspect_ratio_and_has_decodable_pixels() {
        let pixels = [80; 4 * 2 * 3];
        let (width, height, resized) = resize_rgb8_to_fit(&pixels, 4, 2, 3).unwrap();
        assert_eq!((width, height), (3, 2));
        assert_eq!(resized, [80; 3 * 2 * 3]);
        let thumbnail = create_thumbnail_jpeg(&pixels, 4, 2, 3).unwrap();
        let (width, height, decoded) = decode_image_to_rgb8(&thumbnail).unwrap();
        assert_eq!((width, height, decoded.len()), (3, 2, 18));
        assert_eq!(resize_rgb8_to_fit(&[80; 3], 1, 1, 2).unwrap().0, 2);
    }

    #[test]
    fn rejects_invalid_resize_and_raw_dimensions_without_allocating() {
        for (width, height, limit) in [(0, 2, 4), (2, 2, 0), (2, 2, 8193), (8193, 1, 4)] {
            assert!(resize_rgb8_to_fit(&[0; 12], width, height, limit).is_err());
        }
        assert!(resize_rgb8_to_fit(&[0; 13], 2, 2, 4).is_err());
        for (width, height) in [(0, 0), (8193, 1), (4001, 4000), (u32::MAX, u32::MAX)] {
            assert!(convert_bgra8_to_rgb8(&[], width, height).is_err());
            assert!(convert_nv12_to_rgb8(&[], width, height).is_err());
            assert!(convert_yuyv_to_rgb8(&[], width, height).is_empty());
            assert!(encode_rgb8_jpeg(&[], width, height, 80).is_err());
        }
    }

    #[test]
    fn ignores_jpeg_markers_embedded_in_app_payload() {
        let pixels = [64; 2 * 2 * 3];
        let jpeg = encode_rgb8_jpeg(&pixels, 2, 2, 95).expect("encode synthetic JPEG");
        for payload in [[0xFF, 0xC4, 0x00, 0x00], [0x00, 0x00, 0xFF, 0xDA]] {
            let mut stripped = jpeg_without_dht(&jpeg);
            // APP1's payload resembles a marker, but its length keeps it inside APP1.
            stripped.splice(2..2, [0xFF, 0xE1, 0x00, 0x06].into_iter().chain(payload));

            let patched = ensure_jpeg_has_dht(&stripped);
            assert!(matches!(patched, Cow::Owned(_)));
            assert_eq!(
                decode_mjpeg_to_rgb8(&stripped).unwrap().2.len(),
                pixels.len()
            );
        }
    }

    #[test]
    fn keeps_jpeg_with_existing_dht_borrowed() {
        let jpeg = encode_rgb8_jpeg(&[42; 12], 2, 2, 80).expect("encode synthetic JPEG");
        assert!(matches!(ensure_jpeg_has_dht(&jpeg), Cow::Borrowed(_)));
    }

    #[test]
    fn converts_nv12_black_and_white() {
        let black = convert_nv12_to_rgb8(&[0, 0, 0, 0, 128, 128], 2, 2)
            .expect("valid compact NV12 black frame");
        assert_eq!(black, [0; 12]);

        let white = convert_nv12_to_rgb8(&[255, 255, 255, 255, 128, 128], 2, 2)
            .expect("valid compact NV12 white frame");
        assert_eq!(white, [255; 12]);
    }

    #[test]
    fn converts_nv12_chroma_for_each_two_by_two_block() {
        // Four columns share two chroma pairs, each repeated on the next row.
        let frame = [128; 8]
            .into_iter()
            .chain([255, 128, 128, 255])
            .collect::<Vec<_>>();
        let rgb = convert_nv12_to_rgb8(&frame, 4, 2).unwrap();
        assert_eq!(&rgb[..3], &[128, 84, 255]);
        assert_eq!(&rgb[3..6], &[128, 84, 255]);
        assert_eq!(&rgb[6..9], &[255, 37, 128]);
        assert_eq!(&rgb[12..], &rgb[..12]);
    }

    #[test]
    fn converts_bgra_channel_order_and_ignores_padding() {
        assert_eq!(
            convert_bgra8_to_rgb8(&[1, 2, 3, 99, 4, 5, 6, 88, 0], 2, 1).unwrap(),
            [3, 2, 1, 6, 5, 4]
        );
    }

    #[test]
    fn rejects_invalid_nv12_buffers_and_dimensions() {
        for (buffer, width, height) in [
            (&[0, 0, 0, 0, 128][..], 2, 2),
            (&[0, 0, 0, 0, 128, 128, 0][..], 2, 2),
            (&[0, 0, 0, 128][..], 1, 2),
            (&[][..], 0, 2),
        ] {
            assert!(matches!(
                convert_nv12_to_rgb8(buffer, width, height),
                Err(ImagingError::InvalidBufferSize)
            ));
        }
    }

    #[test]
    fn converts_yuyv_black_and_white() {
        // Two pixels: black (Y=0, U=128, Y=0, V=128)
        let yuyv = [0, 128, 0, 128];
        let rgb = convert_yuyv_to_rgb8(&yuyv, 2, 1);
        assert_eq!(rgb.len(), 6);
        assert_eq!(rgb, [0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn converts_yuyv_chroma_and_rejects_incomplete_rows() {
        assert_eq!(
            convert_yuyv_to_rgb8(&[128, 255, 128, 128], 2, 1),
            [128, 84, 255, 128, 84, 255]
        );
        assert!(convert_yuyv_to_rgb8(&[128, 255, 128], 2, 1).is_empty());
        assert!(convert_yuyv_to_rgb8(&[128; 12], 3, 2).is_empty());
        assert!(convert_yuyv_to_rgb8(&[], u32::MAX, u32::MAX).is_empty());
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
