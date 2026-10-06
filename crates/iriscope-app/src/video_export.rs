//! Optional H.264 MP4 exports. Encoding never modifies or replaces the source AVI.
use iriscope_core::{library::CaptureFileVersion, video::AviMjpegReader};
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn cancelled(cancel: &dyn Fn() -> bool) -> io::Result<()> {
    if cancel() {
        Err(io::Error::new(io::ErrorKind::Interrupted, "Export annulé."))
    } else {
        Ok(())
    }
}

struct StagedExport(PathBuf);
impl StagedExport {
    fn new(parent: &Path) -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let path = parent.join(format!(
            ".iriscope-export-{stamp}-{}.part.mp4",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?;
        Ok(Self(path))
    }
}
impl Drop for StagedExport {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
struct Encoder(Child);
impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A bundled `FFmpeg` takes priority over the optional system installation.
fn encoder_path() -> PathBuf {
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let bundled = parent.join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        if bundled.is_file() {
            return bundled;
        }
        #[cfg(target_os = "linux")]
        if let Some(base) = parent.parent() {
            let installed = base.join("lib/iriscope/ffmpeg");
            if installed.is_file() {
                return installed;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

#[derive(Debug)]
struct MissingEncoder;
impl std::fmt::Display for MissingEncoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("L’export MP4 nécessite FFmpeg. Installez-le puis relancez l’export. La vidéo originale est conservée.")
    }
}
impl std::error::Error for MissingEncoder {}

pub(super) fn encoder_is_missing(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(<dyn std::error::Error + Send + Sync>::is::<MissingEncoder>)
}

fn start_encoder(program: &Path, source: &Path, staging: &Path) -> io::Result<Encoder> {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .args([
            "-nostdin",
            "-xerror",
            "-hide_banner",
            "-loglevel",
            "error",
            "-progress",
            "pipe:1",
            "-nostats",
            "-threads",
            "2",
            "-err_detect",
            "explode",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-threads",
            "2",
            "-map_metadata",
            "-1",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(staging)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().map(Encoder).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::new(error.kind(), MissingEncoder)
        } else {
            error
        }
    })
}
fn collect_errors(mut stream: impl Read) -> String {
    let mut tail = Vec::new();
    let mut buffer = [0_u8; 2048];
    while let Ok(count) = stream.read(&mut buffer) {
        if count == 0 {
            break;
        }
        tail.extend_from_slice(&buffer[..count]);
        if tail.len() > 8192 {
            tail.drain(..tail.len() - 8192);
        }
    }
    String::from_utf8_lossy(&tail).trim().to_owned()
}
fn encode(
    program: &Path,
    source: &Path,
    staging: &Path,
    duration_us: u128,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(i32),
) -> io::Result<()> {
    let mut encoder = start_encoder(program, source, staging)?;
    let elapsed = Arc::new(AtomicU64::new(0));
    let current = Arc::clone(&elapsed);
    let stdout = encoder
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Progression de l’export indisponible."))?;
    let stderr = encoder
        .0
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("Journal de l’export indisponible."))?;
    let output = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = line
                .strip_prefix("out_time_us=")
                .and_then(|value| value.parse().ok())
            {
                current.store(value, Ordering::Release);
            }
        }
    });
    let errors = thread::spawn(move || collect_errors(stderr));
    let result = loop {
        progress(
            i32::try_from(u128::from(elapsed.load(Ordering::Acquire)) * 99 / duration_us.max(1))
                .unwrap_or(99)
                .clamp(0, 99),
        );
        if let Err(error) = cancelled(cancel) {
            break Err(error);
        }
        match encoder.0.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("L’encodage MP4 a échoué."))
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(error) => break Err(error),
        }
    };
    drop(encoder); // Kill/reap on cancellation before joining pipe readers.
    let _ = output.join();
    let errors = errors.join().unwrap_or_default();
    result.map_err(|error| {
        if error.kind() == io::ErrorKind::Interrupted || errors.is_empty() {
            error
        } else {
            io::Error::other(format!("L’encodage MP4 a échoué : {errors}"))
        }
    })
}

