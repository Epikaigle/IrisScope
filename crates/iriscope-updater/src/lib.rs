//! Signed GitHub updates, independent of the camera and graphical toolkit.
#![allow(clippy::missing_errors_doc)]

mod install;
mod transport;

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub use install::{InstallTarget, ReadyUpdate, acknowledge_start, apply_plan};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub const REPOSITORY: &str = "Epikaigle/iriscope-app";
const PUBLIC_KEY: &str = include_str!("../public-key.hex");
const MAX_ARCHIVE_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    pub platform: String,
    pub architecture: String,
    pub kind: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub asset: Asset,
    manifest: Vec<u8>,
    signature: Vec<u8>,
}

fn hex_bytes<const N: usize>(value: &str) -> Result<[u8; N]> {
    let value = value.trim();
    if value.len() != N * 2 || !value.is_ascii() {
        return Err("Empreinte ou clé invalide".into());
    }
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)?;
    }
    Ok(output)
}

fn verify_manifest_with_key(bytes: &[u8], signature: &[u8], key: &[u8; 32]) -> Result<Manifest> {
    if bytes.len() > 64 * 1024 {
        return Err("Manifeste trop volumineux".into());
    }
    let signature = Signature::from_slice(signature)?;
    VerifyingKey::from_bytes(key)?.verify_strict(bytes, &signature)?;
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    let version = semver::Version::parse(&manifest.version)?;
    if manifest.schema != 1 || !version.pre.is_empty() || !version.build.is_empty() {
        return Err("Version de mise à jour non prise en charge".into());
    }
    let mut names = std::collections::HashSet::new();
    for asset in &manifest.assets {
        if !names.insert(&asset.name)
            || asset.name.is_empty()
            || !asset
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            || asset.name.starts_with('.')
            || !(1..=MAX_ARCHIVE_BYTES).contains(&asset.size)
        {
            return Err("Fichier de mise à jour invalide".into());
        }
        hex_bytes::<32>(&asset.sha256)?;
    }
    Ok(manifest)
}

fn verify_manifest(bytes: &[u8], signature: &[u8]) -> Result<Manifest> {
    verify_manifest_with_key(bytes, signature, &hex_bytes(PUBLIC_KEY)?)
}

fn download_url(version: &str, name: &str) -> String {
    format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}")
}

fn select_release(
    bytes: Vec<u8>,
    signature: Vec<u8>,
    current: &str,
    tag: &str,
    target: &InstallTarget,
) -> Result<Option<Release>> {
    let manifest = verify_manifest(&bytes, &signature)?;
    select_verified_manifest(manifest, bytes, signature, current, tag, target)
}

