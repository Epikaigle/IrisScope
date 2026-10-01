//! Direct lossless Motion-JPEG AVI video recording.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{
    capabilities::FrameRate,
    storage::{
        collision_name, create_private_directory, publish_completed_capture,
        validate_capture_file_name,
    },
};

const MAX_AVI_FRAME_BYTES: u32 = 32 * 1024 * 1024;
const MAX_AVI_PIXELS: u64 = 16_000_000;
const MAX_AVI_FRAMES: usize = 1_000_000;
const MAX_AVI_CHUNKS: usize = 1_100_000;
const AVI_HEADER_BYTES: usize = 2_048;

fn header_u32(header: &[u8; AVI_HEADER_BYTES], offset: usize) -> u32 {
    header
        .get(offset..offset.saturating_add(4))
        .and_then(|bytes| bytes.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

// These offsets describe the header emitted by AviMjpegWriter. AVI itself has
// many other valid layouts, which the built-in player cannot safely assume.
fn iriscope_header_details(header: &[u8; AVI_HEADER_BYTES]) -> Option<(u32, u32, FrameRate)> {
    if &header[0..4] != b"RIFF"
        || &header[8..12] != b"AVI "
        || &header[12..16] != b"LIST"
        || header_u32(header, 16) != 192
        || &header[20..24] != b"hdrl"
        || &header[24..28] != b"avih"
        || header_u32(header, 28) != 56
        || &header[88..92] != b"LIST"
        || header_u32(header, 92) != 116
        || &header[96..100] != b"strl"
        || &header[100..104] != b"strh"
        || header_u32(header, 104) != 56
        || &header[108..112] != b"vids"
        || &header[112..116] != b"MJPG"
        || &header[164..168] != b"strf"
        || header_u32(header, 168) != 40
        || header_u32(header, 172) != 40
        || &header[188..192] != b"MJPG"
        || &header[212..216] != b"JUNK"
        || header_u32(header, 216) != 1_816
        || &header[2_036..2_040] != b"LIST"
        || &header[2_044..2_048] != b"movi"
    {
        return None;
    }

    let width = header_u32(header, 64);
    let height = header_u32(header, 68);
    if width == 0
        || height == 0
        || width > 8_192
        || height > 8_192
        || u64::from(width) * u64::from(height) > MAX_AVI_PIXELS
        || header_u32(header, 176) != width
        || header_u32(header, 180) != height
        || header_u32(header, 32) == 0
    {
        return None;
    }
    let frame_rate = FrameRate::new(header_u32(header, 132), header_u32(header, 128))?;
    Some((width, height, frame_rate))
}

fn completed_movi_end(
    file: &mut File,
    header: &[u8; AVI_HEADER_BYTES],
    file_len: u64,
) -> io::Result<Option<u64>> {
    let frame_count = u64::from(header_u32(header, 48));
    let stream_frame_count = u64::from(header_u32(header, 140));
    let movi_size = u64::from(header_u32(header, 2_040));
    let Some(index_bytes) = frame_count.checked_mul(16) else {
        return Ok(None);
    };
    let Some(movi_end) = 2_044_u64.checked_add(movi_size) else {
        return Ok(None);
    };
    if frame_count == 0
        || frame_count != stream_frame_count
        || frame_count > MAX_AVI_FRAMES as u64
        || movi_size < 12
        || u64::from(header_u32(header, 4)).checked_add(8) != Some(file_len)
        || movi_end
            .checked_add(8)
            .and_then(|end| end.checked_add(index_bytes))
            != Some(file_len)
    {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(movi_end))?;
    let mut index_header = [0_u8; 8];
    file.read_exact(&mut index_header)?;
    if &index_header[..4] != b"idx1"
        || u64::from(u32::from_le_bytes([
            index_header[4],
            index_header[5],
            index_header[6],
            index_header[7],
        ])) != index_bytes
    {
        return Ok(None);
    }
    Ok(Some(movi_end))
}

fn has_jpeg_envelope(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9])
}

