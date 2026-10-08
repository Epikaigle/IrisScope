#!/usr/bin/env python3
"""Build portable IrisScope archives from a native release executable.

Run on each target operating system after `cargo build --release -p iriscope-app`.
Only Python's standard library is required.
"""

import argparse
import hashlib
import io
import json
import platform
import plistlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
FONT_LICENSES = (
    ROOT / "ui/fonts/OFL-Manrope.txt",
    ROOT / "ui/fonts/OFL-InstrumentSerif.txt",
)


def host_platform() -> str:
    return {"linux": "linux", "win32": "windows", "darwin": "macos"}.get(
        sys.platform, "unsupported"
    )


def host_architecture() -> str:
    machine = platform.machine().lower()
    return {"amd64": "x86_64", "x64": "x86_64", "arm64": "aarch64"}.get(
        machine, machine
    )


def add_zip_bytes(archive: zipfile.ZipFile, name: str, data: bytes, mode: int = 0o644) -> None:
    entry = zipfile.ZipInfo(name)
    entry.create_system = 3
    entry.external_attr = (0o100000 | mode) << 16
    entry.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(entry, data)


def add_tar_bytes(archive: tarfile.TarFile, name: str, data: bytes, mode: int = 0o644) -> None:
    entry = tarfile.TarInfo(name)
    entry.size = len(data)
    entry.mode = mode
    archive.addfile(entry, io.BytesIO(data))


def add_zip_file(archive: zipfile.ZipFile, name: str, source: Path) -> None:
    entry = zipfile.ZipInfo(name)
    entry.create_system = 3
    entry.external_attr = (0o100000 | 0o755) << 16
    entry.compress_type = zipfile.ZIP_DEFLATED
    with source.open("rb") as binary:
        entry.file_size = source.stat().st_size
        with archive.open(entry, "w", force_zip64=True) as destination:
            shutil.copyfileobj(binary, destination, length=1024 * 1024)


def add_tar_file(archive: tarfile.TarFile, name: str, source: Path) -> None:
    entry = tarfile.TarInfo(name)
    entry.size = source.stat().st_size
    entry.mode = 0o755
    with source.open("rb") as binary:
        archive.addfile(entry, binary)


def shared_files() -> list[tuple[str, bytes]]:
    return [
        ("README.md", (ROOT / "README.md").read_bytes()),
        ("CORRECTIONS.md", (ROOT / "CORRECTIONS.md").read_bytes()),
        ("VERSION.txt", (release_version() + "\n").encode("ascii")),
        ("documentation/RELEASE-0.2.0.md", (ROOT / "documentation/RELEASE-0.2.0.md").read_bytes()),
        ("documentation/RELEASE-0.3.0.md", (ROOT / "documentation/RELEASE-0.3.0.md").read_bytes()),
        ("documentation/RELEASE-0.4.0.md", (ROOT / "documentation/RELEASE-0.4.0.md").read_bytes()),
        ("documentation/VISIONNEUSE.md", (ROOT / "documentation/VISIONNEUSE.md").read_bytes()),
        ("documentation/VALIDATION-MATERIELLE.md", (ROOT / "documentation/VALIDATION-MATERIELLE.md").read_bytes()),
        ("documentation/VALIDATION-LINUX-2026-10-06.md", (ROOT / "documentation/VALIDATION-LINUX-2026-10-06.md").read_bytes()),
        ("documentation/VALIDATION-MACOS-2026-10-07.md", (ROOT / "documentation/VALIDATION-MACOS-2026-10-07.md").read_bytes()),
        ("documentation/VALIDATION-MACOS-2026-10-08.md", (ROOT / "documentation/VALIDATION-MACOS-2026-10-08.md").read_bytes()),
        ("documentation/VALIDATION-INTERFACE-MACOS-2026-10-08.md", (ROOT / "documentation/VALIDATION-INTERFACE-MACOS-2026-10-08.md").read_bytes()),
        ("documentation/BOUTON-MULTIPLATEFORME.md", (ROOT / "documentation/BOUTON-MULTIPLATEFORME.md").read_bytes()),
        ("documentation/ACCES-USB-MACOS.md", (ROOT / "documentation/ACCES-USB-MACOS.md").read_bytes()),
        ("documentation/ESSAIS-WINDOWS-MACOS.md", (ROOT / "documentation/ESSAIS-WINDOWS-MACOS.md").read_bytes()),
        ("documentation/VALIDATION-INTERFACE-2026-10-06.md", (ROOT / "documentation/VALIDATION-INTERFACE-2026-10-06.md").read_bytes()),
        ("scripts/diagnose-de400-button.py", (ROOT / "scripts/diagnose-de400-button.py").read_bytes()),
        ("scripts/diagnose-de400-usb-status.py", (ROOT / "scripts/diagnose-de400-usb-status.py").read_bytes()),
        ("scripts/install-de400-button.sh", (ROOT / "scripts/install-de400-button.sh").read_bytes()),
        ("scripts/uninstall-de400-button.sh", (ROOT / "scripts/uninstall-de400-button.sh").read_bytes()),
        ("helpers/linux/README.md", (ROOT / "helpers/linux/README.md").read_bytes()),
        ("helpers/linux/de400_button_bridge.py", (ROOT / "helpers/linux/de400_button_bridge.py").read_bytes()),
        ("helpers/linux/iriscope-button.service", (ROOT / "helpers/linux/iriscope-button.service").read_bytes()),
    ] + [
        (f"licenses/{path.name}", path.read_bytes()) for path in FONT_LICENSES
    ]


