use crate::Result;
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) fn require_success(status: u16) -> Result<()> {
    if status != 200 {
        return Err(format!("Le serveur de mise à jour répond HTTP {status}").into());
    }
    Ok(())
}

pub(crate) fn fetch(
    url: &str,
    destination: &Path,
    maximum: u64,
    seconds: u32,
    cancelled: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<u16> {
    let response_path = destination.with_extension("http-status");
    let errors_path = destination.with_extension("curl-errors");
    let mut child = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            &seconds.to_string(),
            "--max-filesize",
            &maximum.to_string(),
            "--user-agent",
            "IrisScope-Updater",
            "--write-out",
            "%{http_code}",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .stdout(Stdio::from(File::create(&response_path)?))
        .stderr(Stdio::from(File::create(&errors_path)?))
        .spawn()
        .map_err(|error| format!("Impossible de lancer le téléchargement (curl) : {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds) + 5);
    let status = loop {
        if cancelled.load(Ordering::Acquire) || Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Téléchargement annulé ou délai dépassé".into());
        }
        let size = std::fs::metadata(destination).map_or(0, |metadata| metadata.len());
        if size > maximum {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Fichier de mise à jour trop volumineux".into());
        }
        progress(size);
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    if !status.success() {
        return Err(
            "Téléchargement impossible. Vérifiez la connexion Internet puis réessayez.".into(),
        );
    }
    Ok(std::fs::read_to_string(response_path)?.parse()?)
}
