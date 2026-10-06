#!/usr/bin/env python3
"""Observe only the DE400 interrupt status endpoint through Linux usbmon.

Requires root to read usbmon. No USB writes, interface detach, or media files.
Other endpoint traffic is discarded and never printed or saved. Close IrisScope
before running this bounded probe. Coordinate physical presses after STATUS_ARMED.
"""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=45)
    parser.add_argument("--video", default="/dev/video0")
    args = parser.parse_args()
    if not 1 <= args.seconds <= 45:
        parser.error("--seconds must be between 1 and 45")
    if os.geteuid() != 0:
        parser.error("root is required to read the USB status endpoint with usbmon")

    node = (Path("/sys/class/video4linux") / Path(args.video).name / "device").resolve()
    usb = next((parent for parent in node.parents if (parent / "idVendor").is_file()), None)
    if usb is None or (usb / "idVendor").read_text().strip() != "21cd" or (usb / "idProduct").read_text().strip() != "603b":
        parser.error("--video must refer to the Infoxelle / Firefly DE400 (21cd:603b)")
    bus = int((usb / "busnum").read_text())
    device = int((usb / "devnum").read_text())
    subprocess.run(["/usr/sbin/modprobe", "usbmon"], check=True)
    monitor = Path(f"/sys/kernel/debug/usb/usbmon/{bus}u")
    if not monitor.is_file():
        parser.error(f"usbmon is unavailable at {monitor}; debugfs must be mounted")
    fd = os.open(monitor, os.O_RDONLY | os.O_NONBLOCK)
    stop = False

    def interrupt(_signum, _frame):
        nonlocal stop
        stop = True

    signal.signal(signal.SIGINT, interrupt)
    signal.signal(signal.SIGTERM, interrupt)
    video = subprocess.Popen(
        ["v4l2-ctl", "-d", args.video,
         "--set-fmt-video=width=1280,height=1024,pixelformat=MJPG", "--set-parm=8",
         "--stream-mmap=3", "--stream-poll", "--stream-no-query", "--stream-to=/dev/null"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    events = []
    pending = b""
    started = time.monotonic()
    print(f"STATUS_ARMED bus={bus} device={device} endpoint=0x81 duration={args.seconds}s", flush=True)
    try:
        while not stop and time.monotonic() - started < args.seconds:
            if video.poll() is not None:
                raise RuntimeError(f"camera stream exited with code {video.returncode}")
            # Text usbmon returns one event per read. Drain the queue so
            # video traffic does not crowd out the interrupt status packets.
            for _ in range(512):
                try:
                    chunk = os.read(fd, 65536)
                except BlockingIOError:
                    break
                if not chunk:
                    raise RuntimeError("usbmon stream ended")
                pending += chunk
                lines = pending.split(b"\n")
                pending = lines.pop()
                if len(pending) > 65536:
                    raise RuntimeError("unexpected usbmon record size")
                for line in lines:
                    fields = line.decode("ascii", errors="replace").split()
                    if len(fields) < 5 or fields[2] != "C":
                        continue
                    address = fields[3].split(":")
                    if len(address) != 4 or address[0] != "Ii":
                        continue
                    if tuple(map(int, address[1:])) != (bus, device, 1):
                        continue
                    event = {"seconds": round(time.monotonic() - started, 3), "status": " ".join(fields[4:])}
                    if len(events) < 100:
                        events.append(event)
                        print("USB_STATUS " + json.dumps(event), flush=True)
            time.sleep(0.01)
    finally:
        video.terminate()
        try:
            video.wait(timeout=2)
        except subprocess.TimeoutExpired:
            video.kill()
            video.wait()
        os.close(fd)
        print("STATUS_RESULT " + json.dumps({"seconds": round(time.monotonic() - started, 3), "events": events}), flush=True)


if __name__ == "__main__":
    main()
