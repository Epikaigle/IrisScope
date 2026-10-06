#!/usr/bin/env python3
"""Forward verified DE400 interrupt packets to local IrisScope clients.

Reads Linux usbmon; never opens a video stream or writes to a USB device.
Only 21cd:603b endpoint 0x81 packets 02 01 00 00/01 are used. Traffic from
other devices and endpoints, including image payloads, is discarded.
"""

import grp
import os
from pathlib import Path
import selectors
import signal
import socket
import time

SOCKET_PATH = "/run/iriscope-button/status.sock"


def discover_devices(root=Path("/sys/bus/usb/devices")):
    devices = {}
    for path in root.iterdir():
        try:
            if (path / "idVendor").read_text().strip() != "21cd":
                continue
            if (path / "idProduct").read_text().strip() != "603b":
                continue
            address = (int((path / "busnum").read_text()), int((path / "devnum").read_text()))
            devices[address] = path.name
        except (OSError, ValueError):
            continue
    return devices


def decode_status(line, devices):
    """Reject failures, unrelated endpoints and every unverified packet shape."""
    fields = line.split()
    if len(fields) != 8 or fields[2] != b"C" or fields[6] != b"=":
        return None
    address = fields[3].split(b":")
    if len(address) != 4 or address[0] != b"Ii":
        return None
    try:
        bus, device, endpoint = map(int, address[1:])
        if endpoint != 1 or int(fields[4].split(b":")[0]) != 0 or int(fields[5]) != 4:
            return None
        key = devices.get((bus, device))
        packet = bytes.fromhex(fields[7].decode("ascii"))
    except (ValueError, UnicodeError):
        return None
    if key is None or packet not in (b"\x02\x01\x00\x01", b"\x02\x01\x00\x00"):
        return None
    return key, bool(packet[3])


def main():
    os.umask(0o077)
    selector = selectors.PollSelector()
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    socket_path = Path(SOCKET_PATH)
    socket_path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    socket_path.unlink(missing_ok=True)
    server.bind(SOCKET_PATH)
    os.chown(SOCKET_PATH, 0, grp.getgrnam("video").gr_gid)
    os.chmod(SOCKET_PATH, 0o660)
    server.listen(8)
    server.setblocking(False)
    selector.register(server, selectors.EVENT_READ, ("server", None))
    clients = {}
    monitors = {}
    devices = {}
    stop = False
    next_scan = 0.0

    def shutdown(_signum, _frame):
        nonlocal stop
        stop = True

    def close_client(client):
        selector.unregister(client)
        clients.pop(client, None)
        client.close()

    def send(client, payload):
        try:
            if client.send(payload) != len(payload):
                close_client(client)
                return False
            return True
        except OSError:
            close_client(client)
            return False

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)
    print("DE400 button bridge ready; no media is recorded", flush=True)
    try:
        while not stop:
            if time.monotonic() >= next_scan:
                next_scan = time.monotonic() + 1
                devices = discover_devices()
                for bus in {address[0] for address in devices} - monitors.keys():
                    try:
                        fd = os.open(f"/sys/kernel/debug/usb/usbmon/{bus}u", os.O_RDONLY | os.O_NONBLOCK)
                        monitors[bus] = [fd, b""]
                        selector.register(fd, selectors.EVENT_READ, ("monitor", bus))
                    except OSError as error:
                        print(f"usbmon bus {bus} unavailable: {error}", flush=True)
            for item, _mask in selector.select(timeout=0.25):
                kind, bus = item.data
                if kind == "server":
                    client, _address = server.accept()
                    client.setblocking(False)
                    if len(clients) >= 16:
                        client.close()
                        continue
                    clients[client] = [None, b""]
                    selector.register(client, selectors.EVENT_READ, ("client", None))
                elif kind == "client":
                    client = item.fileobj
                    if client not in clients:
                        continue
                    try:
                        data = client.recv(64)
                    except BlockingIOError:
                        continue
                    except OSError:
                        close_client(client)
                        continue
                    if not data or clients[client][0] is not None:
                        close_client(client)
                        continue
                    clients[client][1] += data
                    pending = clients[client][1]
                    if len(pending) > 32 or pending.count(b"\n") > 1:
                        close_client(client)
                        continue
                    if not pending.endswith(b"\n"):
                        continue
                    key = pending[:-1].decode("ascii", errors="replace")
                    if key not in devices.values():
                        close_client(client)
                        continue
                    if send(client, b"READY\n"):
                        clients[client] = [key, b""]
                else:
                    # usbmon text returns one record per read, regardless of
                    # the requested size. Drain its small kernel queue before
                    # sleeping: video URBs otherwise crowd out button packets.
                    for line in read_monitor(monitors[bus]):
                        event = decode_status(line, devices)
                        if event is None:
                            continue
                        key, value = event
                        for client, (subscription, _pending) in list(clients.items()):
                            if subscription == key:
                                send(client, b"P\n" if value else b"R\n")
            # debugfs usbmon files always poll as readable, even when read()
            # returns EAGAIN. Bound the loop instead of spinning on an idle bus.
            time.sleep(0.01)
    finally:
        for client in list(clients):
            close_client(client)
        for fd, _pending in monitors.values():
            os.close(fd)
        selector.close()
        server.close()
        socket_path.unlink(missing_ok=True)


def read_monitor(monitor):
    """Drain a bounded batch of complete records, preserving split lines."""
    fd, pending = monitor
    for _ in range(512):
        try:
            chunk = os.read(fd, 65536)
        except BlockingIOError:
            break
        if not chunk:
            raise RuntimeError("usbmon stream ended")
        lines = (pending + chunk).split(b"\n")
        pending = lines.pop()
        if len(pending) > 65536:
            raise RuntimeError("unexpected usbmon record size")
        yield from lines
    monitor[1] = pending


if __name__ == "__main__":
    main()