fn invalid_avi(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Lightweight reader for MJPEG AVI files produced by `IrisScope`.
///
/// Only compressed-frame offsets are kept in memory. JPEG payloads are read lazily.
pub struct AviMjpegReader {
    file: File,
    frame_rate: FrameRate,
    frames: Vec<(u64, u32)>,
}

impl AviMjpegReader {
    /// Opens an `IrisScope` MJPEG AVI and indexes its video frames.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file is invalid, truncated, or cannot be read.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_file(File::open(path)?)
    }

    /// Indexes an AVI through an already opened, verified file handle.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file is invalid, truncated, or cannot be read.
    pub fn from_file(mut file: File) -> io::Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let file_len = file.metadata()?.len();
        if file_len < 2_048 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AVI file is smaller than the IrisScope header",
            ));
        }
        if file_len > u64::from(u32::MAX) + 8 {
            return Err(invalid_avi("classic AVI file exceeds 4 GiB"));
        }

        let mut header = [0_u8; AVI_HEADER_BYTES];
        file.read_exact(&mut header)?;
        let (_, _, frame_rate) = iriscope_header_details(&header)
            .ok_or_else(|| invalid_avi("unsupported IrisScope AVI header"))?;
        let movi_end = completed_movi_end(&mut file, &header, file_len)?
            .ok_or_else(|| invalid_avi("incomplete or inconsistent IrisScope AVI index"))?;

        let mut frames = Vec::new();
        let mut position = AVI_HEADER_BYTES as u64;
        let mut chunk_count = 0;

        while position.saturating_add(8) <= movi_end {
            if chunk_count >= MAX_AVI_CHUNKS {
                return Err(invalid_avi("AVI contains too many chunks"));
            }
            chunk_count += 1;
            file.seek(SeekFrom::Start(position))?;
            let mut chunk_header = [0_u8; 8];
            file.read_exact(&mut chunk_header)?;

            let chunk_id = &chunk_header[..4];
            let size = u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]);

            if chunk_id != b"00dc" {
                return Err(invalid_avi("unsupported AVI video chunk"));
            }
            if size > MAX_AVI_FRAME_BYTES {
                return Err(invalid_avi(
                    "AVI JPEG frame exceeds the safe playback limit",
                ));
            }

            let payload_offset = position.saturating_add(8);
            let payload_end = payload_offset.saturating_add(u64::from(size));
            if payload_end > movi_end {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated AVI chunk",
                ));
            }

            if frames.len() >= MAX_AVI_FRAMES {
                return Err(invalid_avi("AVI contains too many frames"));
            }
            frames.push((payload_offset, size));

            position = payload_end.saturating_add(u64::from(size % 2));
        }

        if frames.is_empty()
            || frames.len() != usize::try_from(header_u32(&header, 48)).unwrap_or(usize::MAX)
            || position != movi_end
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AVI frame list does not match its index",
            ));
        }

        Ok(Self {
            file,
            frame_rate,
            frames,
        })
    }

    /// Returns the recorded frame rate.
    #[must_use]
    pub const fn frame_rate(&self) -> FrameRate {
        self.frame_rate
    }

    /// Returns the number of indexed frames.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Reads the current version of the exact file handle used for playback.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the handle's filesystem metadata cannot be read.
    pub fn file_version(&self) -> io::Result<crate::library::CaptureFileVersion> {
        crate::file_validation::capture_file_version_from_file(&self.file)
    }

    /// Reads the playback handle's version without streaming its contents.
    ///
    /// # Errors
    ///
    /// Returns `Unsupported` if the filesystem cannot provide a strong native
    /// version, or an I/O error if its metadata cannot be read.
    pub fn file_version_fast(&self) -> io::Result<crate::library::CaptureFileVersion> {
        crate::file_validation::capture_file_version_fast_from_file(&self.file)
    }

    /// Reads one compressed JPEG frame.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the frame index is invalid or the payload cannot be read.
    pub fn read_frame(&mut self, index: usize) -> io::Result<Vec<u8>> {
        let &(offset, size) = self.frames.get(index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "AVI frame index out of range")
        })?;

        if size > MAX_AVI_FRAME_BYTES {
            return Err(invalid_avi(
                "AVI JPEG frame exceeds the safe playback limit",
            ));
        }

        self.file.seek(SeekFrom::Start(offset))?;
        let frame_size = usize::try_from(size).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "AVI frame is too large for this platform",
            )
        })?;
        let mut bytes = vec![0_u8; frame_size];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

/// Cheap compatibility check for a completed IrisScope-style MJPEG AVI.
/// This reads the header, first chunk header, and index header, not every frame.
///
/// # Errors
///
/// Returns an I/O error if the file cannot be opened or its header cannot be read.
pub fn is_iriscope_avi(path: &Path) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < 2_056 || file_len > u64::from(u32::MAX) + 8 {
        return Ok(false);
    }
    let mut header = [0_u8; AVI_HEADER_BYTES];
    file.read_exact(&mut header)?;
    if iriscope_header_details(&header).is_none() {
        return Ok(false);
    }
    let Some(movi_end) = completed_movi_end(&mut file, &header, file_len)? else {
        return Ok(false);
    };
    file.seek(SeekFrom::Start(AVI_HEADER_BYTES as u64))?;
    let mut first_chunk = [0_u8; 8];
    file.read_exact(&mut first_chunk)?;
    let first_size = u64::from(u32::from_le_bytes([
        first_chunk[4],
        first_chunk[5],
        first_chunk[6],
        first_chunk[7],
    ]));
    Ok(&first_chunk[..4] == b"00dc"
        && first_size >= 4
        && first_size <= u64::from(MAX_AVI_FRAME_BYTES)
        && 2_056 + first_size + first_size % 2 <= movi_end)
}

