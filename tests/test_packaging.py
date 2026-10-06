"""Verify package contents, native identifiers, permissions and data boundaries."""
import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load_script(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PORTABLE = load_script("package-release")
INSTALLER = load_script("package-installer")


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="iriscope-package-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / "app"
        self.binary.write_bytes(b"synthetic native application")
        self.engine = self.root / "encoder"
        self.engine.write_bytes(b"synthetic native encoder")
        self.license = self.root / "COPYING"
        self.license.write_text("Test license and attribution", encoding="utf-8")
        self.output = self.root / "dist"
        self.output.mkdir()
        self.version = PORTABLE.release_version()

    def test_archives_contain_versioned_resources_and_encoder_permissions(self):
        archive = self.output / "linux.tar.gz"
        PORTABLE.package_linux(archive, self.binary, "IrisScope", self.engine, self.license)
        with tarfile.open(archive) as package:
            self.assertEqual(package.getmember("IrisScope/ffmpeg").mode, 0o755)
            self.assertEqual(package.getmember("IrisScope/iriscope-app").mode, 0o755)
            self.assertEqual(package.extractfile("IrisScope/VERSION.txt").read().decode().strip(), self.version)
            info = json.load(package.extractfile("IrisScope/release-info.json"))
            self.assertEqual(info["input_executable_sha256"], hashlib.sha256(self.binary.read_bytes()).hexdigest())
            self.assertEqual(info["mp4_export"], "bundled-ffmpeg")
            self.assertEqual(package.extractfile("IrisScope/licenses/FFmpeg-COPYING.txt").read(), self.license.read_bytes())
            self.assertFalse(any(name.startswith("/") or "settings.json" in name or ".iriscope-index" in name for name in package.getnames()))

    def test_windows_and_macos_archives_keep_native_identity_and_camera_permission(self):
        windows = self.output / "windows.zip"
        PORTABLE.package_windows(windows, self.binary, "IrisScope", self.engine, self.license)
        with zipfile.ZipFile(windows) as package:
            self.assertEqual(package.read("IrisScope/ffmpeg.exe"), self.engine.read_bytes())
            self.assertEqual(json.loads(package.read("IrisScope/release-info.json"))["version"], self.version)
        mac = self.output / "mac.zip"
        PORTABLE.package_macos(mac, self.binary, self.version, self.engine, self.license)
        with zipfile.ZipFile(mac) as package:
            info = plistlib.loads(package.read("IrisScope.app/Contents/Info.plist"))
            self.assertEqual(info["CFBundleIdentifier"], "app.iriscope.IrisScope")
            self.assertEqual(info["CFBundleShortVersionString"], self.version)
            self.assertTrue(info["NSCameraUsageDescription"])
            self.assertEqual(package.getinfo("IrisScope.app/Contents/MacOS/ffmpeg").external_attr >> 16 & 0o777, 0o755)

    def test_windows_installer_only_manages_the_application_directory(self):
        stage = self.root / "windows-stage"
        stage.mkdir()
        with patch.object(INSTALLER, "run") as compiler:
            INSTALLER.install_windows(stage, self.binary, self.output, self.version, None, "ISCC", self.engine, self.license)
        compiler.assert_called_once()
        script = (stage / "IrisScope.iss").read_text(encoding="utf-8-sig")
        self.assertIn("AppId=app.iriscope.IrisScope", script)
        self.assertIn("DefaultDirName={localappdata}\\Programs\\IrisScope", script)
        self.assertNotIn("[UninstallDelete]", script)
        self.assertNotIn("{userdocs}", script)
        self.assertNotIn("{userappdata}", script)
        self.assertEqual((stage / "ffmpeg.exe").read_bytes(), self.engine.read_bytes())
        self.assertTrue((stage / "docs/RELEASE-0.2.0.md").is_file())

    def test_macos_installer_restores_executable_modes_without_signing(self):
        stage = self.root / "mac-stage"
        stage.mkdir()
        with patch.object(INSTALLER, "run") as tools:
            INSTALLER.install_macos(stage, self.binary, self.output, self.version, None, None, self.engine, self.license)
        self.assertEqual(tools.call_count, 1)
        self.assertEqual(tools.call_args.args[0], "hdiutil")
        for name in ["IrisScope", "ffmpeg"]:
            self.assertEqual((stage / "IrisScope.app/Contents/MacOS" / name).stat().st_mode & 0o777, 0o755)
        self.assertEqual((stage / "Applications").readlink(), Path("/Applications"))

    @unittest.skipUnless(shutil.which("dpkg-deb"), "Debian packaging tools required")
    def test_real_debian_package_has_no_user_data_or_removal_hooks(self):
        stage = self.root / "linux-stage"
        stage.mkdir()
        artifact = INSTALLER.install_linux(stage, self.binary, self.output, self.version, self.engine, self.license)
        extracted = self.root / "extracted"
        subprocess.run(["dpkg-deb", "-x", str(artifact), str(extracted)], check=True)
        self.assertEqual((extracted / "usr/bin/iriscope").stat().st_mode & 0o777, 0o755)
        self.assertEqual((extracted / "usr/lib/iriscope/ffmpeg").stat().st_mode & 0o777, 0o755)
        self.assertEqual((extracted / "usr/share/doc/iriscope/VERSION.txt").read_text().strip(), self.version)
        self.assertEqual(sorted(path.name for path in extracted.iterdir()), ["usr"])
        self.assertEqual(sorted(path.name for path in (stage / "DEBIAN").iterdir()), ["control"])

    def test_bundled_encoder_requires_its_license(self):
        with self.assertRaises(SystemExit):
            PORTABLE.validate_encoder(self.engine, None)
        PORTABLE.validate_encoder(self.engine, self.license)


if __name__ == "__main__":
    unittest.main()
