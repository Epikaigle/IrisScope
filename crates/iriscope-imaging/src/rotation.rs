//! Rotation geometry shared by display pixels and exported annotations.

use image::{Rgb, RgbImage};

/// Full image bounds after a clockwise rotation, without cropping the source.
#[derive(Clone, Copy, Debug)]
pub struct RotationGeometry {
    width: f64,
    height: f64,
    cosine: f64,
    sine: f64,
    output_width: u32,
    output_height: u32,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Bounded image dimensions.
impl RotationGeometry {
    /// Computes the enclosing pixel bounds. Angles are normalized to 0..359°.
    #[must_use]
    pub fn new(width: u32, height: u32, degrees: i32) -> Self {
        let (cosine, sine) = match degrees.rem_euclid(360) {
            0 => (1.0, 0.0),
            90 => (0.0, 1.0),
            180 => (-1.0, 0.0),
            270 => (0.0, -1.0),
            angle => {
                let radians = f64::from(angle).to_radians();
                (radians.cos(), radians.sin())
            }
        };
        let width = f64::from(width);
        let height = f64::from(height);
        Self {
            width,
            height,
            cosine,
            sine,
            output_width: (width * cosine.abs() + height * sine.abs()).ceil() as u32,
            output_height: (width * sine.abs() + height * cosine.abs()).ceil() as u32,
        }
    }

    /// Dimensions of the enclosing rotated image.
    #[must_use]
    pub fn dimensions(self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    /// Maps normalized source coordinates into normalized output coordinates.
    #[must_use]
    pub fn project(self, x: f32, y: f32, mirror: bool) -> (f32, f32) {
        let dx = (f64::from(x) - 0.5) * self.width;
        let dy = (f64::from(y) - 0.5) * self.height;
        let x = ((dx * self.cosine - dy * self.sine) / f64::from(self.output_width.max(1)) + 0.5)
            as f32;
        let y = ((dx * self.sine + dy * self.cosine) / f64::from(self.output_height.max(1)) + 0.5)
            as f32;
        (if mirror { 1.0 - x } else { x }, y)
    }

    fn source_pixel(self, x: u32, y: u32) -> (f64, f64) {
        let dx = f64::from(x) + 0.5 - f64::from(self.output_width) / 2.0;
        let dy = f64::from(y) + 0.5 - f64::from(self.output_height) / 2.0;
        (
            dx * self.cosine + dy * self.sine + self.width / 2.0 - 0.5,
            -dx * self.sine + dy * self.cosine + self.height / 2.0 - 0.5,
        )
    }
}

/// Rotates RGB pixels clockwise with bilinear sampling and black outer corners.
#[must_use]
pub fn rotate_rgb(image: RgbImage, degrees: i32) -> RgbImage {
    match degrees.rem_euclid(360) {
        0 => image,
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => {
            let geometry = RotationGeometry::new(image.width(), image.height(), degrees);
            let (width, height) = geometry.dimensions();
            RgbImage::from_fn(width, height, |x, y| {
                let (sx, sy) = geometry.source_pixel(x, y);
                sample(&image, sx, sy)
            })
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Coordinates and colors clamped.
fn sample(image: &RgbImage, x: f64, y: f64) -> Rgb<u8> {
    if image.width() == 0
        || image.height() == 0
        || x < -0.5
        || y < -0.5
        || x > f64::from(image.width()) - 0.5
        || y > f64::from(image.height()) - 0.5
    {
        return Rgb([0; 3]);
    }
    let x = x.clamp(0.0, f64::from(image.width() - 1));
    let y = y.clamp(0.0, f64::from(image.height() - 1));
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let fx = x - f64::from(x0);
    let fy = y - f64::from(y0);
    Rgb(std::array::from_fn(|channel| {
        let top = f64::from(image.get_pixel(x0, y0)[channel]) * (1.0 - fx)
            + f64::from(image.get_pixel(x1, y0)[channel]) * fx;
        let bottom = f64::from(image.get_pixel(x0, y1)[channel]) * (1.0 - fx)
            + f64::from(image.get_pixel(x1, y1)[channel]) * fx;
        (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precise_rotation_preserves_the_whole_non_square_image() {
        let source = RgbImage::from_pixel(320, 240, Rgb([200, 80, 40]));
        let output = rotate_rgb(source.clone(), 37);
        assert_eq!(output.dimensions(), (400, 385));
        assert_eq!(output.get_pixel(200, 192), &Rgb([200, 80, 40]));
        assert_eq!(output.get_pixel(0, 0), &Rgb([0, 0, 0]));
        for (x, y) in [(0.02, 0.02), (0.98, 0.02), (0.02, 0.98), (0.98, 0.98)] {
            let (x, y) = RotationGeometry::new(320, 240, 37).project(x, y, false);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let pixel =
                output.get_pixel((f64::from(x) * 400.0) as u32, (f64::from(y) * 385.0) as u32);
            assert_eq!(pixel, &Rgb([200, 80, 40]));
        }
        assert_eq!(rotate_rgb(source.clone(), 360), source);
    }
    #[test]
    fn cardinal_projection_and_mirror_stay_exact() {
        let g = RotationGeometry::new(320, 240, 90);
        assert_eq!(g.dimensions(), (240, 320));
        let (x, y) = g.project(0.2, 0.7, false);
        assert!((x - 0.3).abs() < 0.0001 && (y - 0.2).abs() < 0.0001);
        let (mx, my) = g.project(0.2, 0.7, true);
        assert!((mx - 0.7).abs() < 0.0001 && (my - y).abs() < 0.0001);
    }
}