/// Copies every complete MJPEG frame from an interrupted `.part` recording
/// into a new, finalized AVI. The original is preserved for manual review.
/// Old placeholder-only files without dimensions cannot be recovered safely.
///
/// # Errors
///
/// Returns an error for an invalid source, unsafe dimensions, or failed I/O.
pub fn recover_partial_avi(
    source: &Path,
    directory: &Path,
    file_name: &str,
) -> io::Result<Option<PathBuf>> {
    validate_capture_file_name(file_name)?;
    recover_partial_avi_staged(source, directory)?
        .map(|pending| publish_completed_capture(&pending, directory, file_name))
        .transpose()
}

/// Salvages complete JPEG chunks into a finalized temporary AVI.
///
/// The source is preserved. The returned `.part` file is ready for a caller to
/// publish together with its capture metadata through `publish_indexed_capture`.
/// Returns `None` when the source contains no recoverable frames.
///
/// # Errors
///
/// Returns an error for an invalid header, unsupported bounds or failed I/O.
pub fn recover_partial_avi_staged(source: &Path, directory: &Path) -> io::Result<Option<PathBuf>> {
    let mut input = File::open(source)?;
    let file_len = input.metadata()?.len();
    if file_len < 2_048 || file_len > u64::from(u32::MAX) + 8 {
        return Err(invalid_avi("partial AVI size is unsupported"));
    }
    let mut header = [0_u8; AVI_HEADER_BYTES];
    input.read_exact(&mut header)?;
    let (width, height, rate) = iriscope_header_details(&header)
        .ok_or_else(|| invalid_avi("partial AVI has no recoverable IrisScope header"))?;
    let (mut writer, pending) =
        AviMjpegWriter::create_unique(directory, ".iriscope-recovery.part", width, height, rate)?;
    let result = (|| {
        let mut position = 2_048_u64;
        let mut recovered = 0_u32;
        let mut chunks = 0_usize;
        while position.saturating_add(8) <= file_len {
            if chunks >= MAX_AVI_CHUNKS {
                return Err(invalid_avi("partial AVI contains too many chunks"));
            }
            chunks += 1;
            input.seek(SeekFrom::Start(position))?;
            let mut chunk = [0_u8; 8];
            input.read_exact(&mut chunk)?;
            if &chunk[..4] == b"idx1" {
                break;
            }
            let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            let end = position.saturating_add(8).saturating_add(u64::from(size));
            if end > file_len {
                break;
            }
            if &chunk[..4] != b"00dc" {
                return Err(invalid_avi("partial AVI contains an unsupported chunk"));
            }
            if size > MAX_AVI_FRAME_BYTES
                || usize::try_from(recovered).unwrap_or(usize::MAX) >= MAX_AVI_FRAMES
            {
                return Err(invalid_avi("partial AVI exceeds safe recovery limits"));
            }
            let mut jpeg =
                vec![0_u8; usize::try_from(size).map_err(|_| invalid_avi("frame too large"))?];
            input.read_exact(&mut jpeg)?;
            // A complete AVI chunk can still hold an interrupted JPEG.
            // Keep the source intact and salvage only intact envelopes.
            if has_jpeg_envelope(&jpeg) {
                writer.write_frame(&jpeg)?;
                recovered += 1;
            }
            position = end.saturating_add(u64::from(size % 2));
        }
        if recovered == 0 {
            return Ok(None);
        }
        writer.finish()?;
        Ok(Some(recovered))
    })();
    drop(writer);
    match result {
        Ok(Some(_)) => Ok(Some(pending)),
        Ok(None) => {
            let _ = std::fs::remove_file(pending);
            Ok(None)
        }
        Err(error) => {
            let _ = std::fs::remove_file(pending);
            Err(error)
        }
    }
}

