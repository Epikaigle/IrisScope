use crate::{Release, Result, validate_version, verify_archive, verify_manifest};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstallTarget {
    path: PathBuf,
    kind: String,
}

impl InstallTarget {
    pub fn current() -> Result<Self> {
        Self::from_executable(&std::env::current_exe()?)
    }

    pub fn from_executable(executable: &Path) -> Result<Self> {
        let executable = executable.canonicalize()?;
        if cfg!(target_os = "macos") {
            let bundle = executable
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .ok_or("Application macOS incomplète")?;
            if bundle
                .file_name()
                .is_some_and(|name| name == "IrisScope.app")
                && bundle.join("Contents/Info.plist").is_file()
            {
                return Ok(Self {
                    path: bundle.to_path_buf(),
                    kind: "macos-bundle".into(),
                });
            }
        } else if cfg!(target_os = "linux") {
            if executable == Path::new("/usr/bin/iriscope") {
                return Ok(Self {
                    path: executable,
                    kind: "linux-deb".into(),
                });
            }
            let folder = executable
                .parent()
                .ok_or("Dossier d’application introuvable")?;
            if executable
                .file_name()
                .is_some_and(|name| name == "iriscope-app")
                && folder.join("release-info.json").is_file()
            {
                return Ok(Self {
                    path: folder.to_path_buf(),
                    kind: "linux-portable".into(),
                });
            }
        }
        Err("Les mises à jour intégrées nécessitent le paquet installé macOS ou Linux.".into())
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        self.kind != "linux-deb"
            && path
                .canonicalize()
                .unwrap_or_else(|_| path.to_path_buf())
                .starts_with(&self.path)
    }

    fn updater(&self) -> PathBuf {
        match self.kind.as_str() {
            "macos-bundle" => self.path.join("Contents/MacOS/iriscope-updater"),
            "linux-deb" => PathBuf::from("/usr/lib/iriscope/iriscope-updater"),
            _ => self.path.join("iriscope-updater"),
        }
    }
}

pub struct ReadyUpdate {
    directory: tempfile::TempDir,
    release: Release,
}

impl ReadyUpdate {
    pub(crate) fn new(directory: tempfile::TempDir, release: Release) -> Self {
        Self { directory, release }
    }

    /// Called only after the UI and capture workers have shut down cleanly.
    pub fn launch(self, target: &InstallTarget) -> Result<()> {
        let helper = target.updater();
        self.launch_with_helper(target, &helper, launcher_pid())
    }

