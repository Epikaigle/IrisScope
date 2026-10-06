//! Annotated PNG copies and Unicode PDF observation sheets, preserving all sources.
use crate::{
    photo_tools::{self},
    runtime::AppRuntime,
    ui::{AppState, MainWindow},
};
use iriscope_core::library::{AnnotationKind, CaptureFileVersion, PhotoReview};
use slint::ComponentHandle;
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};
const FONT: &[u8] = include_bytes!("../../../ui/fonts/Manrope.ttf");

struct Outline(tiny_skia::PathBuilder);
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}
fn raster_text(
    canvas: &mut tiny_skia::Pixmap,
    text: &str,
    mut x: f32,
    y: f32,
    size: f32,
    paint: &tiny_skia::Paint<'_>,
) {
    let Ok(face) = ttf_parser::Face::parse(FONT, 0) else {
        return;
    };
    let scale = size / f32::from(face.units_per_em());
    for ch in text.chars() {
        if ch == '\n' {
            break;
        }
        let Some(glyph) = face.glyph_index(ch) else {
            continue;
        };
        let mut outline = Outline(tiny_skia::PathBuilder::new());
        face.outline_glyph(glyph, &mut outline);
        if let Some(path) = outline.0.finish() {
            canvas.fill_path(
                &path,
                paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::from_scale(scale, -scale).post_translate(x, y + size),
                None,
            );
        }
        x += f32::from(face.glyph_hor_advance(glyph).unwrap_or(face.units_per_em())) * scale;
    }
}
#[allow(clippy::cast_precision_loss)] // Decoded dimensions are bounded to 8192 pixels.
pub(super) fn annotated(
    mut image: image::RgbImage,
    review: &PhotoReview,
) -> io::Result<image::RgbImage> {
    review.validate()?;
    let mut canvas = tiny_skia::Pixmap::new(image.width(), image.height())
        .ok_or_else(|| io::Error::other("Image trop grande."))?;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(255, 205, 102, 255);
    let stroke = tiny_skia::Stroke {
        width: (image.width() as f32 / 500.0).clamp(2.0, 12.0),
        ..Default::default()
    };
    for a in &review.annotations {
        let x = a.x * image.width() as f32;
        let y = a.y * image.height() as f32;
        let bx = a.end_x * image.width() as f32;
        let by = a.end_y * image.height() as f32;
        let mut path = tiny_skia::PathBuilder::new();
        match a.kind {
            AnnotationKind::Circle => {
                path.push_circle(x, y, (bx - x).hypot(by - y).max(1.0));
            }
            AnnotationKind::Arrow => {
                path.move_to(x, y);
                path.line_to(bx, by);
                let angle = (by - y).atan2(bx - x);
                let tip = stroke.width * 5.0;
                for offset in [-0.52_f32, 0.52] {
                    path.move_to(bx, by);
                    path.line_to(
                        bx - tip * (angle + offset).cos(),
                        by - tip * (angle + offset).sin(),
                    );
                }
            }
            AnnotationKind::Point => {
                path.push_circle(x, y, stroke.width * 2.0);
            }
            AnnotationKind::Text => {
                raster_text(
                    &mut canvas,
                    &a.text,
                    x,
                    y,
                    (image.width() as f32 / 60.0).clamp(14.0, 60.0),
                    &paint,
                );
                continue;
            }
        }
        if let Some(path) = path.finish() {
            if a.kind == AnnotationKind::Point {
                canvas.fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    tiny_skia::Transform::identity(),
                    None,
                );
            } else {
                canvas.stroke_path(
                    &path,
                    &paint,
                    &stroke,
                    tiny_skia::Transform::identity(),
                    None,
                );
            }
        }
    }
    for (pixel, overlay) in image.pixels_mut().zip(canvas.pixels()) {
        let alpha = u16::from(overlay.alpha());
        for (value, new) in pixel
            .0
            .iter_mut()
            .zip([overlay.red(), overlay.green(), overlay.blue()])
        {
            *value = (u16::from(new) + u16::from(*value) * (255 - alpha) / 255).min(255) as u8;
        }
    }
    Ok(image)
}
#[derive(Default)]
struct Pdf {
    objects: Vec<Vec<u8>>,
}
impl Pdf {
    fn add(&mut self, bytes: impl Into<Vec<u8>>) -> usize {
        self.objects.push(bytes.into());
        self.objects.len()
    }
    fn stream(&mut self, header: &str, data: &[u8]) -> usize {
        let mut bytes = format!("<< {header} /Length {} >>\nstream\n", data.len()).into_bytes();
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(b"\nendstream");
        self.add(bytes)
    }
    fn finish(self, root: usize) -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in self.objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }
}
fn pdf_text(face: &ttf_parser::Face<'_>, text: &str, size: f32, x: f32, y: f32) -> String {
    let mut hex = String::new();
    for ch in text.chars() {
        let id = face.glyph_index(ch).map_or(0, |id| id.0);
        let _ = write!(hex, "{id:04X}");
    }
    format!("BT /F1 {size} Tf 0.12 0.15 0.2 rg 1 0 0 1 {x} {y} Tm <{hex}> Tj ET\n")
}
fn wrap_notes(face: &ttf_parser::Face<'_>, text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut width = 0.0;
    for ch in text.chars() {
        let advance = face
            .glyph_index(ch)
            .and_then(|g| face.glyph_hor_advance(g))
            .map_or(6.0, |v| {
                f32::from(v) * 11.0 / f32::from(face.units_per_em())
            });
        if ch == '\n' || width + advance > 510.0 {
            lines.push(std::mem::take(&mut current));
            width = 0.0;
        }
        if ch != '\n' && ch != '\r' {
            current.push(ch);
            width += advance;
        }
    }
    lines.push(current);
    lines
}
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)] // The writer emits one bounded document; dimensions <=8192 and at most two images.
fn make_pdf(
    image: &image::RgbImage,
    caption: &str,
    review: &PhotoReview,
    comparison: Option<(&image::RgbImage, &str)>,
) -> io::Result<Vec<u8>> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|e| io::Error::other(format!("Police indisponible : {e:?}")))?;
    let mut pdf = Pdf::default();
    let catalog = pdf.add(Vec::new());
    let pages = pdf.add(Vec::new());
    let font_file = pdf.stream(&format!("/Length1 {}", FONT.len()), FONT);
    let descriptor=pdf.add(format!("<< /Type /FontDescriptor /FontName /Manrope /Flags 32 /FontBBox [-1200 -1200 2200 2200] /ItalicAngle 0 /Ascent 1000 /Descent -300 /CapHeight 750 /StemV 80 /FontFile2 {font_file} 0 R >>").into_bytes());
    let mut chars = BTreeMap::new();
    let all = format!(
        "IrisScope Observations visuelles Notes (suite) · Photo originale annotée {} {} {}",
        caption,
        review.notes,
        comparison.map_or("", |(_, c)| c)
    );
    for ch in all.chars() {
        if let Some(gid) = face.glyph_index(ch) {
            chars.entry(gid.0).or_insert(ch);
        }
    }
    let mut widths = String::new();
    let mut mappings = String::new();
    for (&gid, &ch) in &chars {
        let width = f32::from(
            face.glyph_hor_advance(ttf_parser::GlyphId(gid))
                .unwrap_or(0),
        ) * 1000.0
            / f32::from(face.units_per_em());
        let _ = write!(widths, "{gid} [{width:.2}] ");
        let mut utf = String::new();
        for unit in ch.encode_utf16(&mut [0; 2]) {
            let _ = write!(utf, "{unit:04X}");
        }
        let _ = writeln!(mappings, "<{gid:04X}> <{utf}>");
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /ManropeUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let mappings: Vec<_> = mappings.lines().collect();
    for chunk in mappings.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for mapping in chunk {
            let _ = writeln!(cmap, "{mapping}");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let unicode = pdf.stream("", cmap.as_bytes());
    let cidfont=pdf.add(format!("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Manrope /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 500 /W [{widths}] >>").into_bytes());
    let font=pdf.add(format!("<< /Type /Font /Subtype /Type0 /BaseFont /Manrope /Encoding /Identity-H /DescendantFonts [{cidfont} 0 R] /ToUnicode {unicode} 0 R >>").into_bytes());
    let mut images = vec![(image, caption)];
    if let Some(other) = comparison {
        images.push(other);
    }
    let mut image_ids = Vec::new();
    for (img, _) in &images {
        let encoded =
            iriscope_imaging::encode_rgb8_jpeg(img.as_raw(), img.width(), img.height(), 95)
                .map_err(io::Error::other)?;
        image_ids.push(pdf.stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode",img.width(),img.height()),&encoded));
    }
    let notes = wrap_notes(&face, &review.notes);
    let mut note = 0;
    let mut page_ids = Vec::new();
    loop {
        let first = page_ids.is_empty();
        let mut content = pdf_text(
            &face,
            if first {
                "IrisScope · Observations visuelles"
            } else {
                "IrisScope · Notes (suite)"
            },
            18.0,
            40.0,
            796.0,
        );
        content.push_str(&pdf_text(&face, caption, 10.0, 40.0, 775.0));
        let mut y = 746.0;
        if first {
            let count = images.len();
            let cell = if count == 1 { 515.0 } else { 250.0 };
            for (index, (img, cap)) in images.iter().enumerate() {
                let scale = (cell / img.width() as f32).min(365.0 / img.height() as f32);
                let w = img.width() as f32 * scale;
                let h = img.height() as f32 * scale;
                let x = 40.0 + index as f32 * 265.0;
                let _ = writeln!(content, "q {w} 0 0 {h} {x} {} cm /Im{index} Do Q", y - h);
                content.push_str(&pdf_text(&face, cap, 9.0, x, y - 383.0));
            }
            y -= 415.0;
            content.push_str(&pdf_text(&face, "Notes", 13.0, 40.0, y));
            y -= 22.0;
        }
        while note < notes.len() && y > 52.0 {
            content.push_str(&pdf_text(&face, &notes[note], 11.0, 40.0, y));
            note += 1;
            y -= 16.0;
        }
        let stream = pdf.stream("", content.as_bytes());
        let mut resources = format!("/Font << /F1 {font} 0 R >> /XObject << ");
        for (index, id) in image_ids.iter().enumerate() {
            let _ = write!(resources, "/Im{index} {id} 0 R ");
        }
        resources.push_str(">>");
        page_ids.push(pdf.add(format!("<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 595 842] /Resources << {resources} >> /Contents {stream} 0 R >>").into_bytes()));
        if note >= notes.len() {
            break;
        }
    }
    pdf.objects[catalog - 1] = format!("<< /Type /Catalog /Pages {pages} 0 R >>").into_bytes();
    pdf.objects[pages - 1] = format!(
        "<< /Type /Pages /Count {} /Kids [{}] >>",
        page_ids.len(),
        page_ids
            .iter()
            .map(|id| format!("{id} 0 R"))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .into_bytes();
    Ok(pdf.finish(catalog))
}
fn publish(destination: &Path, bytes: &[u8], id: u64, cancel: &dyn Fn() -> bool) -> io::Result<()> {
    let directory = destination
        .parent()
        .ok_or_else(|| io::Error::other("Dossier d’export absent."))?;
    let temporary = directory.join(format!(
        ".iriscope-photo-export-{}-{id}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(&temporary)?;
    let result = (|| {
        for chunk in bytes.chunks(64 * 1024) {
            if cancel() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Export annulé."));
            }
            output.write_all(chunk)?;
        }
        output.sync_all()?;
        if cancel() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Export annulé."));
        }
        fs::hard_link(&temporary, destination)?;
        Ok(())
    })();
    drop(output);
    let _ = fs::remove_file(&temporary);
    result
}
fn finish(win: &MainWindow, result: io::Result<Option<PathBuf>>) {
    let s = win.global::<AppState>();
    s.set_export_busy(false);
    s.set_file_dialog_pending(false);
    s.set_export_cancelling(false);
    s.set_export_feedback_error(result.is_err());
    s.set_export_feedback(
        match result {
            Ok(Some(path)) => format!("Export enregistré : {}", path.display()),
            Ok(None) => "Export annulé.".into(),
            Err(e) => format!("Export impossible : {e}"),
        }
        .into(),
    );
}
#[allow(clippy::too_many_lines)] // One export owns the picker, verified source and atomic publication through completion.
pub(super) fn install(window: &MainWindow, runtime: &AppRuntime) {
    let weak = window.as_weak();
    let jobs = Arc::clone(&runtime.background_jobs);
    let control = Arc::clone(&runtime.export_operation);
    let closing = Arc::clone(&runtime.closing);
    window
        .global::<AppState>()
        .on_export_viewer_photo(move |kind| {
            let Some(win) = weak.upgrade() else {
                return;
            };
            let s = win.global::<AppState>();
            if !(0..=1).contains(&kind)
                || s.get_viewer_is_video()
                || s.get_viewer_loading()
                || s.get_export_busy()
                || s.get_storage_busy()
                || s.get_file_dialog_pending()
                || closing.load(Ordering::Acquire)
            {
                return;
            }
            let Some(pixels) = s.get_viewer_original().to_rgb8() else {
                return;
            };
            let Ok(expected) = CaptureFileVersion::from_token(s.get_viewer_file_version().as_str())
            else {
                return;
            };
            let review = photo_tools::snapshot(&win);
            if let Err(e) = review.validate() {
                finish(&win, Err(e));
                return;
            }
            let settings = photo_tools::display_settings(&win);
            let source = PathBuf::from(s.get_viewer_path().as_str());
            let caption = s.get_viewer_caption().to_string();
            let comparison = if kind == 1 && s.get_comparison_enabled() {
                s.get_comparison_image()
                    .to_rgb8()
                    .map(|p| (p, s.get_comparison_caption().to_string()))
            } else {
                None
            };
            let id = control.begin();
            s.set_export_busy(true);
            s.set_export_feedback_error(false);
            s.set_export_feedback("Choisissez l’emplacement de l’export.".into());
            s.set_file_dialog_pending(true);
            let weak = weak.clone();
            let control_job = Arc::clone(&control);
            let closing = Arc::clone(&closing);
            if !jobs.submit(move || {
                let cancel = || closing.load(Ordering::Acquire) || control_job.is_cancelled(id);
                let result = (|| {
                    if cancel() {
                        return Ok(None);
                    }
                    let extension = if kind == 0 { "png" } else { "pdf" };
                    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
                    let dialog = rfd::FileDialog::new()
                        .set_title("IrisScope — Export des observations")
                        .set_directory(source.parent().unwrap_or(Path::new(".")))
                        .set_file_name(format!("{stem}-observations.{extension}"))
                        .add_filter(
                            if kind == 0 { "Image PNG" } else { "Fiche PDF" },
                            &[extension],
                        );
                    let Some(mut destination) = dialog.save_file() else {
                        return Ok(None);
                    };
                    if destination.extension().is_none() {
                        destination.set_extension(extension);
                    }
                    if destination
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case(extension))
                    {
                        return Err(io::Error::other(
                            "Choisissez l’extension correspondant au format d’export.",
                        ));
                    }
                    let _ = weak.upgrade_in_event_loop(|win| {
                        let s = win.global::<AppState>();
                        s.set_file_dialog_pending(false);
                        s.set_export_feedback("Préparation de la copie…".into());
                    });
                    let file = crate::playback::open_verified_capture_cancellable(
                        &source, &expected, &cancel,
                    )?;
                    let original = image::RgbImage::from_raw(
                        pixels.width(),
                        pixels.height(),
                        pixels.as_bytes().to_vec(),
                    )
                    .ok_or_else(|| io::Error::other("Image indisponible."))?;
                    let bytes = if kind == 0 {
                        let img = photo_tools::transform(original, settings);
                        let img =
                            annotated(img, &photo_tools::projected_review(&review, settings))?;
                        iriscope_imaging::encode_rgb8_png(img.as_raw(), img.width(), img.height())
                            .map_err(io::Error::other)?
                    } else {
                        let reference = comparison.and_then(|(p, caption)| {
                            image::RgbImage::from_raw(p.width(), p.height(), p.as_bytes().to_vec())
                                .map(|i| (i, caption))
                        });
                        let original = annotated(original, &review)?;
                        make_pdf(
                            &original,
                            &caption,
                            &review,
                            reference.as_ref().map(|(i, c)| (i, c.as_str())),
                        )?
                    };
                    crate::playback::verify_open_capture_cancellable(
                        &file, &source, &expected, &cancel,
                    )?;
                    publish(&destination, &bytes, id, &cancel)?;
                    Ok(Some(destination))
                })();
                control_job.finish(id);
                let result = match result {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted && cancel() => Ok(None),
                    other => other,
                };
                let _ = weak.upgrade_in_event_loop(move |win| finish(&win, result));
            }) {
                control.finish(id);
                finish(
                    &win,
                    Err(io::Error::other("Traitement disque en cours. Réessayez.")),
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use iriscope_core::library::Annotation;
    #[test]
    fn annotations_change_the_copy_and_encode_a_readable_png() {
        let original = image::RgbImage::from_pixel(128, 96, image::Rgb([10, 20, 30]));
        let review = PhotoReview {
            annotations: vec![
                Annotation {
                    kind: AnnotationKind::Arrow,
                    x: 0.2,
                    y: 0.2,
                    end_x: 0.8,
                    end_y: 0.8,
                    text: String::new(),
                },
                Annotation {
                    kind: AnnotationKind::Text,
                    x: 0.1,
                    y: 0.3,
                    end_x: 0.1,
                    end_y: 0.3,
                    text: "Œil observé".into(),
                },
            ],
            ..Default::default()
        };
        let copy = annotated(original.clone(), &review).unwrap();
        assert_ne!(copy, original);
        let png =
            iriscope_imaging::encode_rgb8_png(copy.as_raw(), copy.width(), copy.height()).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgb8(), copy);
        assert_eq!(original.get_pixel(30, 30).0, [10, 20, 30]);
    }
    #[test]
    fn unicode_pdf_paginates_long_notes_and_contains_both_photos() {
        let original = image::RgbImage::from_pixel(80, 60, image::Rgb([10, 20, 30]));
        let review = PhotoReview {
            notes: "Éclairage · œil gauche · Δ\n".repeat(90),
            ..Default::default()
        };
        let pdf = make_pdf(
            &original,
            "D-000042 · Œil gauche",
            &review,
            Some((&original, "2026-09-01 · Œil gauche")),
        )
        .unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(text.contains("/ToUnicode"));
        assert!(text.contains("<0153>"));
        assert!(text.contains("/Im1"));
        assert!(text.matches("/Type /Page ").count() >= 3);
        assert!(pdf.ends_with(b"%%EOF\n"));
        if let Ok(path) = std::env::var("IRISCOPE_TEST_PDF_OUTPUT") {
            fs::write(path, pdf).unwrap();
        }
    }
    #[test]
    fn publishing_never_overwrites_and_cancel_removes_temporary_copy() {
        let root =
            std::env::temp_dir().join(format!("iriscope-photo-export-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("copy.png");
        fs::write(&path, b"existing").unwrap();
        assert!(publish(&path, b"replacement", 1, &|| false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing");
        assert!(publish(&root.join("cancelled.png"), b"copy", 2, &|| true).is_err());
        assert!(!root.join("cancelled.png").exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
