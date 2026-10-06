"""USB packets observed on the actual DE400, without requiring root or hardware."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

source = Path(__file__).resolve().parents[1] / "helpers/linux/de400_button_bridge.py"
spec = importlib.util.spec_from_file_location("de400_button_bridge", source)
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class ButtonPacketTests(unittest.TestCase):
    devices = {(1, 4): "1-3", (2, 8): "2-2"}

    def test_observed_press_and_release(self):
        self.assertEqual(bridge.decode_status(b"ffff 25 C Ii:1:004:1 0:256 4 = 02010001", self.devices), ("1-3", True))
        self.assertEqual(bridge.decode_status(b"ffff 26 C Ii:1:004:1 0:256 4 = 02010000", self.devices), ("1-3", False))
        self.assertEqual(bridge.decode_status(b"ffff 26 C Ii:2:008:1 0:256 4 = 02010001", self.devices), ("2-2", True))

    def test_other_usb_devices_video_frames_and_errors_do_not_capture(self):
        for line in [
            b"ffff 25 C Ii:1:003:1 0:256 4 = 02010001",
            b"ffff 25 C Zi:1:004:2 0:256 4 = 02010001",
            b"ffff 25 S Ii:1:004:1 0:256 4 = 02010001",
            b"ffff 25 C Ii:1:004:1 -71:256 4 = 02010001",
            b"ffff 25 C Ii:1:004:1 0:256 8 = 02010001",
            b"ffff 25 C Ii:1:004:1 0:256 4 = 02010101",
            b"ffff 25 C Ii:1:004:1 0:256 4 = 02020001",
            b"ffff 25 C Ii:1:004:1 0:256 4 = 02010002",
            b"ffff 25 C Ii:1:004:1 0:256 4 = invalid!",
            b"not a USB status packet",
        ]:
            with self.subTest(line=line):
                self.assertIsNone(bridge.decode_status(line, self.devices))

    def test_usbmon_queue_is_drained_and_partial_records_are_preserved(self):
        monitor = [7, b""]
        with patch.object(bridge.os, "read", side_effect=[b"video\n", b"press", b"\nrelease\npart", BlockingIOError()]) as read:
            self.assertEqual(list(bridge.read_monitor(monitor)), [b"video", b"press", b"release"])
            self.assertEqual(read.call_count, 4)
        self.assertEqual(monitor[1], b"part")
        with patch.object(bridge.os, "read", side_effect=[b"ial\n", BlockingIOError()]):
            self.assertEqual(list(bridge.read_monitor(monitor)), [b"partial"])

    def test_busy_usb_bus_cannot_starve_other_clients(self):
        with patch.object(bridge.os, "read", return_value=b"video\n") as read:
            self.assertEqual(len(list(bridge.read_monitor([7, b""]))), 512)
            self.assertEqual(read.call_count, 512)


if __name__ == "__main__":
    unittest.main()
