//! Direct lossless Motion-JPEG AVI video recording.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::capabilities::FrameRate;

/// Lightweight reader for MJPEG AVI files produced by IrisScope.
///
/// Only compressed-frame offsets are kept in memory. JPEG payloads are read lazily.
pub struct AviMjpegReader {
    file: File,
    frame_rate: FrameRate,
    frames: Vec<(u64, u32)>,
}

impl AviMjpegReader {
    /// Opens an IrisScope MJPEG AVI and indexes its video frames.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file is invalid, truncated, or cannot be read.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < 2_048 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AVI file is smaller than the IrisScope header",
            ));
        }

        let mut header = vec![0_u8; 2_048];
        file.read_exact(&mut header)?;
        if &header[0..4] != b"RIFF" || &header[8..12] != b"AVI " {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an AVI RIFF file",
            ));
        }

        let microseconds_per_frame =
            u32::from_le_bytes(header[32..36].try_into().expect("four-byte AVI timing field"));
        let frame_rate = FrameRate::new(1_000_000, microseconds_per_frame).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid AVI frame timing")
        })?;

        let mut frames = Vec::new();
        let mut position = 2_048_u64;

        while position.saturating_add(8) <= file_len {
            file.seek(SeekFrom::Start(position))?;
            let mut chunk_header = [0_u8; 8];
            file.read_exact(&mut chunk_header)?;

            let chunk_id = &chunk_header[..4];
            let size =
                u32::from_le_bytes(chunk_header[4..8].try_into().expect("four-byte chunk size"));

            if chunk_id == b"idx1" {
                break;
            }

            let payload_offset = position.saturating_add(8);
            let payload_end = payload_offset.saturating_add(u64::from(size));
            if payload_end > file_len {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated AVI chunk",
                ));
            }

            if chunk_id == b"00dc" {
                frames.push((payload_offset, size));
            }

            position = payload_end.saturating_add(u64::from(size % 2));
        }

        if frames.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AVI contains no MJPEG frames",
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

    /// Reads one compressed JPEG frame.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the frame index is invalid or the payload cannot be read.
    pub fn read_frame(&mut self, index: usize) -> io::Result<Vec<u8>> {
        let &(offset, size) = self.frames.get(index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "AVI frame index out of range")
        })?;

        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes =
            vec![0_u8; usize::try_from(size).expect("u32 AVI frame size always fits usize")];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
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
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;

        // Reserve space for RIFF header (2048 bytes header placeholder)
        let placeholder = vec![0_u8; 2048];
        file.write_all(&placeholder)?;

        let movi_start_pos = file.stream_position()?;

        Ok(Self {
            file,
            width,
            height,
            frame_rate,
            frame_count: 0,
            movi_start_pos,
            index_entries: Vec::new(),
            is_closed: false,
        })
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
        if Path::new(file_name)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(file_name)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "video capture name must not contain a directory",
            ));
        }

        fs::create_dir_all(directory)?;
        let requested = Path::new(file_name);
        let stem = requested
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Iris");
        let extension = requested.extension().and_then(|value| value.to_str());

        for collision_index in 1_u32.. {
            let candidate_name = if collision_index == 1 {
                file_name.to_owned()
            } else if let Some(extension) = extension {
                format!("{stem}_{collision_index}.{extension}")
            } else {
                format!("{stem}_{collision_index}")
            };
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
    /// Returns an I/O error if writing to disk fails.
    pub fn write_frame(&mut self, jpeg_bytes: &[u8]) -> io::Result<()> {
        let current_pos = self.file.stream_position()?;
        let offset_from_movi = (current_pos.saturating_sub(self.movi_start_pos)) as u32;
        let size = jpeg_bytes.len() as u32;

        // Chunk ID: '00dc' (video compressed frame)
        self.file.write_all(b"00dc")?;
        self.file.write_all(&size.to_le_bytes())?;
        self.file.write_all(jpeg_bytes)?;

        // AVI chunks must be 2-byte aligned
        if size % 2 != 0 {
            self.file.write_all(&[0])?;
        }

        self.index_entries.push((offset_from_movi, size));
        self.frame_count = self.frame_count.saturating_add(1);
        Ok(())
    }

    /// Returns the number of frames recorded so far.
    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frame_count
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

        // Write index 'idx1'
        let idx_pos = self.file.stream_position()?;
        self.file.write_all(b"idx1")?;
        let idx_size = (self.index_entries.len() * 16) as u32;
        self.file.write_all(&idx_size.to_le_bytes())?;

        for &(offset, size) in &self.index_entries {
            self.file.write_all(b"00dc")?;
            self.file.write_all(&0x10_u32.to_le_bytes())?; // AVIIF_KEYFRAME
            self.file.write_all(&offset.to_le_bytes())?;
            self.file.write_all(&size.to_le_bytes())?;
        }

        let total_file_size = self.file.stream_position()?;
        let movi_size = (idx_pos.saturating_sub(self.movi_start_pos)) as u32;

        // Rewind and write real header
        self.file.seek(SeekFrom::Start(0))?;
        self.write_avi_headers(total_file_size, movi_size)?;

        self.file.flush()?;
        self.is_closed = true;
        Ok(())
    }

    fn write_avi_headers(&mut self, total_file_size: u64, movi_size: u32) -> io::Result<()> {
        let riff_size = (total_file_size.saturating_sub(8)) as u32;
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
    use std::{fs, time::SystemTime};

    use crate::capabilities::FrameRate;

    use super::{AviMjpegReader, AviMjpegWriter};

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
