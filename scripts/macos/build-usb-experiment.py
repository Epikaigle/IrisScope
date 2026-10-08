#!/usr/bin/env python3
"""Build the bundled Mac DE400 components; never install or run privileged code."""
import hashlib
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "target/macos-button-research"
LIBUSB_SHA256 = "fea36f34f9156400209595e300840767ab1a385ede1dc7ee893015aea9c6dbaf"
LIBUVC_COMMIT = "d07de4fa23905ee8cac06f5d24b2a03d42a3b363"


def run(*args, cwd=ROOT):
    subprocess.run([str(arg) for arg in args], cwd=cwd, check=True,
                   env={**os.environ, "MACOSX_DEPLOYMENT_TARGET": "15.0"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    partial = parser.add_mutually_exclusive_group()
    partial.add_argument("--service-only", action="store_true",
                        help="Reuse the USB reader/libraries and compile only the small native service and launcher")
    partial.add_argument("--reader-only", action="store_true",
                        help="Compile only the USB reader against existing USB libraries, without Rust or CMake")
    args = parser.parse_args()
    BUILD.mkdir(parents=True, exist_ok=True)
    if args.reader_only:
        if not (BUILD / "redistribution/NOTICE.txt").is_file():
            parser.error("Build the USB libraries once before using --reader-only")
        build_reader()
        return
    if args.service_only:
        if not (BUILD / "de400-usb-helper").is_file() or not (BUILD / "redistribution/NOTICE.txt").is_file():
            parser.error("Build the USB reader once before using --service-only")
        build_service()
        return
    cmake = shutil.which("cmake") or str(BUILD / "build-tools/bin/cmake")
    if not Path(cmake).is_file():
        raise SystemExit("CMake required. Use your CMake or a private Python venv; no system installation is performed.")
    archive = BUILD / "libusb-1.0.30.tar.bz2"
    if not archive.exists():
        urllib.request.urlretrieve(
            "https://github.com/libusb/libusb/releases/download/v1.0.30/libusb-1.0.30.tar.bz2", archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != LIBUSB_SHA256:
        raise SystemExit("libusb archive checksum mismatch; existing files preserved.")
    usb = BUILD / "libusb-1.0.30"
    if not usb.exists():
        run("tar", "-xjf", archive, "-C", BUILD)
    if not (usb / "libtool").exists():
        run("./configure", "--disable-shared", "--enable-static", cwd=usb)
    run("make", "-j2", cwd=usb)
    uvc = BUILD / "libuvc"
    if not uvc.exists():
        run("git", "clone", "https://github.com/libuvc/libuvc.git", uvc)
        run("git", "checkout", "--detach", LIBUVC_COMMIT, cwd=uvc)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=uvc, text=True).strip()
    if head != LIBUVC_COMMIT:
        raise SystemExit("Unexpected libuvc commit; checkout preserved.")
    patch = ROOT / "scripts/macos/libuvc-de400-descriptors.patch"
    diff = subprocess.check_output(["git", "diff", "--", "src/device.c"], cwd=uvc, text=True)
    if not diff:
        run("git", "apply", "--check", patch, cwd=uvc)
        run("git", "apply", patch, cwd=uvc)
    elif diff != patch.read_text():
        raise SystemExit("Unexpected libuvc source changes; checkout preserved.")
    project = BUILD / "uvc-cmake"
    project.mkdir(exist_ok=True)
    (project / "CMakeLists.txt").write_text('''cmake_minimum_required(VERSION 3.13)
project(IriscopeUSBResearch LANGUAGES C)
add_library(LibUSB::LibUSB STATIC IMPORTED)
set_target_properties(LibUSB::LibUSB PROPERTIES
  IMPORTED_LOCATION "${CMAKE_CURRENT_SOURCE_DIR}/../libusb-1.0.30/libusb/.libs/libusb-1.0.a"
  INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_SOURCE_DIR}/../libusb-1.0.30/libusb")
set(CMAKE_BUILD_TARGET Static CACHE STRING "" FORCE)
set(BUILD_EXAMPLE OFF CACHE BOOL "" FORCE)
set(DISABLE_JPEG ON CACHE BOOL "" FORCE)
set(LIBUVC_NUM_TRANSFER_BUFS 64 CACHE STRING "" FORCE)
add_subdirectory(../libuvc libuvc)
''')
    run(cmake, "-S", project, "-B", BUILD / "uvc-build", "-DCMAKE_BUILD_TYPE=Release",
        "-DCMAKE_OSX_DEPLOYMENT_TARGET=15.0")
    run(cmake, "--build", BUILD / "uvc-build", "-j2")
    for name in ("de400-uvc-probe",):
        run("clang", "-O2", "-std=c11", "-D_DEFAULT_SOURCE", "-Wall", "-Wextra", "-Werror",
            "-I", uvc / "include", "-I", BUILD / "uvc-build/libuvc/include",
            "-I", usb / "libusb", ROOT / f"scripts/macos/{name}.c",
            BUILD / "uvc-build/libuvc/libuvc.a", usb / "libusb/.libs/libusb-1.0.a",
            "-framework", "IOKit", "-framework", "CoreFoundation", "-framework", "Security",
            "-lobjc", "-o", BUILD / name)
    build_reader()
    build_service()
    run("clang", "-std=c11", "-Wall", "-Wextra", "-Werror",
        ROOT / "scripts/macos/test-helper-session.c", "-o", BUILD / "test-helper-session")
    run(BUILD / "test-helper-session")
    resources = BUILD / "redistribution"
    resources.mkdir(exist_ok=True)
    shutil.copy2(usb / "COPYING", resources / "libusb-COPYING.txt")
    shutil.copy2(uvc / "LICENSE.txt", resources / "libuvc-LICENSE.txt")
    shutil.copy2(archive, resources / archive.name)
    run("git", "archive", "--format=tar", "--output", resources / "libuvc-source.tar", LIBUVC_COMMIT, cwd=uvc)
    for name in ("de400-usb-helper.c", "helper-controls.h", "helper-session.h", "libuvc-de400-descriptors.patch", "build-usb-experiment.py"):
        shutil.copy2(ROOT / "scripts/macos" / name, resources / name)
    shutil.copy2(BUILD / "uvc-build/libuvc/libuvc.a", resources / "libuvc.a")
    (resources / "NOTICE.txt").write_text(f"""DE400 USB helper: libusb 1.0.30 (LGPL 2.1 or later), libuvc {LIBUVC_COMMIT} (BSD).
Corresponding libusb/libuvc source archives, the local libuvc patch, helper source,
helper object and libuvc static library are supplied for modification/relinking.
Rebuild libusb with ./configure --disable-shared --enable-static && make.
Relink on this architecture:
clang de400-usb-helper.o libuvc.a PATH_TO_MODIFIED_LIBUSB/libusb/.libs/libusb-1.0.a \\
  -framework IOKit -framework CoreFoundation -framework Security -lobjc -o de400-usb-helper
Replace Contents/MacOS/de400-usb-helper and locally sign it and the bundle with
codesign --force --sign -. No proprietary Firefly SDK is included.
""")
    print(f"Built {BUILD / 'de400-usb-helper'}; USB devices were not opened.")


def build_reader():
    uvc = BUILD / "libuvc"
    usb = BUILD / "libusb-1.0.30"
    resources = BUILD / "redistribution"
    for library in [BUILD / "uvc-build/libuvc/libuvc.a", usb / "libusb/.libs/libusb-1.0.a"]:
        if not library.is_file():
            raise SystemExit(f"Cached USB library missing: {library}; run a full USB build once")
    resources.mkdir(exist_ok=True)
    options = ["-O2", "-std=c11", "-D_DEFAULT_SOURCE", "-Wall", "-Wextra", "-Werror",
               "-I", uvc / "include", "-I", BUILD / "uvc-build/libuvc/include", "-I", usb / "libusb"]
    source = ROOT / "scripts/macos/de400-usb-helper.c"
    run("clang", *options, source, BUILD / "uvc-build/libuvc/libuvc.a", usb / "libusb/.libs/libusb-1.0.a",
        "-framework", "IOKit", "-framework", "CoreFoundation", "-framework", "Security", "-lobjc",
        "-o", BUILD / "de400-usb-helper")
    run("clang", *options, "-c", source, "-o", resources / "de400-usb-helper.o")
    for name in ["de400-usb-helper.c", "helper-controls.h", "helper-session.h", "build-usb-experiment.py"]:
        shutil.copy2(ROOT / "scripts/macos" / name, resources / name)
    print("Built only the DE400 reader; USB libraries and Rust executable reused.")


def build_service():
    for name, frameworks in [("iriscope-launcher", ["Cocoa", "IOKit", "Security"]),
                             ("iriscope-usb-service", ["Foundation", "Security"])]:
        options = [option for framework in frameworks for option in ["-framework", framework]]
        run("clang", "-O2", "-fobjc-arc", "-mmacosx-version-min=13.0", "-Wall", "-Wextra", "-Werror",
            ROOT / f"scripts/macos/{name}.m", *options, "-o", BUILD / name)
    run("clang", "-O2", "-fobjc-arc", "-mmacosx-version-min=13.0", "-Wall", "-Wextra", "-Werror",
        ROOT / "scripts/macos/test-usb-service.m", "-framework", "Foundation", "-framework", "Security",
        "-o", BUILD / "test-usb-service")
    run(BUILD / "test-usb-service")
    resources = BUILD / "redistribution"
    resources.mkdir(exist_ok=True)
    for name in ["iriscope-usb-service.m", "iriscope-launcher.m", "usb-service.h", "usb-service-client.h",
                 "test-usb-service.m", "build-usb-experiment.py"]:
        shutil.copy2(ROOT / "scripts/macos" / name, resources / name)
    print("Built the native launcher and on-demand USB service; no Rust compilation or installation.")


if __name__ == "__main__":
    main()