/// An active AVI MJPEG recording file.
///
/// Encapsulates raw camera JPEG frames without re-encoding, preserving exact original
/// quality with negligible CPU overhead.
pub struct AviMjpegWriter {
    file: File,
    width: u32,
    height: u32,
    frame_rate: FrameRate,
    frame_count: u32,
    movi_start_pos: u64,
    index_entries: Vec<(u32, u32)>, // (offset_from_movi, size)
    is_closed: bool,
}

fn avi_size_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "classic AVI cannot exceed 4 GiB; stop this recording and start another",
    )
}

fn checked_frame_capacity(
    current_pos: u64,
    movi_start_pos: u64,
    frame_size: usize,
    existing_frames: usize,
) -> io::Result<(u32, u32)> {
    if frame_size > MAX_AVI_FRAME_BYTES as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "JPEG frame exceeds the supported AVI playback limit",
        ));
    }
    let frame_size = u32::try_from(frame_size).map_err(|_| avi_size_error())?;
    let offset = current_pos
        .checked_sub(movi_start_pos)
        .and_then(|offset| u32::try_from(offset).ok())
        .ok_or_else(avi_size_error)?;
    let frame_count = existing_frames.checked_add(1).ok_or_else(avi_size_error)?;
    u32::try_from(frame_count).map_err(|_| avi_size_error())?;
    let index_bytes = frame_count
        .checked_mul(16)
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or_else(avi_size_error)?;
    let frame_end = current_pos
        .checked_add(8 + u64::from(frame_size) + u64::from(frame_size % 2))
        .ok_or_else(avi_size_error)?;
    let movi_list_size = frame_end
        .checked_sub(movi_start_pos)
        .and_then(|size| size.checked_add(4))
        .ok_or_else(avi_size_error)?;
    u32::try_from(movi_list_size).map_err(|_| avi_size_error())?;
    let total_size = frame_end
        .checked_add(8 + u64::from(index_bytes))
        .ok_or_else(avi_size_error)?;
    u32::try_from(total_size.saturating_sub(8)).map_err(|_| avi_size_error())?;
    Ok((offset, frame_size))
}

