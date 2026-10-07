#!/usr/bin/env python3
"""Build the bundled Mac DE400 components; never install or run privileged code."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "target/macos-button-research"
LIBUSB_SHA256 = "fea36f34f9156400209595e300840767ab1a385ede1dc7ee893015aea9c6dbaf"
LIBUVC_COMMIT = "d07de4fa23905ee8cac06f5d24b2a03d42a3b363"


def run(*args, cwd=ROOT):
    subprocess.run([str(arg) for arg in args], cwd=cwd, check=True)


def main():
    BUILD.mkdir(parents=True, exist_ok=True)
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
    run(cmake, "-S", project, "-B", BUILD / "uvc-build", "-DCMAKE_BUILD_TYPE=Release")
    run(cmake, "--build", BUILD / "uvc-build", "-j2")
    for name in ("de400-uvc-probe", "de400-usb-helper"):
        run("clang", "-O2", "-std=c11", "-D_DEFAULT_SOURCE", "-Wall", "-Wextra", "-Werror",
            "-I", uvc / "include", "-I", BUILD / "uvc-build/libuvc/include",
            "-I", usb / "libusb", ROOT / f"scripts/macos/{name}.c",
            BUILD / "uvc-build/libuvc/libuvc.a", usb / "libusb/.libs/libusb-1.0.a",
            "-framework", "IOKit", "-framework", "CoreFoundation", "-framework", "Security",
            "-lobjc", "-o", BUILD / name)
    run("clang", "-O2", "-fobjc-arc", "-Wall", "-Wextra", "-Werror",
        ROOT / "scripts/macos/iriscope-launcher.m", "-framework", "Cocoa", "-framework", "IOKit",
        "-o", BUILD / "iriscope-launcher")
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
    run("clang", "-O2", "-std=c11", "-D_DEFAULT_SOURCE", "-I", uvc / "include",
        "-I", BUILD / "uvc-build/libuvc/include", "-I", usb / "libusb",
        "-c", ROOT / "scripts/macos/de400-usb-helper.c", "-o", resources / "de400-usb-helper.o")
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


if __name__ == "__main__":
    main()