fn select_verified_manifest(
    manifest: Manifest,
    bytes: Vec<u8>,
    signature: Vec<u8>,
    current: &str,
    tag: &str,
    target: &InstallTarget,
) -> Result<Option<Release>> {
    if tag != format!("v{}", manifest.version) {
        return Err("Le tag ne correspond pas à la version signée".into());
    }
    if semver::Version::parse(&manifest.version)? <= semver::Version::parse(current)? {
        return Ok(None);
    }
    let asset = manifest
        .assets
        .iter()
        .find(|asset| {
            asset.platform == std::env::consts::OS
                && asset.architecture == std::env::consts::ARCH
                && asset.kind == target.kind()
        })
        .ok_or("Aucun paquet compatible avec cette installation")?
        .clone();
    Ok(Some(Release {
        version: manifest.version,
        asset,
        manifest: bytes,
        signature,
    }))
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

/// Queries only stable releases; an offline check does not affect capture.
pub fn check(
    current: &str,
    target: &InstallTarget,
    cancelled: &AtomicBool,
) -> Result<Option<Release>> {
    let directory = tempfile::Builder::new()
        .prefix("iriscope-check-")
        .tempdir()?;
    let api = directory.path().join("release.json");
    let status = transport::fetch(
        &format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
        &api,
        1024 * 1024,
        30,
        cancelled,
        &|_| {},
    )?;
    if status == 404 {
        return Ok(None);
    }
    transport::require_success(status)?;
    let release: GitHubRelease = serde_json::from_slice(&std::fs::read(api)?)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .ok_or("Tag de version invalide")?;
    // Do not construct a URL from arbitrary remote text.
    semver::Version::parse(version)?;
    let manifest_path = directory.path().join("manifest.json");
    let signature_path = directory.path().join("manifest.sig");
    for (name, path, maximum) in [
        ("update-manifest.json", &manifest_path, 64 * 1024),
        ("update-manifest.sig", &signature_path, 64),
    ] {
        let status = transport::fetch(
            &download_url(version, name),
            path,
            maximum,
            30,
            cancelled,
            &|_| {},
        )?;
        transport::require_success(status)?;
    }
    select_release(
        std::fs::read(manifest_path)?,
        std::fs::read(signature_path)?,
        current,
        &release.tag_name,
        target,
    )
}

fn verify_archive(path: &Path, asset: &Asset) -> Result<()> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != asset.size {
        return Err("Téléchargement incomplet".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    if digest.finalize().as_slice() != hex_bytes::<32>(&asset.sha256)? {
        return Err("Le fichier téléchargé ne correspond pas à la version signée".into());
    }
    Ok(())
}

/// Downloads and checks every byte before offering installation.
pub fn download(
    release: &Release,
    cancelled: &AtomicBool,
    progress: &dyn Fn(u8),
) -> Result<ReadyUpdate> {
    let directory = tempfile::Builder::new()
        .prefix("iriscope-update-")
        .tempdir()?;
    let archive = directory.path().join(&release.asset.name);
    let status = transport::fetch(
        &download_url(&release.version, &release.asset.name),
        &archive,
        release.asset.size,
        600,
        cancelled,
        &|size| {
            let percent = (size.saturating_mul(100) / release.asset.size).min(100);
            progress(u8::try_from(percent).unwrap_or(100));
        },
    )?;
    transport::require_success(status)?;
    verify_archive(&archive, &release.asset)?;
    std::fs::write(directory.path().join("manifest.json"), &release.manifest)?;
    std::fs::write(directory.path().join("manifest.sig"), &release.signature)?;
    Ok(ReadyUpdate::new(directory, release.clone()))
}

/// Bootstrap a locally signed package through the same detached installer.
/// The signature, platform, version and package hash are checked before launch.
pub fn install_local_signed(
    executable: &Path,
    archive: &Path,
    metadata: &Path,
    signature: &Path,
) -> Result<()> {
    let target = InstallTarget::from_executable(executable)?;
    let bytes = std::fs::read(metadata)?;
    let signature = std::fs::read(signature)?;
    let manifest = verify_manifest(&bytes, &signature)?;
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()?;
    let output = String::from_utf8(output.stdout)?;
    let current = output
        .trim()
        .strip_prefix("IrisScope ")
        .ok_or("Version installée inconnue")?;
    let tag = format!("v{}", manifest.version);
    let release = select_verified_manifest(manifest, bytes, signature, current, &tag, &target)?
        .ok_or("La mise à jour locale doit être plus récente")?;
    verify_archive(archive, &release.asset)?;
    let directory = tempfile::Builder::new()
        .prefix("iriscope-update-")
        .tempdir()?;
    std::fs::copy(archive, directory.path().join(&release.asset.name))?;
    std::fs::write(directory.path().join("manifest.json"), &release.manifest)?;
    std::fs::write(directory.path().join("manifest.sig"), &release.signature)?;
    ReadyUpdate::new(directory, release).launch_with_helper(&target, &std::env::current_exe()?, 0)
}

fn validate_version(binary: &Path, version: &str) -> Result<()> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("IrisScope {version}")
    {
        return Err("La version du programme téléchargé ne correspond pas au manifeste".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
