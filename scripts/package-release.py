#!/usr/bin/env python3
"""Build portable IrisScope archives from a native release executable.

Run on each target operating system after `cargo build --release -p iriscope-app`.
Only Python's standard library is required.
"""

import argparse
import hashlib
import io
import platform
import plistlib
import shutil
import sys
import tarfile
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
        ("scripts/diagnose-de400-button.py", (ROOT / "scripts/diagnose-de400-button.py").read_bytes()),
    ] + [
        (f"licenses/{path.name}", path.read_bytes()) for path in FONT_LICENSES
    ]


def package_windows(output: Path, binary: Path, folder: str) -> None:
    with zipfile.ZipFile(output, "w") as archive:
        add_zip_file(archive, f"{folder}/IrisScope.exe", binary)
        for name, data in shared_files():
            add_zip_bytes(archive, f"{folder}/{name}", data)


def package_linux(output: Path, binary: Path, folder: str) -> None:
    with tarfile.open(output, "w:gz") as archive:
        add_tar_file(archive, f"{folder}/iriscope-app", binary)
        for name, data in shared_files():
            add_tar_bytes(archive, f"{folder}/{name}", data)


def package_macos(output: Path, binary: Path, version: str) -> None:
    bundle = "IrisScope.app/Contents"
    info = {
        "CFBundleDevelopmentRegion": "fr",
        "CFBundleDisplayName": "IrisScope",
        "CFBundleExecutable": "IrisScope",
        "CFBundleIdentifier": "app.iriscope.IrisScope",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": "IrisScope",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSApplicationCategoryType": "public.app-category.photography",
        "NSCameraUsageDescription": "IrisScope utilise la caméra Firefly DE400 pour afficher et enregistrer les images de l'iris.",
        "NSHighResolutionCapable": True,
    }
    with zipfile.ZipFile(output, "w") as archive:
        add_zip_bytes(archive, f"{bundle}/Info.plist", plistlib.dumps(info))
        add_zip_file(archive, f"{bundle}/MacOS/IrisScope", binary)
        for name, data in shared_files():
            add_zip_bytes(archive, f"{bundle}/Resources/{name}", data)


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
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error(f"release executable not found: {args.binary}")

    architecture = host_architecture()
    folder = f"IrisScope-{version}-{system}-{architecture}"
    suffix = ".tar.gz" if system == "linux" else ".zip"
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / f"{folder}{suffix}"
    if system == "linux":
        package_linux(output, args.binary, folder)
    elif system == "windows":
        package_windows(output, args.binary, folder)
    else:
        package_macos(output, args.binary, version)

    with output.open("rb") as archive:
        digest = hashlib.file_digest(archive, "sha256").hexdigest()
    checksum = output.with_name(output.name + ".sha256")
    checksum.write_text(f"{digest}  {output.name}\n", encoding="ascii")
    print(output)
    print(checksum)


if __name__ == "__main__":
    main()