    pub(crate) fn launch_with_helper(
        self,
        target: &InstallTarget,
        source: &Path,
        launcher_pid: u32,
    ) -> Result<()> {
        let helper = self.directory.path().join("installer");
        std::fs::copy(source, &helper)?;
        executable_permissions(&helper)?;
        let plan = Plan {
            target: target.clone(),
            version: self.release.version,
            current_pid: std::process::id(),
            // The Mac launcher must release its instance lock and USB session too.
            launcher_pid,
        };
        let plan_path = self.directory.path().join("plan.json");
        std::fs::write(&plan_path, serde_json::to_vec(&plan)?)?;
        let log = File::create(self.directory.path().join("installation.log"))?;
        Command::new(helper)
            .arg("--apply")
            .arg(plan_path)
            // The old GUI may have been started inside the folder we replace.
            .current_dir(self.directory.path())
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        // The detached installer owns cleanup after the old process exits.
        let _ = self.directory.keep();
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct Plan {
    target: InstallTarget,
    version: String,
    current_pid: u32,
    launcher_pid: u32,
}

fn executable_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn launcher_pid() -> u32 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: getppid has no arguments or memory safety preconditions.
        u32::try_from(unsafe { libc::getppid() }).unwrap_or(0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

fn wait_for_process(pid: u32) -> Result<()> {
    if pid == 0 {
        return Ok(());
    }
    let pid = i32::try_from(pid)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        #[cfg(unix)]
        {
            // SAFETY: signal 0 queries existence and sends no signal.
            if unsafe { libc::kill(pid, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err("L’application ne s’est pas fermée ; installation annulée.".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub(crate) fn unpack(archive: &Path, destination: &Path) -> Result<()> {
    let mut tar = tar::Archive::new(GzDecoder::new(File::open(archive)?));
    let mut total = 0_u64;
    for (index, entry) in tar.entries()?.enumerate() {
        if index >= 20_000 {
            return Err("Archive trop complexe".into());
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("Chemin interdit dans l’archive".into());
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err("Liens et fichiers spéciaux interdits dans l’archive".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("Archive trop volumineuse")?;
        if total > 1024 * 1024 * 1024 {
            return Err("Archive trop volumineuse".into());
        }
        let executable = entry.header().mode()? & 0o111 != 0;
        if !entry.unpack_in(destination)? {
            return Err("Chemin sortant du dossier d’installation".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                destination.join(&path),
                std::fs::Permissions::from_mode(if kind.is_dir() || executable {
                    0o755
                } else {
                    0o644
                }),
            )?;
        }
    }
    Ok(())
}

fn replace_and_launch(
    source: &Path,
    target: &Path,
    launch: impl FnOnce() -> Result<()>,
) -> Result<PathBuf> {
    if std::fs::symlink_metadata(target)?.file_type().is_symlink() {
        return Err("Application cible sous forme de lien".into());
    }
    let parent = target
        .parent()
        .ok_or("Dossier d’installation introuvable")?;
    let backup = tempfile::Builder::new()
        .prefix(".iriscope-previous-")
        .tempdir_in(parent)?;
    let old = backup
        .path()
        .join(target.file_name().ok_or("Nom d’application invalide")?);
    std::fs::rename(target, &old)?;
    if let Err(error) = std::fs::rename(source, target) {
        std::fs::rename(&old, target)?;
        return Err(error.into());
    }
    if let Err(error) = launch() {
        std::fs::rename(target, source)?;
        std::fs::rename(&old, target)?;
        return Err(error);
    }
    Ok(backup.keep())
}

fn launch_command(target: &InstallTarget, receipt: &Path) -> Command {
    let mut command = if target.kind == "macos-bundle" {
        let mut command = Command::new("/usr/bin/open");
        command
            .arg("-n")
            .arg(&target.path)
            .arg("--env")
            .arg(format!("IRISCOPE_UPDATE_RECEIPT={}", receipt.display()));
        command
    } else {
        Command::new(if target.kind == "linux-deb" {
            target.path.clone()
        } else {
            target.path.join("iriscope-app")
        })
    };
    // `open` inherits its caller's environment. The old launcher has already
    // closed this USB socket, so the restarted launcher must create a new one.
    command.env_remove("IRISCOPE_DE400_SOCKET");
    command.env_remove("IRISCOPE_EXPERIMENTAL_DE400_SOCKET");
    command.env_remove("IRISCOPE_UPDATE_RECEIPT");
    if target.kind != "macos-bundle" {
        command.env("IRISCOPE_UPDATE_RECEIPT", receipt);
    }
    // Renaming and deleting the previous application invalidates an inherited
    // working directory inside it. Use the new folder (or its stable parent).
    command.current_dir(if target.kind == "linux-portable" {
        &target.path
    } else {
        target.path.parent().unwrap_or(Path::new("/"))
    });
    command
}

fn launch_application(target: &InstallTarget, receipt: &Path) -> Result<()> {
    let mut command = launch_command(target, receipt);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if target.kind == "macos-bundle" {
        if !command.status()?.success() {
            return Err("Impossible de relancer IrisScope".into());
        }
    } else {
        command.spawn()?;
    }
    Ok(())
}

/// The restarted GUI acknowledges only after its window has been created.
pub fn acknowledge_start(version: &str) -> Result<()> {
    let Some(path) = std::env::var_os("IRISCOPE_UPDATE_RECEIPT") else {
        return Ok(());
    };
    let path = PathBuf::from(path);
    if path.file_name().is_none_or(|name| name != "started")
        || path
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|name| !name.to_string_lossy().starts_with("iriscope-update-"))
    {
        return Err("Accusé de mise à jour invalide".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    writeln!(file, "IrisScope {version}")?;
    Ok(())
}

/// Revalidates the signed package in the detached installer before replacement.
#[allow(clippy::too_many_lines)] // Keep verification, replacement and acknowledgement in order.
pub fn apply_plan(plan_path: &Path) -> Result<()> {
    let directory = plan_path.parent().ok_or("Plan de mise à jour incomplet")?;
    let plan: Plan = serde_json::from_slice(&std::fs::read(plan_path)?)?;
    let manifest = verify_manifest(
        &std::fs::read(directory.join("manifest.json"))?,
        &std::fs::read(directory.join("manifest.sig"))?,
    )?;
    if manifest.version != plan.version {
        return Err("Version du plan invalide".into());
    }
    let asset = manifest
        .assets
        .iter()
        .find(|asset| {
            asset.platform == std::env::consts::OS
                && asset.architecture == std::env::consts::ARCH
                && asset.kind == plan.target.kind
        })
        .ok_or("Paquet incompatible avec ce système")?;
    let archive = directory.join(&asset.name);
    verify_archive(&archive, asset)?;
    let installed_binary = match plan.target.kind.as_str() {
        "macos-bundle" => plan.target.path.join("Contents/MacOS/IrisScopeGui"),
        "linux-deb" => plan.target.path.clone(),
        _ => plan.target.path.join("iriscope-app"),
    };
    let installed = Command::new(installed_binary).arg("--version").output()?;
    let installed_text = String::from_utf8(installed.stdout)?;
    let installed_version = installed_text
        .trim()
        .strip_prefix("IrisScope ")
        .ok_or("Version installée inconnue")?;
    if !installed.status.success()
        || semver::Version::parse(&plan.version)? <= semver::Version::parse(installed_version)?
    {
        return Err(
            "La nouvelle version doit être plus récente que l’application installée".into(),
        );
    }
    wait_for_process(plan.current_pid)?;
    wait_for_process(plan.launcher_pid)?;
    let receipt = directory.join("started");
    let mut backup = None;
    if plan.target.kind == "linux-deb" {
        if plan.target.path != Path::new("/usr/bin/iriscope") {
            return Err("Installation Debian invalide".into());
        }
        for (field, expected) in [("Package", "iriscope"), ("Version", plan.version.as_str())] {
            let value = Command::new("dpkg-deb")
                .args(["--field"])
                .arg(&archive)
                .arg(field)
                .output()?;
            if !value.status.success() || String::from_utf8_lossy(&value.stdout).trim() != expected
            {
                return Err("Métadonnées Debian incompatibles".into());
            }
        }
        if !Command::new("pkexec")
            .args(["/usr/bin/dpkg", "--install"])
            .arg(archive)
            .status()?
            .success()
        {
            return Err("Installation annulée ou refusée par le système".into());
        }
        validate_version(&plan.target.path, &plan.version)?;
        launch_application(&plan.target, &receipt)?;
    } else {
        let parent = plan
            .target
            .path
            .parent()
            .ok_or("Dossier d’installation introuvable")?;
        let stage = tempfile::Builder::new().prefix(".iriscope-install-").tempdir_in(parent)
            .map_err(|_| "IrisScope ne peut pas écrire dans son dossier. Déplacez l’application dans un dossier personnel puis réessayez.")?;
        unpack(&archive, stage.path())?;
        let name = if plan.target.kind == "macos-bundle" {
            "IrisScope.app".to_owned()
        } else if plan.target.kind == "linux-portable" {
            format!(
                "IrisScope-{}-linux-{}",
                plan.version,
                std::env::consts::ARCH
            )
        } else {
            return Err("Format d’installation non pris en charge".into());
        };
        let source = stage.path().join(name);
        let binary = if plan.target.kind == "macos-bundle" {
            if plan
                .target
                .path
                .file_name()
                .is_none_or(|name| name != "IrisScope.app")
            {
                return Err("Application cible invalide".into());
            }
            if !Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&source)
                .status()?
                .success()
            {
                return Err("Signature macOS du paquet invalide".into());
            }
            source.join("Contents/MacOS/IrisScopeGui")
        } else {
            source.join("iriscope-app")
        };
        validate_version(&binary, &plan.version)?;
        backup = Some(replace_and_launch(&source, &plan.target.path, || {
            launch_application(&plan.target, &receipt)
        })?);
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if std::fs::read_to_string(&receipt)
            .is_ok_and(|value| value.trim() == format!("IrisScope {}", plan.version))
        {
            if let Some(backup) = backup
                && let Err(error) = std::fs::remove_dir_all(backup)
            {
                eprintln!("Ancienne application conservée : {error}");
            }
            if let Err(error) = std::fs::remove_dir_all(directory) {
                eprintln!("Cache de mise à jour conservé : {error}");
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "Le démarrage n’a pas été confirmé. L’ancienne version est conservée : {}",
        backup.map_or_else(
            || "paquet système".into(),
            |path| path.display().to_string()
        )
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn portable_restart_uses_the_replacement_folder_after_backup_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let target = InstallTarget {
            path: directory.path().join("IrisScope"),
            kind: "linux-portable".into(),
        };
        let source = directory.path().join("new");
        std::fs::create_dir(&target.path).unwrap();
        std::fs::create_dir(&source).unwrap();
        let binary = source.join("iriscope-app");
        std::fs::write(&binary, "#!/bin/sh\npwd -P\n").unwrap();
        executable_permissions(&binary).unwrap();
        let receipt = directory.path().join("started");
        let check_directory = || -> Result<()> {
            let output = launch_command(&target, &receipt).output()?;
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout)?.trim(),
                target.path.canonicalize()?.to_str().unwrap()
            );
            Ok(())
        };
        let backup = replace_and_launch(&source, &target.path, check_directory).unwrap();
        std::fs::remove_dir_all(backup).unwrap();
        check_directory().unwrap();
    }

    #[test]
    fn restart_cannot_reuse_the_previous_usb_session() {
        let target = InstallTarget {
            path: PathBuf::from("/Applications/IrisScope.app"),
            kind: "macos-bundle".into(),
        };
        let command = launch_command(&target, Path::new("/tmp/iriscope-update-test/started"));
        for name in [
            "IRISCOPE_DE400_SOCKET",
            "IRISCOPE_EXPERIMENTAL_DE400_SOCKET",
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none())
            );
        }
        assert!(
            command
                .get_args()
                .any(|arg| arg == "IRISCOPE_UPDATE_RECEIPT=/tmp/iriscope-update-test/started")
        );
    }

    #[test]
    fn failed_restart_restores_previous_application() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&new).unwrap();
        std::fs::write(old.join("version"), "old").unwrap();
        std::fs::write(new.join("version"), "new").unwrap();
        assert!(replace_and_launch(&new, &old, || Err("restart failure".into())).is_err());
        assert_eq!(std::fs::read_to_string(old.join("version")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(new.join("version")).unwrap(), "new");
    }
}