def release_version() -> str:
    return tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]


def validate_binary_version(binary: Path, version: str) -> None:
    result = subprocess.run([str(binary.resolve()), "--version"], capture_output=True, text=True, timeout=10, check=True)
    if result.stdout.strip() != f"IrisScope {version}":
        raise SystemExit(f"Executable version does not match {version}: {result.stdout.strip()}")


def validate_encoder(ffmpeg: Path | None, license_path: Path | None) -> None:
    if bool(ffmpeg) != bool(license_path):
        raise SystemExit("Bundling FFmpeg requires both --ffmpeg and --ffmpeg-license")
    for path in [ffmpeg, license_path]:
        if path is not None and not path.is_file():
            raise SystemExit(f"Required FFmpeg resource not found: {path}")


def release_info(binary: Path, system: str, version: str, ffmpeg: Path | None = None) -> bytes:
    with binary.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    data = {"application": "IrisScope", "version": version, "platform": system,
            "architecture": host_architecture(), "input_executable_sha256": digest,
            "mp4_export": "bundled-ffmpeg" if ffmpeg else "optional-system-ffmpeg"}
    return (json.dumps(data, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def package_windows(output: Path, binary: Path, folder: str, ffmpeg: Path | None = None, license_path: Path | None = None) -> None:
    with zipfile.ZipFile(output, "w") as archive:
        add_zip_file(archive, f"{folder}/IrisScope.exe", binary)
        for name, data in shared_files():
            add_zip_bytes(archive, f"{folder}/{name}", data)
        add_zip_bytes(archive, f"{folder}/release-info.json", release_info(binary, "windows", release_version(), ffmpeg))
        if ffmpeg:
            add_zip_file(archive, f"{folder}/ffmpeg.exe", ffmpeg)
            add_zip_bytes(archive, f"{folder}/licenses/FFmpeg-COPYING.txt", license_path.read_bytes())


def package_linux(output: Path, binary: Path, folder: str, ffmpeg: Path | None = None, license_path: Path | None = None) -> None:
    with tarfile.open(output, "w:gz") as archive:
        add_tar_file(archive, f"{folder}/iriscope-app", binary)
        for name, data in shared_files():
            add_tar_bytes(archive, f"{folder}/{name}", data)
        add_tar_bytes(archive, f"{folder}/release-info.json", release_info(binary, "linux", release_version(), ffmpeg))
        if ffmpeg:
            add_tar_file(archive, f"{folder}/ffmpeg", ffmpeg)
            add_tar_bytes(archive, f"{folder}/licenses/FFmpeg-COPYING.txt", license_path.read_bytes())


def stage_macos(stage: Path, binary: Path, version: str, ffmpeg: Path | None = None,
                license_path: Path | None = None, usb_dir: Path | None = None) -> Path:
    bundle = stage / "IrisScope.app"
    contents = bundle / "Contents"
    macos = contents / "MacOS"
    resources = contents / "Resources"
    macos.mkdir(parents=True)
    resources.mkdir()
    info = {
        "CFBundleDevelopmentRegion": "fr",
        "CFBundleDisplayName": "Iriscope",
        "CFBundleExecutable": "IrisScope",
        "CFBundleIdentifier": "app.iriscope.IrisScope",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "Iriscope",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSApplicationCategoryType": "public.app-category.photography",
        "NSCameraUsageDescription": "Iriscope utilise la caméra Firefly DE400 pour afficher et enregistrer les images de l'iris.",
        "NSHighResolutionCapable": True,
    }
    (contents / "Info.plist").write_bytes(plistlib.dumps(info))
    shutil.copy2(binary, macos / ("IrisScopeGui" if usb_dir else "IrisScope"))
    if usb_dir:
        # The USB libraries/reader target macOS 15; the XPC API alone needs 13.
        info["LSMinimumSystemVersion"] = "15.0"
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        for source, name in [("iriscope-launcher", "IrisScope"), ("de400-usb-helper", "de400-usb-helper"),
                             ("iriscope-usb-service", "iriscope-usb-service")]:
            shutil.copy2(usb_dir / source, macos / name)
        if not (usb_dir / "redistribution/NOTICE.txt").is_file():
            raise SystemExit("USB redistribution sources/licenses missing; rebuild scripts/macos/build-usb-experiment.py")
        shutil.copytree(usb_dir / "redistribution", resources / "USB-sources")
    for name, data in shared_files():
        destination = resources / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    details = json.loads(release_info(binary, "macos", version, ffmpeg))
    details["de400_usb"] = "installed-on-demand-video-button-controls" if usb_dir else "avfoundation-only"
    (resources / "release-info.json").write_text(json.dumps(details, indent=2) + "\n", encoding="utf-8")
    if ffmpeg:
        shutil.copy2(ffmpeg, macos / "ffmpeg")
        (resources / "licenses/FFmpeg-COPYING.txt").write_bytes(license_path.read_bytes())
    for path in [stage, bundle, *bundle.rglob("*")]:
        path.chmod(0o755 if path.is_dir() or path.parent == macos else 0o644)
    return bundle


def sign_macos(bundle: Path, identity: str) -> None:
    # Sign nested native programs first. Ad hoc signing is the free local default.
    with tempfile.TemporaryDirectory(prefix="iriscope-signing-") as temporary:
        entitlements = Path(temporary) / "camera.plist"
        entitlements.write_bytes(plistlib.dumps({"com.apple.security.device.camera": True}))
        options = [] if identity == "-" else ["--options", "runtime", "--timestamp"]
        for executable in sorted((bundle / "Contents/MacOS").iterdir()):
            # codesign treats the main executable as the whole enclosing bundle.
            # Sign it with the bundle only after every auxiliary program.
            if executable.name == "IrisScope":
                continue
            camera = ["--entitlements", str(entitlements)] if identity != "-" and executable.name in {"IrisScope", "IrisScopeGui"} else []
            subprocess.run(["codesign", "--force", *options, *camera, "--sign", identity, str(executable)], check=True)
        camera = [] if identity == "-" else ["--entitlements", str(entitlements)]
        subprocess.run(["codesign", "--force", *options, *camera, "--sign", identity, str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)


def package_macos(output: Path, binary: Path, version: str, ffmpeg: Path | None = None,
                  license_path: Path | None = None, usb_dir: Path | None = None,
                  signing_identity: str | None = None) -> None:
    with tempfile.TemporaryDirectory(prefix="iriscope-bundle-") as temporary:
        bundle = stage_macos(Path(temporary), binary, version, ffmpeg, license_path, usb_dir)
        if signing_identity:
            sign_macos(bundle, signing_identity)
        with zipfile.ZipFile(output, "w") as archive:
            for path in sorted(bundle.rglob("*")):
                if path.is_file():
                    name = str(path.relative_to(bundle.parent))
                    add_zip_bytes(archive, name, path.read_bytes(), path.stat().st_mode & 0o777)


def main() -> None:
    with (ROOT / "Cargo.toml").open("rb") as manifest:
        version = tomllib.load(manifest)["workspace"]["package"]["version"]
    system = host_platform()
    if system == "unsupported":
        raise SystemExit(f"Unsupported packaging host: {sys.platform}")

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        type=Path,
        default=ROOT / "target/release" / ("iriscope-app.exe" if system == "windows" else "iriscope-app"),
        help="Path to the executable built on this operating system",
    )
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist")
    parser.add_argument("--ffmpeg", type=Path, help="Optional standalone native FFmpeg executable with libx264")
    parser.add_argument("--ffmpeg-license", type=Path, help="License and attribution file for the provided FFmpeg build")
    parser.add_argument("--macos-usb-dir", type=Path, default=ROOT / "target/macos-button-research",
                        help="Built native Mac launcher/helper and redistribution sources")
    parser.add_argument("--signing-identity", default="-", help="Mac codesign identity; '-' uses free local ad hoc signing")
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error(f"release executable not found: {args.binary}")
    validate_binary_version(args.binary, version)
    validate_encoder(args.ffmpeg, args.ffmpeg_license)

    architecture = host_architecture()
    folder = f"IrisScope-{version}-{system}-{architecture}"
    suffix = ".tar.gz" if system == "linux" else ".zip"
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / f"{folder}{suffix}"
    if system == "linux":
        package_linux(output, args.binary, folder, args.ffmpeg, args.ffmpeg_license)
    elif system == "windows":
        package_windows(output, args.binary, folder, args.ffmpeg, args.ffmpeg_license)
    else:
        package_macos(output, args.binary, version, args.ffmpeg, args.ffmpeg_license,
                      args.macos_usb_dir, args.signing_identity)

    with output.open("rb") as archive:
        digest = hashlib.file_digest(archive, "sha256").hexdigest()
    checksum = output.with_name(output.name + ".sha256")
    checksum.write_text(f"{digest}  {output.name}\n", encoding="ascii")
    print(output)
    print(checksum)


if __name__ == "__main__":
    main()
