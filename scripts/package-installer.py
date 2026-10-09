#!/usr/bin/env python3
"""Create native installers; optional signing uses certificates already installed on the host.

Linux: dpkg-deb. Windows: Inno Setup 6 (ISCC) and optionally signtool.
macOS: hdiutil; optionally codesign and notarytool with an existing keychain profile.
No signing secrets are written to the package or command logs.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    subprocess.run([str(value) for value in args], check=True)


def metadata():
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def portable_module():
    spec = importlib.util.spec_from_file_location("portable", ROOT / "scripts/package-release.py")
    portable = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(portable)
    return portable


def stage_resources(stage, binary, system, version, ffmpeg=None, license_path=None):
    portable = portable_module()
    for name, data in portable.shared_files():
        path = stage / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    (stage / "release-info.json").write_bytes(portable.release_info(binary, system, version, ffmpeg))
    if ffmpeg:
        (stage / "licenses/FFmpeg-COPYING.txt").write_bytes(license_path.read_bytes())


def install_linux(stage, binary, output, version, ffmpeg=None, license_path=None):
    architecture = {"x86_64": "amd64", "aarch64": "arm64"}.get(platform.machine().lower())
    if architecture is None:
        raise SystemExit("Unsupported Debian architecture")
    executable = stage / "usr/bin/iriscope"
    executable.parent.mkdir(parents=True)
    shutil.copy2(binary, executable)
    executable.chmod(0o755)
    executables = {executable}
    updater = stage / "usr/lib/iriscope/iriscope-updater"
    updater.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(portable_module().updater_binary(), updater)
    executables.add(updater)
    if ffmpeg:
        encoder = stage / "usr/lib/iriscope/ffmpeg"
        encoder.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ffmpeg, encoder)
        executables.add(encoder)
    applications = stage / "usr/share/applications"
    applications.mkdir(parents=True)
    (applications / "iriscope.desktop").write_text("[Desktop Entry]\nType=Application\nName=Iriscope\nComment=Capture et consultation des images de l’iris\nExec=iriscope\nIcon=iriscope\nStartupWMClass=iriscope\nTerminal=false\nCategories=Graphics;Photography;\n", encoding="utf-8")
    icons = stage / "usr/share/icons/hicolor/scalable/apps"
    icons.mkdir(parents=True)
    shutil.copy2(ROOT / "assets/icons/iriscope.svg", icons / "iriscope.svg")
    docs = stage / "usr/share/doc/iriscope"
    docs.mkdir(parents=True)
    stage_resources(docs, binary, "linux", version, ffmpeg, license_path)
    control = stage / "DEBIAN"
    control.mkdir()
    (control / "control").write_text(f"Package: iriscope\nVersion: {version}\nArchitecture: {architecture}\nMaintainer: IrisScope contributors\nSection: graphics\nPriority: optional\nDepends: libc6 (>= 2.39), libgcc-s1, libudev1, libfontconfig1, libx11-6, libx11-xcb1, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libgl1, curl, pkexec\nRecommends: xdg-desktop-portal | zenity, ffmpeg\nDescription: Capture et consultation des images de l’iris\n Application locale pour la caméra Firefly DE400.\n", encoding="utf-8")
    # Package modes must be independent of the developer's restrictive umask.
    stage.chmod(0o755)
    for path in stage.rglob("*"):
        path.chmod(0o755 if path.is_dir() or path in executables else 0o644)
    artifact = output / f"IrisScope-{version}-{architecture}.deb"
    run("dpkg-deb", "--build", "--root-owner-group", stage, artifact)
    return artifact


def install_windows(stage, binary, output, version, thumbprint, iscc, ffmpeg=None, license_path=None):
    executable = stage / "IrisScope.exe"
    shutil.copy2(binary, executable)
    if thumbprint:
        run("signtool", "sign", "/sha1", thumbprint, "/fd", "SHA256", "/tr", "http://timestamp.digicert.com", "/td", "SHA256", executable)
    stage_resources(stage, executable, "windows", version, ffmpeg, license_path)
    if ffmpeg:
        shutil.copy2(ffmpeg, stage / "ffmpeg.exe")
    # Inno Setup quotes literals with doubled quote characters.
    quote = lambda path: str(path).replace('"', '""')
    script = stage / "IrisScope.iss"
    script.write_text(f'''[Setup]
AppId=app.iriscope.IrisScope
AppName=Iriscope
AppVersion={version}
DefaultDirName={{localappdata}}\\Programs\\IrisScope
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DefaultGroupName=IrisScope
OutputDir={quote(output)}
OutputBaseFilename=IrisScope-{version}-Setup
Compression=lzma2
SolidCompression=yes
UninstallDisplayIcon={{app}}\\IrisScope.exe
WizardStyle=modern
[Files]
Source: "{quote(stage / '*')}"; DestDir: "{{app}}"; Excludes: "IrisScope.iss"; Flags: ignoreversion recursesubdirs createallsubdirs
[Icons]
Name: "{{group}}\\IrisScope"; Filename: "{{app}}\\IrisScope.exe"
Name: "{{userdesktop}}\\IrisScope"; Filename: "{{app}}\\IrisScope.exe"; Tasks: desktopicon
[Tasks]
Name: "desktopicon"; Description: "Créer un raccourci sur le bureau"; Flags: unchecked
''', encoding="utf-8-sig")
    run(iscc, script)
    artifact = output / f"IrisScope-{version}-Setup.exe"
    if thumbprint:
        run("signtool", "sign", "/sha1", thumbprint, "/fd", "SHA256", "/tr", "http://timestamp.digicert.com", "/td", "SHA256", artifact)
        run("signtool", "verify", "/pa", artifact)
    return artifact


def install_macos(stage, binary, output, version, identity, notary_profile, ffmpeg=None, license_path=None, usb_dir=None):
    portable = portable_module()
    if notary_profile and (not identity or identity == "-"):
        raise SystemExit("Notarization requires an existing Developer ID signing identity; ad hoc signing is local only")
    bundle = portable.stage_macos(stage, binary, version, ffmpeg, license_path, usb_dir)
    if identity:
        portable.sign_macos(bundle, identity)
    if notary_profile:
        notarize_zip = output / f"IrisScope-{version}-notarize.zip"
        run("ditto", "-c", "-k", "--keepParent", bundle, notarize_zip)
        run("xcrun", "notarytool", "submit", notarize_zip, "--keychain-profile", notary_profile, "--wait")
        run("xcrun", "stapler", "staple", bundle)
        run("spctl", "--assess", "--type", "execute", "--verbose", bundle)
        notarize_zip.unlink()
    (stage / "Applications").symlink_to("/Applications")
    artifact = output / f"IrisScope-{version}-macOS-Intel.dmg"
    run("hdiutil", "create", "-volname", "IrisScope", "-srcfolder", stage, "-ov", "-format", "UDZO", artifact)
    return artifact


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release" / ("iriscope-app.exe" if os.name == "nt" else "iriscope-app"))
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--windows-certificate", help="Thumbprint of a certificate already installed in the Windows user store")
    parser.add_argument("--iscc", default="ISCC", help="Inno Setup compiler path")
    parser.add_argument("--signing-identity", default="-", help="Mac identity; '-' uses free local ad hoc signing")
    parser.add_argument("--macos-usb-dir", type=Path, default=ROOT / "target/macos-button-research")
    parser.add_argument("--notary-profile", help="Existing notarytool keychain profile")
    parser.add_argument("--ffmpeg", type=Path, help="Optional standalone native FFmpeg executable with libx264")
    parser.add_argument("--ffmpeg-license", type=Path, help="License and attribution file for the provided FFmpeg build")
    args = parser.parse_args()
    args.binary = args.binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    portable = portable_module()
    portable.validate_binary_version(args.binary, metadata())
    portable.validate_encoder(args.ffmpeg, args.ffmpeg_license)
    with tempfile.TemporaryDirectory(prefix="iriscope-installer-") as temporary:
        stage = Path(temporary)
        if platform.system() == "Linux":
            artifact = install_linux(stage, args.binary, args.output, metadata(), args.ffmpeg, args.ffmpeg_license)
        elif platform.system() == "Windows":
            artifact = install_windows(stage, args.binary, args.output, metadata(), args.windows_certificate, args.iscc, args.ffmpeg, args.ffmpeg_license)
        elif platform.system() == "Darwin":
            artifact = install_macos(stage, args.binary, args.output, metadata(), args.signing_identity, args.notary_profile, args.ffmpeg, args.ffmpeg_license, args.macos_usb_dir)
        else:
            raise SystemExit("Unsupported installer host")
    import hashlib
    with artifact.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    artifact.with_name(artifact.name + ".sha256").write_text(f"{digest}  {artifact.name}\n", encoding="ascii")
    print(artifact)


if __name__ == "__main__":
    main()
