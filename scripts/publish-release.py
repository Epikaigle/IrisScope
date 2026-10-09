#!/usr/bin/env python3
"""Publish tested packages atomically as one signed GitHub release."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "Epikaigle/iriscope-app"
API = f"https://api.github.com/repos/{REPOSITORY}"


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file, code, message, headers, url):
        redirected = super().redirect_request(request, file, code, message, headers, url)
        if redirected and urllib.parse.urlparse(url).hostname != "api.github.com":
            redirected.remove_header("Authorization")
        return redirected


def request(path, method="GET", body=None, binary=False):
    url = path if path.startswith("https://") else API + path
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if not token:
        raise SystemExit("Missing GITHUB_TOKEN")
    hostname = urllib.parse.urlparse(url).hostname
    if hostname not in ("api.github.com", "uploads.github.com"):
        raise SystemExit("GitHub authentication must not be sent to another host")
    headers = {"Authorization": f"Bearer {token}", "User-Agent": "IrisScope-release",
               "Accept": "application/octet-stream" if binary else "application/vnd.github+json"}
    if body is not None and not isinstance(body, bytes):
        body = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    elif body is not None:
        headers["Content-Type"] = "application/octet-stream"
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    opener = urllib.request.build_opener(SafeRedirect())
    with opener.open(req, timeout=180) as response:
        data = response.read()
        return data if binary else json.loads(data) if data else None


def metadata():
    import tomllib
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    ref = os.environ.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/") and ref != f"refs/tags/v{version}":
        raise SystemExit("The release tag must match Cargo.toml")
    commit = os.environ.get("GITHUB_SHA") or subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    return version, commit


def draft(version, commit):
    tag = f"v{version}"
    releases = request("/releases?per_page=100")
    release = next((item for item in releases if item["tag_name"] == tag), None)
    if release:
        if not release["draft"]:
            raise SystemExit("This version is already published. Increase Cargo.toml before publishing again.")
        if release["target_commitish"] != commit:
            raise SystemExit("Draft belongs to another source commit; refusing to mix packages")
        return release
    notes = (ROOT / f"documentation/RELEASE-{version}.md").read_text()
    return request("/releases", "POST", dict(tag_name=tag, target_commitish=commit,
                   name=f"IrisScope {version} — macOS Intel et Linux", body=notes,
                   draft=True, prerelease=False))


def macos_names(version):
    names = [f"IrisScope-{version}-macos-x86_64.zip", f"IrisScope-{version}-macOS.dmg",
             f"IrisScope-{version}-macos-x86_64-update.tar.gz"]
    return [entry for name in names for entry in (name, name + ".sha256")]


def verify_checksums(directory, names):
    import hashlib
    for name in names:
        if name.endswith(".sha256"):
            expected, filename = (directory / name).read_text().strip().split(maxsplit=1)
            if filename != name.removesuffix(".sha256"):
                raise SystemExit("Invalid checksum filename")
            with (directory / filename).open("rb") as file:
                if hashlib.file_digest(file, "sha256").hexdigest() != expected:
                    raise SystemExit(f"Checksum mismatch: {filename}")


def upload(release, file):
    existing = next((item for item in release["assets"] if item["name"] == file.name), None)
    if existing:
        request(f"/releases/assets/{existing['id']}", "DELETE")
    url = release["upload_url"].split("{")[0] + "?" + urllib.parse.urlencode({"name": file.name})
    request(url, "POST", file.read_bytes())
    print(f"Uploaded {file.name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=ROOT / "dist")
    parser.add_argument("--reuse-macos", action="store_true")
    parser.add_argument("--upload-macos", action="store_true")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("--check-version", action="store_true")
    args = parser.parse_args()
    version, commit = metadata()
    if args.check_version:
        print(f"Release source: v{version}, {commit}")
        return
    release = draft(version, commit)
    args.directory.mkdir(parents=True, exist_ok=True)
    if args.reuse_macos:
        for name in macos_names(version):
            asset = next((item for item in release["assets"] if item["name"] == name), None)
            if not asset:
                raise SystemExit(f"Missing locally tested Mac package: {name}")
            (args.directory / name).write_bytes(request(f"/releases/assets/{asset['id']}", binary=True))
        verify_checksums(args.directory, macos_names(version))
        print("Reused byte-identical Mac packages tested locally")
    elif args.upload_macos:
        verify_checksums(args.directory, macos_names(version))
        for name in macos_names(version):
            upload(release, args.directory / name)
        print("Mac packages remain in a draft until Linux packages are ready")
    elif args.publish:
        spec = importlib.util.spec_from_file_location("updater_package", ROOT / "scripts/package-update.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        linux_names = [entry for name in (f"IrisScope-{version}-linux-x86_64.tar.gz", f"IrisScope-{version}-amd64.deb")
                       for entry in (name, name + ".sha256")]
        names = macos_names(version) + linux_names
        verify_checksums(args.directory, names)
        module.create_manifest(args.directory, version, os.environ["IRISCOPE_UPDATE_PRIVATE_KEY"])
        for name in names + ["update-manifest.json", "update-manifest.sig"]:
            upload(release, args.directory / name)
        release = request(f"/releases/{release['id']}", "PATCH", dict(draft=False, make_latest="true"))
        print(release["html_url"])
    else:
        parser.error("Choose --reuse-macos, --upload-macos or --publish")


if __name__ == "__main__":
    main()