impl AviMjpegWriter {
    /// Opens a new AVI file and writes the initial RIFF header.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the destination file cannot be created.
    pub fn create(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
    ) -> io::Result<Self> {
        // The classic AVI header stores the frame rectangle in 16-bit fields
        // and the uncompressed image size in a 32-bit field.
        if width == 0
            || height == 0
            || width > 8_192
            || height > 8_192
            || u64::from(width) * u64::from(height) > MAX_AVI_PIXELS
            || u16::try_from(width).is_err()
            || u16::try_from(height).is_err()
            || u64::from(width) * u64::from(height) * 3 > u64::from(u32::MAX)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "video dimensions cannot be represented in an AVI header",
            ));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;

        // Reserve space for RIFF header (2048 bytes header placeholder)
        let placeholder = vec![0_u8; 2048];
        file.write_all(&placeholder)?;

        let movi_start_pos = file.stream_position()?;

        let mut writer = Self {
            file,
            width,
            height,
            frame_rate,
            frame_count: 0,
            movi_start_pos,
            index_entries: Vec::new(),
            is_closed: false,
        };
        // A valid provisional header makes completed frames recoverable even if
        // the process exits before finish() writes the final index and sizes.
        writer.file.seek(SeekFrom::Start(0))?;
        writer.write_avi_headers(2_048, 0)?;
        writer.file.seek(SeekFrom::Start(2_048))?;
        Ok(writer)
    }

    /// Creates a recording without overwriting an existing capture.
    ///
    /// When the requested filename already exists, a numeric suffix is appended
    /// before the extension until a new file can be created atomically.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the filename is invalid, the destination directory
    /// cannot be created, or the file cannot be opened for writing.
    pub fn create_unique(
        directory: &Path,
        file_name: &str,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
    ) -> io::Result<(Self, PathBuf)> {
        validate_capture_file_name(file_name)?;
        create_private_directory(directory)?;
        let requested = Path::new(file_name);
        let stem = requested
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Iris");
        let extension = requested.extension().and_then(|value| value.to_str());

        for collision_index in 1_u32.. {
            let candidate_name = collision_name(stem, extension, file_name, collision_index)?;
            validate_capture_file_name(&candidate_name)?;
            let candidate = directory.join(candidate_name);

            match Self::create(&candidate, width, height, frame_rate) {
                Ok(writer) => return Ok((writer, candidate)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }

        unreachable!("the collision counter covers every u32 filename suffix")
    }

    /// Appends one native JPEG frame to the video container.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if writing to disk fails or the classic AVI size
    /// limit would be exceeded. The check happens before writing a frame.
    pub fn write_frame(&mut self, jpeg_bytes: &[u8]) -> io::Result<()> {
        let current_pos = self.file.stream_position()?;
        let (offset_from_movi, size) = checked_frame_capacity(
            current_pos,
            self.movi_start_pos,
            jpeg_bytes.len(),
            self.index_entries.len(),
        )?;

        // Chunk ID: '00dc' (video compressed frame)
        self.file.write_all(b"00dc")?;
        self.file.write_all(&size.to_le_bytes())?;
        self.file.write_all(jpeg_bytes)?;

        // AVI chunks must be 2-byte aligned
        if size % 2 != 0 {
            self.file.write_all(&[0])?;
        }

        self.index_entries.push((offset_from_movi, size));
        self.frame_count += 1;
        Ok(())
    }

    /// Returns the number of frames recorded so far.
    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Sets the constant playback rate written to the AVI headers on finalization.
    /// Call this after measuring the timestamps of the frames actually recorded.
    pub fn set_frame_rate(&mut self, frame_rate: FrameRate) {
        self.frame_rate = frame_rate;
    }

    /// Finalizes the AVI file with index and complete headers.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if headers cannot be updated.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.is_closed {
            return Ok(());
        }

        let idx_size = self
            .index_entries
            .len()
            .checked_mul(16)
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or_else(avi_size_error)?;
        // Write index 'idx1'
        let idx_pos = self.file.stream_position()?;
        let total_file_size = idx_pos
            .checked_add(8)
            .and_then(|size| size.checked_add(u64::from(idx_size)))
            .ok_or_else(avi_size_error)?;
        let movi_size = idx_pos
            .checked_sub(self.movi_start_pos)
            .and_then(|size| u32::try_from(size).ok())
            .ok_or_else(avi_size_error)?;
        movi_size.checked_add(4).ok_or_else(avi_size_error)?;
        u32::try_from(total_file_size.saturating_sub(8)).map_err(|_| avi_size_error())?;
        self.file.write_all(b"idx1")?;
        self.file.write_all(&idx_size.to_le_bytes())?;

        for &(offset, size) in &self.index_entries {
            self.file.write_all(b"00dc")?;
            self.file.write_all(&0x10_u32.to_le_bytes())?; // AVIIF_KEYFRAME
            self.file.write_all(&offset.to_le_bytes())?;
            self.file.write_all(&size.to_le_bytes())?;
        }

        // Rewind and write real header
        self.file.seek(SeekFrom::Start(0))?;
        self.write_avi_headers(total_file_size, movi_size)?;

        self.file.flush()?;
        self.file.sync_all()?;
        self.is_closed = true;
        Ok(())
    }

    fn write_avi_headers(&mut self, total_file_size: u64, movi_size: u32) -> io::Result<()> {
        let riff_size =
            u32::try_from(total_file_size.saturating_sub(8)).map_err(|_| avi_size_error())?;
        let numerator = u64::from(self.frame_rate.numerator()).max(1);
        let denominator = u64::from(self.frame_rate.denominator()).max(1);
        let microsec_per_frame = u32::try_from(
            (1_000_000_u64
                .saturating_mul(denominator)
                .saturating_add(numerator / 2))
                / numerator,
        )
        .unwrap_or(u32::MAX);

        // RIFF Header
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&riff_size.to_le_bytes())?;
        self.file.write_all(b"AVI ")?;

        // hdrl LIST
        let hdrl_size = 4 + 64 + (12 + 64 + 48); // list type + avih + strl list
        self.file.write_all(b"LIST")?;
        self.file.write_all(&(hdrl_size as u32).to_le_bytes())?;
        self.file.write_all(b"hdrl")?;

        // avih chunk (56 bytes data + 8 header)
        self.file.write_all(b"avih")?;
        self.file.write_all(&56_u32.to_le_bytes())?;
        self.file.write_all(&microsec_per_frame.to_le_bytes())?;
        self.file.write_all(&0_u32.to_le_bytes())?; // max bytes/sec
        self.file.write_all(&0_u32.to_le_bytes())?; // padding
        self.file.write_all(&0x10_u32.to_le_bytes())?; // flags: has index
        self.file.write_all(&self.frame_count.to_le_bytes())?;
        self.file.write_all(&0_u32.to_le_bytes())?; // initial frames
        self.file.write_all(&1_u32.to_le_bytes())?; // streams
        self.file.write_all(&0_u32.to_le_bytes())?; // suggested buffer size
        self.file.write_all(&self.width.to_le_bytes())?;
        self.file.write_all(&self.height.to_le_bytes())?;
        self.file.write_all(&[0_u8; 16])?; // reserved

        // strl LIST
        let strl_size = 4 + (8 + 56) + (8 + 40);
        self.file.write_all(b"LIST")?;
        self.file.write_all(&(strl_size as u32).to_le_bytes())?;
        self.file.write_all(b"strl")?;

        // strh chunk (56 bytes)
        self.file.write_all(b"strh")?;
        self.file.write_all(&56_u32.to_le_bytes())?;
        self.file.write_all(b"vids")?;
        self.file.write_all(b"MJPG")?;
        self.file.write_all(&0_u32.to_le_bytes())?; // flags
        self.file.write_all(&0_u16.to_le_bytes())?; // priority
        self.file.write_all(&0_u16.to_le_bytes())?; // language
        self.file.write_all(&0_u32.to_le_bytes())?; // initial frames
        self.file
            .write_all(&self.frame_rate.denominator().to_le_bytes())?; // scale
        self.file
            .write_all(&self.frame_rate.numerator().to_le_bytes())?; // rate
        self.file.write_all(&0_u32.to_le_bytes())?; // start
        self.file.write_all(&self.frame_count.to_le_bytes())?; // length
        self.file.write_all(&0_u32.to_le_bytes())?; // suggested buffer size
        self.file.write_all(&10_000_u32.to_le_bytes())?; // quality
        self.file.write_all(&0_u32.to_le_bytes())?; // sample size
        self.file.write_all(&0_u16.to_le_bytes())?; // frame rect left
        self.file.write_all(&0_u16.to_le_bytes())?; // top
        self.file.write_all(&(self.width as u16).to_le_bytes())?;
        self.file.write_all(&(self.height as u16).to_le_bytes())?;

        // strf chunk (40 bytes BITMAPINFOHEADER)
        self.file.write_all(b"strf")?;
        self.file.write_all(&40_u32.to_le_bytes())?;
        self.file.write_all(&40_u32.to_le_bytes())?; // biSize
        self.file.write_all(&(self.width as i32).to_le_bytes())?;
        self.file.write_all(&(self.height as i32).to_le_bytes())?;
        self.file.write_all(&1_u16.to_le_bytes())?; // biPlanes
        self.file.write_all(&24_u16.to_le_bytes())?; // biBitCount
        self.file.write_all(b"MJPG")?; // biCompression
        self.file
            .write_all(&(self.width * self.height * 3).to_le_bytes())?; // biSizeImage
        self.file.write_all(&0_i32.to_le_bytes())?; // biXPelsPerMeter
        self.file.write_all(&0_i32.to_le_bytes())?; // biYPelsPerMeter
        self.file.write_all(&0_u32.to_le_bytes())?; // biClrUsed
        self.file.write_all(&0_u32.to_le_bytes())?; // biClrImportant

        // Pad until movi list
        let current = self.file.stream_position()?;
        let target_movi_header = self.movi_start_pos.saturating_sub(12);
        if current < target_movi_header {
            let pad_size = (target_movi_header - current) as u32;
            self.file.write_all(b"JUNK")?;
            self.file
                .write_all(&(pad_size.saturating_sub(8)).to_le_bytes())?;
            let zeros = vec![0_u8; (pad_size.saturating_sub(8)) as usize];
            self.file.write_all(&zeros)?;
        }

        // movi LIST header
        self.file.seek(SeekFrom::Start(target_movi_header))?;
        self.file.write_all(b"LIST")?;
        self.file.write_all(&(movi_size + 4).to_le_bytes())?;
        self.file.write_all(b"movi")?;

        Ok(())
    }
}

