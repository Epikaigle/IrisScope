#!/usr/bin/env python3
"""Prepare signed updater archives/metadata. The private key never enters Git."""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def package_macos_update(archive: Path, output: Path):
    """Repack the already signed portable bundle, preserving its bytes and modes."""
    with tempfile.TemporaryDirectory(prefix="iriscope-update-pack-") as directory:
        staging = Path(directory)
        with zipfile.ZipFile(archive) as portable:
            for info in portable.infolist():
                path = Path(info.filename)
                if path.is_absolute() or ".." in path.parts or path.parts[0] != "IrisScope.app":
                    raise SystemExit("Unsafe bundle path")
                destination = staging / path
                if info.is_dir():
                    destination.mkdir(parents=True, exist_ok=True)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes(portable.read(info))
                    destination.chmod((info.external_attr >> 16) & 0o777 or 0o644)
        for directory in staging.rglob("*"):
            if directory.is_dir():
                directory.chmod(0o755)
        with tarfile.open(output, "w:gz") as update:
            update.add(staging / "IrisScope.app", arcname="IrisScope.app")


def create_manifest(directory: Path, version: str, key_text: str):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives import serialization
    key = Ed25519PrivateKey.from_private_bytes(base64.b64decode(key_text.strip(), validate=True))
    public = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
    expected = (ROOT / "crates/iriscope-updater/public-key.hex").read_text().strip()
    if public != expected:
        raise SystemExit("Update key does not match the public key embedded in IrisScope")
    definitions = [
        ("macos", "x86_64", "macos-bundle", f"IrisScope-{version}-macos-x86_64-update.tar.gz"),
        ("linux", "x86_64", "linux-portable", f"IrisScope-{version}-linux-x86_64.tar.gz"),
        ("linux", "x86_64", "linux-deb", f"IrisScope-{version}-amd64.deb"),
    ]
    assets = []
    for system, architecture, kind, name in definitions:
        path = directory / name
        if not path.is_file():
            raise SystemExit(f"Release incomplete: {name}")
        with path.open("rb") as file:
            digest = hashlib.file_digest(file, "sha256").hexdigest()
        assets.append(dict(platform=system, architecture=architecture, kind=kind,
                           name=name, size=path.stat().st_size, sha256=digest))
    metadata = (json.dumps(dict(schema=1, version=version, assets=assets), indent=2) + "\n").encode()
    (directory / "update-manifest.json").write_bytes(metadata)
    (directory / "update-manifest.sig").write_bytes(key.sign(metadata))
    print(f"Signed update manifest: {version}, {len(assets)} platform packages")


def main():
    spec = importlib.util.spec_from_file_location("portable", ROOT / "scripts/package-release.py")
    portable = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(portable)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=ROOT / "dist")
    parser.add_argument("--macos-archive", type=Path)
    parser.add_argument("--manifest", action="store_true")
    parser.add_argument("--key-file", type=Path, help="Local protected key; CI uses IRISCOPE_UPDATE_PRIVATE_KEY")
    args = parser.parse_args()
    version = portable.release_version()
    if args.macos_archive:
        output = args.directory / f"IrisScope-{version}-macos-x86_64-update.tar.gz"
        package_macos_update(args.macos_archive, output)
        digest = hashlib.sha256(output.read_bytes()).hexdigest()
        output.with_name(output.name + ".sha256").write_text(f"{digest}  {output.name}\n")
        print(output)
    if args.manifest:
        key = args.key_file.read_text() if args.key_file else os.environ.get("IRISCOPE_UPDATE_PRIVATE_KEY", "")
        if not key:
            raise SystemExit("Missing IRISCOPE_UPDATE_PRIVATE_KEY")
        create_manifest(args.directory, version, key)
    if not args.macos_archive and not args.manifest:
        parser.error("Choose --macos-archive and/or --manifest")


if __name__ == "__main__":
    main()
