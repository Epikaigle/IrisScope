#!/usr/bin/env python3
"""Bounded DE400 probe: one MJPEG video stream plus its UVCH metadata.
No vendor USB commands. Images go to /dev/null. Close the IrisScope preview
before running this script and verify --video/--metadata refer to the DE400.
Start only after coordinating the physical press window.
"""
import argparse
import collections
import json
import os
import selectors
import signal
import struct
import subprocess
import sys
import time


class MetadataParser:
    def __init__(self):
        self.pending = bytearray()
        self.bytes = 0
        self.headers = 0
        self.flags = collections.Counter()
        self.previous_fid = None
        self.frames = 0
        self.sti_headers = 0
        self.sti_frames = 0
        self.sti_rises = 0
        self.previous_sti = False
        self.last_sti_frame = None
        self.first_ts = None
        self.last_ts = None
        self.events = []

    def feed(self, chunk, emit=print):
        self.bytes += len(chunk)
        self.pending.extend(chunk)
        while len(self.pending) >= 12:
            length, flags = self.pending[10:12]
            standard_length = 2 + (4 if flags & 4 else 0) + (6 if flags & 8 else 0)
            if length < standard_length or length > 12:
                raise ValueError(f'UVCH header length {length} incompatible with flags 0x{flags:02x}; parsing stopped rather than guessing boundaries')
            block_length = 10 + length
            if len(self.pending) < block_length:
                break
            block = bytes(self.pending[:block_length])
            del self.pending[:block_length]
            timestamp, sof = struct.unpack_from('=QH', block)
            self.first_ts = timestamp if self.first_ts is None else self.first_ts
            self.last_ts = timestamp
            self.headers += 1
            self.flags[f'0x{flags:02x}'] += 1
            fid = flags & 1
            if self.previous_fid != fid:
                self.frames += 1
            self.previous_fid = fid
            pts = struct.unpack_from('<I', block, 12)[0] if flags & 4 else None
            frame_id = ('pts', pts) if pts is not None else ('fid-transition', self.frames)
            sti = bool(flags & 0x20)
            if sti:
                self.sti_headers += 1
                if self.last_sti_frame != frame_id:
                    self.sti_frames += 1
                    self.last_sti_frame = frame_id
                    event = {'seconds': round((timestamp - self.first_ts) / 1e9, 3), 'sof': sof, 'flags': f'0x{flags:02x}', 'pts': pts, 'frame': self.frames}
                    if len(self.events) < 100:
                        self.events.append(event)
                    if self.sti_frames <= 30:
                        emit('STILL_IMAGE ' + json.dumps(event), flush=True)
                if not self.previous_sti:
                    self.sti_rises += 1
            self.previous_sti = sti

    def summary(self):
        return {'metadata_bytes': self.bytes, 'metadata_headers': self.headers,
                'fid_frames_seen': self.frames, 'flags_histogram': dict(self.flags),
                'sti_headers': self.sti_headers, 'sti_frames': self.sti_frames,
                'sti_rising_edges': self.sti_rises, 'sti_events': self.events,
                'trailing_metadata_bytes': len(self.pending),
                'metadata_duration_s': None if self.first_ts is None else round((self.last_ts - self.first_ts) / 1e9, 3)}


def self_test():
    parser = MetadataParser()
    def header(ts, flag, pts):
        return struct.pack('=QH', ts, 123) + bytes([12, flag | 0x0c]) + struct.pack('<I', pts) + bytes(6)
    data = header(1_000_000_000, 0, 10) + header(1_100_000_000, 0x21, 11) + header(1_100_001_000, 0x21, 11) + header(1_200_000_000, 0, 12) + header(1_300_000_000, 0x21, 13)
    for offset in range(0, len(data), 7):
        parser.feed(data[offset:offset + 7], emit=lambda *a, **k: None)
    assert parser.headers == 5 and parser.frames == 4
    assert parser.sti_headers == 3 and parser.sti_frames == 2 and parser.sti_rises == 2
    assert not parser.pending
    print('SELF_TEST_OK')


