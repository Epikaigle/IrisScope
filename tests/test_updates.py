"""Publication must fail closed and packages must carry authentic metadata."""
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def script(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PACKAGE = script("package-update")
PUBLISH = script("publish-release")


class ReleaseTests(unittest.TestCase):
    def test_publication_keeps_only_installers_and_signs_the_release_body(self):
        try:
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
            from cryptography.hazmat.primitives import serialization
        except ImportError:
            self.skipTest("cryptography required for publication signature test")
        key = Ed25519PrivateKey.generate()
        private = base64.b64encode(key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())).decode()
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "scripts").mkdir()
            shutil.copy2(ROOT / "scripts/package-update.py", root / "scripts/package-update.py")
            (root / "crates/iriscope-updater").mkdir(parents=True)
            (root / "crates/iriscope-updater/public-key.hex").write_text(key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex())
            (root / "documentation").mkdir()
            (root / "documentation/RELEASE-0.4.3.md").write_text("Release notes")
            names = ["IrisScope-0.4.3-macOS-Intel.dmg", "IrisScope-0.4.3-amd64.deb"]
            for name in names:
                (root / name).write_bytes(b"verified-package")
                (root / (name + ".sha256")).write_text(hashlib.sha256(b"verified-package").hexdigest() + "  " + name + "\n")
            release = dict(id=42, assets=[dict(id=43, name="draft.sha256")], html_url="https://github.com/test/release")
            with patch.object(PUBLISH, "ROOT", root), patch.object(PUBLISH, "metadata", return_value=("0.4.3", "abc")), patch.object(PUBLISH, "draft", return_value=release), patch.object(PUBLISH, "upload") as upload, patch.object(PUBLISH, "request", return_value=release) as request, patch.dict("os.environ", {"IRISCOPE_UPDATE_PRIVATE_KEY": private}), patch("sys.argv", ["publish-release.py", "--publish", "--directory", str(root)]):
                PUBLISH.main()
                self.assertEqual([call.args[1].name for call in upload.call_args_list], names)
                self.assertIn(unittest.mock.call("/releases/assets/43", "DELETE"), request.call_args_list)
                published = request.call_args_list[-1].args
                self.assertEqual(published[:2], ("/releases/42", "PATCH"))
                self.assertFalse(published[2]["draft"])
                body = published[2]["body"]
                envelope = json.loads(body.split("<!-- iriscope-update-v1\n", 1)[1].split("\n-->", 1)[0])
                key.public_key().verify(bytes.fromhex(envelope["signature"]), bytes.fromhex(envelope["manifest"]))
                self.assertEqual([asset["name"] for asset in json.loads(bytes.fromhex(envelope["manifest"]))["assets"]], names)

    def test_published_or_other_commit_release_cannot_be_overwritten(self):
        for release in [dict(tag_name="v0.4.1", draft=False, target_commitish="abc"),
                        dict(tag_name="v0.4.1", draft=True, target_commitish="other")]:
            with patch.object(PUBLISH, "request", return_value=[release]):
                with self.assertRaises(SystemExit):
                    PUBLISH.draft("0.4.1", "abc")

    def test_tag_must_match_the_package_version(self):
        with patch.dict("os.environ", {"GITHUB_REF": "refs/tags/v9.0.0"}):
            with self.assertRaises(SystemExit):
                PUBLISH.metadata()

    def test_macos_repack_refuses_paths_outside_the_application(self):
        with tempfile.TemporaryDirectory() as folder:
            archive = Path(folder) / "mac.zip"
            with zipfile.ZipFile(archive, "w") as file:
                file.writestr("../escape", b"unsafe")
            with self.assertRaises(SystemExit):
                PACKAGE.package_macos_update(archive, Path(folder) / "update.tar.gz")
            self.assertFalse((Path(folder).parent / "escape").exists())

    def test_checksums_are_checked_before_uploading(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "app").write_bytes(b"original")
            (root / "app.sha256").write_text(hashlib.sha256(b"original").hexdigest() + "  app\n")
            PUBLISH.verify_checksums(root, ["app", "app.sha256"])
            (root / "app").write_bytes(b"changed")
            with self.assertRaises(SystemExit):
                PUBLISH.verify_checksums(root, ["app", "app.sha256"])

    def test_signed_manifest_is_complete_and_authentic(self):
        try:
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
            from cryptography.hazmat.primitives import serialization
        except ImportError:
            self.skipTest("cryptography required for publication signature test")
        key = Ed25519PrivateKey.generate()
        private = base64.b64encode(key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())).decode()
        public = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            key_folder = root / "crates/iriscope-updater"
            key_folder.mkdir(parents=True)
            (key_folder / "public-key.hex").write_text(public)
            with patch.object(PACKAGE, "ROOT", root):
                with self.assertRaises(SystemExit):
                    PACKAGE.create_manifest(root, "0.4.1", private)
                for name in ["IrisScope-0.4.1-macOS-Intel.dmg", "IrisScope-0.4.1-amd64.deb"]:
                    (root / name).write_bytes(b"signed-package-test")
                PACKAGE.create_manifest(root, "0.4.1", private)
                metadata = (root / "update-manifest.json").read_bytes()
                key.public_key().verify((root / "update-manifest.sig").read_bytes(), metadata)
                self.assertEqual(len(json.loads(metadata)["assets"]), 2)
                body = PUBLISH.signed_release_body("Release notes", root)
                envelope = json.loads(body.split("<!-- iriscope-update-v1\n", 1)[1].split("\n-->", 1)[0])
                self.assertEqual(bytes.fromhex(envelope["manifest"]), metadata)
                key.public_key().verify(bytes.fromhex(envelope["signature"]), bytes.fromhex(envelope["manifest"]))