impl Drop for AviMjpegWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::{Seek, SeekFrom, Write},
        time::SystemTime,
    };

    use crate::capabilities::FrameRate;

    use super::{is_iriscope_avi, recover_partial_avi, recover_partial_avi_staged};

    #[test]
    fn staged_recovery_preserves_source_without_publishing_a_final_capture() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("iriscope-recovery-staged-{unique}"));
        let source = directory.join(".iriscope-recording.part");
        fs::create_dir_all(&directory).expect("directory");
        let mut writer =
            super::AviMjpegWriter::create(&source, 640, 480, FrameRate::new(8, 1).expect("rate"))
                .expect("writer");
        writer
            .write_frame(&[0xff, 0xd8, 0xff, 0xd9])
            .expect("frame");
        writer.file.sync_all().expect("sync");
        writer.is_closed = true;
        drop(writer);
        let staged = recover_partial_avi_staged(&source, &directory)
            .expect("recover staging")
            .expect("complete frame");
        assert!(source.exists());
        assert!(
            staged
                .extension()
                .is_some_and(|extension| extension == "part")
        );
        assert!(is_iriscope_avi(&staged).expect("probe complete staging"));
        assert!(fs::read_dir(&directory).expect("scan").all(|entry| {
            entry
                .expect("entry")
                .path()
                .extension()
                .is_none_or(|extension| extension != "avi")
        }),);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn provisional_header_allows_recovery_of_complete_frames() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("iriscope-recovery-{unique}"));
        std::fs::create_dir_all(&dir).expect("directory");
        let source = dir.join(".iriscope-recording.part");
        let rate = FrameRate::new(8, 1).expect("rate");
        let mut writer = super::AviMjpegWriter::create(&source, 640, 480, rate).expect("writer");
        writer
            .write_frame(&[0xff, 0xd8, 0xff, 0xd9])
            .expect("frame");
        writer.file.sync_all().expect("sync");
        writer.is_closed = true; // simulate a process exit without finish()
        drop(writer);
        let mut append = OpenOptions::new()
            .append(true)
            .open(&source)
            .expect("append");
        append
            .write_all(b"00dc\x20\x00\x00\x00partial")
            .expect("partial final frame");
        drop(append);
        let recovered = recover_partial_avi(&source, &dir, "recovered.avi")
            .expect("recover")
            .expect("complete frame exists");
        assert!(source.exists());
        assert!(is_iriscope_avi(&recovered).expect("probe"));
        let mut reader = super::AviMjpegReader::open(recovered).expect("open recovered");
        assert_eq!(reader.frame_count(), 1);
        assert_eq!(
            reader.read_frame(0).expect("frame"),
            [0xff, 0xd8, 0xff, 0xd9]
        );
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn recovery_skips_corrupt_complete_jpeg_chunks() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("iriscope-recovery-corrupt-{unique}"));
        fs::create_dir_all(&dir).expect("directory");
        let source = dir.join(".iriscope-recording.part");
        let rate = FrameRate::new(30_000, 1_001).expect("rate");
        let mut writer = super::AviMjpegWriter::create(&source, 640, 480, rate).expect("writer");
        writer
            .write_frame(&[0xff, 0xd8, 0x01, 0xff, 0xd9])
            .expect("first");
        writer.write_frame(b"broken frame").expect("corrupt chunk");
        writer
            .write_frame(&[0xff, 0xd8, 0x02, 0xff, 0xd9])
            .expect("third");
        writer.file.sync_all().expect("sync");
        writer.is_closed = true;
        drop(writer);

        let recovered = recover_partial_avi(&source, &dir, "recovered.avi")
            .expect("recover")
            .expect("two good frames");
        let mut reader = super::AviMjpegReader::open(recovered).expect("open recovered");
        assert_eq!(reader.frame_count(), 2);
        assert_eq!(reader.frame_rate(), rate);
        assert_eq!(
            reader.read_frame(0).expect("first"),
            [0xff, 0xd8, 0x01, 0xff, 0xd9]
        );
        assert_eq!(
            reader.read_frame(1).expect("second"),
            [0xff, 0xd8, 0x02, 0xff, 0xd9]
        );
        assert!(source.exists());
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn probe_rejects_noncanonical_or_incomplete_avi() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope-probe-{unique}.avi"));
        let rate = FrameRate::new(8, 1).expect("rate");
        let mut writer = super::AviMjpegWriter::create(&path, 640, 480, rate).expect("writer");
        writer
            .write_frame(&[0xff, 0xd8, 0xff, 0xd9])
            .expect("frame");
        writer.finish().expect("finish");
        drop(writer);
        assert!(is_iriscope_avi(&path).expect("valid probe"));

        let mut file = OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open header");
        file.seek(SeekFrom::Start(96)).expect("seek stream list");
        file.write_all(b"othr").expect("change stream list");
        drop(file);
        assert!(!is_iriscope_avi(&path).expect("foreign probe"));
        assert!(super::AviMjpegReader::open(&path).is_err());
        fs::remove_file(path).expect("cleanup");
    }

    use super::{AviMjpegReader, AviMjpegWriter, checked_frame_capacity};

    #[test]
    fn rejects_frame_before_classic_avi_size_fields_overflow() {
        let header = 2_048_u64;
        assert!(checked_frame_capacity(header, header, 1, 0).is_ok());
        // Account for the next frame, its alignment byte, and the growing idx1
        // index before accepting any bytes into the file.
        let near_limit = u64::from(u32::MAX) - 16;
        assert!(checked_frame_capacity(near_limit, header, 32, 1).is_err());
        assert!(checked_frame_capacity(header, header, usize::MAX, 0).is_err());
        assert!(checked_frame_capacity(header, header, 32 * 1024 * 1024 + 1, 0).is_err());
    }

    #[test]
    fn rejects_dimensions_that_truncate_avi_header_fields() {
        let path = std::env::temp_dir().join("iriscope_invalid_video_dimensions.avi");
        let rate = FrameRate::new(1, 1).expect("valid rate");
        assert!(AviMjpegWriter::create(&path, 65_536, 10, rate).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn reader_rejects_excessive_dimensions_and_frame_payloads() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("iriscope_oversized_avi_{unique}.avi"));
        let rate = FrameRate::new(30, 1).expect("valid rate");
        let mut writer = AviMjpegWriter::create(&path, 640, 480, rate).expect("create AVI");
        writer
            .write_frame(&[0xff, 0xd8, 0xff, 0xd9])
            .expect("write frame");
        writer.finish().expect("finish AVI");
        drop(writer);

        let mut file = OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open AVI header");
        file.seek(SeekFrom::Start(64)).expect("seek width");
        file.write_all(&16_384_u32.to_le_bytes())
            .expect("write width");
        drop(file);
        assert!(AviMjpegReader::open(&path).is_err());

        let mut file = OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open AVI header");
        file.seek(SeekFrom::Start(64)).expect("seek width");
        file.write_all(&640_u32.to_le_bytes())
            .expect("restore width");
        file.seek(SeekFrom::Start(2_052)).expect("seek frame size");
        file.write_all(&(super::MAX_AVI_FRAME_BYTES + 1).to_le_bytes())
            .expect("write oversized size");
        drop(file);
        assert!(AviMjpegReader::open(&path).is_err());
        fs::remove_file(path).expect("remove AVI");
    }

    #[test]
    fn creates_and_finishes_avi_file() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock is valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("test_video_{unique}.avi"));

        let frame_rate = FrameRate::new(25, 4).expect("valid test frame rate");
        let mut writer =
            AviMjpegWriter::create(&path, 1280, 1024, frame_rate).expect("create test AVI");
        let fake_jpeg = [0xff, 0xd8, 0xff, 0xd9];
        writer.write_frame(&fake_jpeg).expect("write frame 1");
        writer.write_frame(&fake_jpeg).expect("write frame 2");
        assert_eq!(writer.frame_count(), 2);
        writer.finish().expect("finish AVI");

        let bytes = fs::read(&path).expect("read test AVI");
        assert!(bytes.starts_with(b"RIFF"));
        assert!(&bytes[8..12] == b"AVI ");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn updated_rate_is_written_to_final_avi_headers() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock is valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("test_video_rate_{unique}.avi"));
        let requested = FrameRate::new(30, 1).expect("valid requested rate");
        let measured = FrameRate::new(25, 4).expect("valid measured rate");
        {
            let mut writer =
                AviMjpegWriter::create(&path, 640, 480, requested).expect("create AVI");
            let fake_jpeg = [0xff, 0xd8, 0xff, 0xd9];
            writer.write_frame(&fake_jpeg).expect("write frame 1");
            writer.write_frame(&fake_jpeg).expect("write frame 2");
            writer.set_frame_rate(measured);
            writer.finish().expect("finish AVI");
        }
        let reader = AviMjpegReader::open(&path).expect("open finalized AVI");
        assert_eq!(reader.frame_rate(), measured);
        let _ = fs::remove_file(path);
    }
    #[test]
    fn reader_indexes_and_reads_writer_frames() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock is valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("test_video_reader_{unique}.avi"));

        let frame_rate = FrameRate::new(25, 4).expect("valid test frame rate");
        let first = [0xff, 0xd8, 0x01, 0xff, 0xd9];
        let second = [0xff, 0xd8, 0x02, 0xff, 0xd9];

        {
            let mut writer =
                AviMjpegWriter::create(&path, 1280, 1024, frame_rate).expect("create test AVI");
            writer.write_frame(&first).expect("write first frame");
            writer.write_frame(&second).expect("write second frame");
            writer.finish().expect("finish AVI");
        }

        let mut reader = AviMjpegReader::open(&path).expect("open AVI");
        assert_eq!(reader.frame_count(), 2);
        assert_eq!(reader.frame_rate(), frame_rate);
        assert_eq!(reader.read_frame(0).expect("read first frame"), first);
        assert_eq!(reader.read_frame(1).expect("read second frame"), second);

        let _ = fs::remove_file(path);
    }
}