def run(args):
    parser = MetadataParser()
    selector = selectors.DefaultSelector()
    children = []
    logs = {'video-stdout': bytearray(), 'video-stderr': bytearray(), 'metadata-stderr': bytearray()}
    total_logs = collections.Counter()
    stop = False
    armed = False
    started = time.monotonic()
    deadline = started + args.seconds
    failure = None
    exit_codes = {}

    def interrupt(signum, frame):
        nonlocal stop
        stop = True

    for name in (signal.SIGINT, signal.SIGTERM):
        signal.signal(name, interrupt)

    def start_child(kind, command):
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        children.append((kind, child))
        for stream_kind, stream in [('stdout', child.stdout), ('stderr', child.stderr)]:
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ, f'{kind}-{stream_kind}')
        return child

    try:
        start_child('metadata', ['stdbuf', '-o0', 'v4l2-ctl', '-d', args.metadata, '--set-fmt-meta=UVCH', '--stream-mmap=2', '--stream-poll', '--stream-no-query', '--stream-to=-'])
        start_child('video', ['v4l2-ctl', '-d', args.video, '--set-fmt-video=width=1280,height=1024,pixelformat=MJPG', '--set-parm=8', '--stream-mmap=3', '--stream-poll', '--stream-no-query', '--stream-to=/dev/null'])
        print(f'PROBE_START duration={args.seconds}s video={args.video} MJPEG=1280x1024@8 metadata={args.metadata} UVCH; no image files', flush=True)
        while not stop and time.monotonic() < deadline:
            for key, _ in selector.select(timeout=min(0.25, max(0, deadline - time.monotonic()))):
                chunk = os.read(key.fileobj.fileno(), 16 * 1024)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                name = key.data
                if name == 'metadata-stdout':
                    parser.feed(chunk)
                    if parser.headers and not armed:
                        armed = True
                        print('PROBE_ARMED metadata received; physical button press window is active', flush=True)
                else:
                    total_logs[name] += len(chunk)
                    logs[name].extend(chunk[-8192:])
                    if len(logs[name]) > 8192:
                        del logs[name][:-8192]
            for name, child in children:
                if child.poll() is not None:
                    failure = f'{name} exited before observation ended (code {child.returncode})'
                    stop = True
                    break
    except (OSError, ValueError) as error:
        failure = str(error)
    finally:
        for name, child in children:
            if child.poll() is None:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        grace = time.monotonic() + 2
        for name, child in children:
            try:
                child.wait(timeout=max(0.01, grace - time.monotonic()))
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait(timeout=2)
            exit_codes[name] = child.returncode
        for key in list(selector.get_map().values()):
            try:
                while True:
                    chunk = os.read(key.fileobj.fileno(), 16 * 1024)
                    if not chunk:
                        break
                    if key.data == 'metadata-stdout' and failure is None:
                        parser.feed(chunk)
                    elif key.data in logs:
                        total_logs[key.data] += len(chunk)
                        logs[key.data].extend(chunk[-8192:])
                        if len(logs[key.data]) > 8192:
                            del logs[key.data][:-8192]
            except (BlockingIOError, OSError, ValueError):
                pass
            key.fileobj.close()
        selector.close()
        for name, child in children:
            for stream in (child.stdout, child.stderr):
                if stream and not stream.closed:
                    stream.close()

    result = parser.summary()
    result.update({'wall_duration_s': round(time.monotonic() - started, 3), 'armed': armed, 'failure': failure, 'child_exit_codes': exit_codes, 'diagnostic_bytes': dict(total_logs), 'diagnostic_tail': {key: value.decode('utf-8', errors='replace') for key, value in logs.items()}})
    if args.report:
        with open(args.report, 'w', encoding='utf-8') as report:
            json.dump(result, report, ensure_ascii=False, indent=2)
            report.write('\n')
    print('PROBE_RESULT ' + json.dumps(result, ensure_ascii=False), flush=True)
    return 1 if failure or not armed else 0


if __name__ == '__main__':
    arg_parser = argparse.ArgumentParser(description=__doc__)
    arg_parser.add_argument('--seconds', type=float, default=45)
    arg_parser.add_argument('--video', default='/dev/video0')
    arg_parser.add_argument('--metadata', default='/dev/video1')
    arg_parser.add_argument('--report', default='/tmp/iriscope-de400-button-probe.json')
    arg_parser.add_argument('--self-test', action='store_true')
    args = arg_parser.parse_args()
    if args.self_test:
        self_test()
    elif not 1 <= args.seconds <= 45:
        arg_parser.error('--seconds must be between 1 and 45')
    else:
        sys.exit(run(args))