pub(crate) fn export_mp4(
    source: &Path,
    expected: &CaptureFileVersion,
    destination: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(i32),
) -> io::Result<PathBuf> {
    export_with_encoder(
        source,
        expected,
        destination,
        &encoder_path(),
        cancel,
        progress,
    )
}
fn export_with_encoder(
    source: &Path,
    expected: &CaptureFileVersion,
    destination: &Path,
    program: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(i32),
) -> io::Result<PathBuf> {
    cancelled(cancel)?;
    if !destination
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choisissez un fichier avec l’extension .mp4.",
        ));
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Ce fichier existe déjà. Choisissez un autre nom ; aucun fichier n’a été remplacé.",
        ));
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent)?;
    let file = crate::playback::open_verified_capture_cancellable(source, expected, cancel)?;
    let mut reader = AviMjpegReader::from_file(file.try_clone()?)?;
    if reader.frame_count() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "La vidéo ne contient aucune image.",
        ));
    }
    let fps = reader.frame_rate();
    let duration_us = reader.frame_count() as u128 * 1_000_000 * u128::from(fps.denominator())
        / u128::from(fps.numerator());
    // Some FFmpeg builds silently skip a corrupt MJPEG packet even with
    // -xerror. Verify each original image before publishing a shortened export.
    for index in 0..reader.frame_count() {
        cancelled(cancel)?;
        let jpeg = reader.read_frame(index)?;
        iriscope_imaging::decode_mjpeg_to_rgb8(&jpeg).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "L’encodage MP4 est impossible : image {} invalide ({error}).",
                    index + 1
                ),
            )
        })?;
    }
    drop(reader);
    iriscope_core::disk_space::ensure_available_space(
        &parent,
        file.metadata()?.len().saturating_mul(2),
    )?;
    let staging = StagedExport::new(&parent)?;
    encode(program, source, &staging.0, duration_us, cancel, progress)?;
    cancelled(cancel)?;
    crate::playback::verify_open_capture_cancellable(&file, source, expected, cancel)?;
    // Windows requires a writable handle for FlushFileBuffers (File::sync_all).
    let output = OpenOptions::new().read(true).write(true).open(&staging.0)?;
    if output.metadata()?.len() == 0 {
        return Err(io::Error::other("L’export MP4 est vide."));
    }
    output.sync_all()?;
    drop(output);
    cancelled(cancel)?;
    fs::hard_link(&staging.0, destination)?; // Atomic publication; never overwrite a chosen file.
    #[cfg(unix)]
    if let Ok(directory) = fs::File::open(&parent) {
        let _ = directory.sync_all();
    }
    progress(100);
    Ok(destination.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iriscope_core::{
        capabilities::FrameRate, library::capture_file_version, video::AviMjpegWriter,
    };
    struct Fixture {
        root: PathBuf,
        source: PathBuf,
        expected: CaptureFileVersion,
        original: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "iriscope-mp4-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            let source = root.join("Émilie — œil gauche.avi");
            let jpeg =
                iriscope_imaging::encode_rgb8_jpeg(&vec![84_u8; 64 * 64 * 3], 64, 64, 85).unwrap();
            let mut video =
                AviMjpegWriter::create(&source, 64, 64, FrameRate::new(8, 1).unwrap()).unwrap();
            for _ in 0..16 {
                video.write_frame(&jpeg).unwrap();
            }
            video.finish().unwrap();
            let expected = capture_file_version(&source).unwrap();
            let original = fs::read(&source).unwrap();
            Self {
                root,
                source,
                expected,
                original,
            }
        }
        fn assert_original(&self) {
            assert_eq!(fs::read(&self.source).unwrap(), self.original);
        }
        fn assert_no_staging(&self) {
            assert!(fs::read_dir(&self.root).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".iriscope-export-")
            }));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn missing_encoder_and_existing_destination_preserve_the_original() {
        let fixture = Fixture::new();
        let target = fixture.root.join("copie.mp4");
        let missing = fixture.root.join("ffmpeg-absent");
        let error = export_with_encoder(
            &fixture.source,
            &fixture.expected,
            &target,
            &missing,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("FFmpeg"));
        assert!(encoder_is_missing(&error));
        assert!(!encoder_is_missing(&io::Error::new(
            io::ErrorKind::NotFound,
            "source video disappeared"
        )));
        assert!(!target.exists());
        fixture.assert_no_staging();
        fs::write(&target, b"existing export").unwrap();
        assert_eq!(
            export_with_encoder(
                &fixture.source,
                &fixture.expected,
                &target,
                &missing,
                &|| false,
                &mut |_| {}
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(target).unwrap(), b"existing export");
        fixture.assert_original();
    }
    #[test]
    fn changed_video_is_rejected_before_encoding() {
        let fixture = Fixture::new();
        fs::write(&fixture.source, b"changed video").unwrap();
        let target = fixture.root.join("copie.mp4");
        assert!(
            export_mp4(
                &fixture.source,
                &fixture.expected,
                &target,
                &|| false,
                &mut |_| {}
            )
            .is_err()
        );
        assert!(!target.exists());
        assert_eq!(fs::read(&fixture.source).unwrap(), b"changed video");
        fixture.assert_no_staging();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn real_ffmpeg_creates_readable_h264_with_the_recorded_duration() {
        let fixture = Fixture::new();
        let target = fixture.root.join("copie.mp4");
        let mut updates = Vec::new();
        export_mp4(
            &fixture.source,
            &fixture.expected,
            &target,
            &|| false,
            &mut |percent| {
                updates.push(percent);
            },
        )
        .unwrap();
        assert_eq!(updates.last(), Some(&100));
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height,nb_frames,duration",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(&target)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let details = String::from_utf8(probe.stdout).unwrap();
        for expected in [
            "codec_name=h264",
            "width=64",
            "height=64",
            "nb_frames=16",
            "duration=2.000000",
        ] {
            assert!(details.contains(expected), "{details}");
        }
        fixture.assert_original();
        fixture.assert_no_staging();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn corrupt_jpeg_frame_fails_without_publishing_a_partial_video() {
        let fixture = Fixture::new();
        let source = fixture.root.join("damaged.avi");
        let good =
            iriscope_imaging::encode_rgb8_jpeg(&vec![84_u8; 64 * 64 * 3], 64, 64, 85).unwrap();
        let mut video =
            AviMjpegWriter::create(&source, 64, 64, FrameRate::new(8, 1).unwrap()).unwrap();
        video.write_frame(&good).unwrap();
        video
            .write_frame(b"\xff\xd8invalid JPEG frame\xff\xd9")
            .unwrap();
        video.write_frame(&good).unwrap();
        video.finish().unwrap();
        drop(video);
        let original = fs::read(&source).unwrap();
        let expected = capture_file_version(&source).unwrap();
        let target = fixture.root.join("damaged.mp4");
        let error = export_mp4(&source, &expected, &target, &|| false, &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("encodage MP4"), "{error}");
        assert!(!target.exists());
        assert_eq!(fs::read(source).unwrap(), original);
        fixture.assert_no_staging();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn cancelling_real_encoder_removes_the_partial_export() {
        use std::cell::Cell;
        let fixture = Fixture::new();
        let target = fixture.root.join("annulé.mp4");
        let cancel = Cell::new(false);
        let result = export_mp4(
            &fixture.source,
            &fixture.expected,
            &target,
            &|| cancel.get(),
            &mut |_| {
                cancel.set(true);
            },
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(!target.exists());
        fixture.assert_original();
        fixture.assert_no_staging();
    }
}
