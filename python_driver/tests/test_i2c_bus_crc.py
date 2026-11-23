"""验证 v3 帧封装、回执确认和会话恢复，无需 I2C 硬件。"""

import errno
import unittest
from collections import deque
from contextlib import ExitStack
from unittest.mock import call, patch

from ssd1309_driver.crc import append_crc, strip_and_verify
from ssd1309_driver.errors import FrameRejectedError, I2cError, InvalidDataError
from ssd1309_driver.i2c_bus import I2cBus

SYNC = bytes.fromhex("7F 00 00 00 19 F0")
SYNCED = bytes.fromhex("01 00 00 00 01 D8")
NEED_SYNC = bytes.fromhex("00 00 00 00 00 24")
COMMAND = bytes.fromhex("00 01 01 00 AE F5 80")
OK_COMMAND = bytes.fromhex("83 01 00 00 78 60")
DATA = bytes.fromhex("40 02 02 00 01 02 F6 F2")
OK_DATA = bytes.fromhex("83 02 00 00 88 60")
PANEL_QUERY = bytes.fromhex("01 00 03 00 01 28")
PANEL_REPLY = bytes.fromhex("83 03 00 81 19 C0")
CRC_ERROR = bytes.fromhex("84 01 00 00 79 14")
BUSY_DROP = bytes.fromhex("C6 01 00 00 6D 6C")


class I2cBusFramingTest(unittest.TestCase):
    def setUp(self):
        stack = ExitStack()
        self.addCleanup(stack.close)
        self.clock = 0
        self.events = deque()
        self.writes = []
        module = "ssd1309_driver.i2c_bus"
        stack.enter_context(patch(f"{module}.os.open", return_value=7))
        self.close = stack.enter_context(patch(f"{module}.os.close"))
        self.ioctl = stack.enter_context(patch(f"{module}.fcntl.ioctl"))
        stack.enter_context(patch(f"{module}.os.write", side_effect=self._write))
        stack.enter_context(patch(f"{module}.os.read", side_effect=self._read))
        stack.enter_context(
            patch(f"{module}.time.monotonic_ns", side_effect=lambda: self.clock)
        )
        stack.enter_context(patch(f"{module}.time.sleep", side_effect=self._sleep))
        self.bus = I2cBus(1, 0x3C)

    def _write(self, fd, frame):
        self.assertEqual(fd, 7)
        kind, expected, result = self.events.popleft()
        self.assertEqual((kind, expected), ("write", frame))
        self.writes.append((self.clock, frame))
        if isinstance(result, OSError):
            raise result
        return len(frame) if result is None else result

    def _read(self, fd, length):
        self.assertEqual((fd, length), (7, 6))
        kind, reply, delay = self.events.popleft()
        self.assertEqual(kind, "read")
        self.clock += delay
        if isinstance(reply, OSError):
            raise reply
        return reply

    def _sleep(self, seconds):
        self.clock += round(seconds * 1_000_000_000)

    def _open(self):
        self.events.extend((("write", SYNC, None), ("read", SYNCED, 0)))
        self.assertIs(self.bus.open(), self.bus)
        self.assertFalse(self.events)

    def test_wire_vectors_and_panel_status(self):
        self._open()
        self.assertEqual(self.clock, 5_000_000)
        self.assertEqual(
            self.ioctl.call_args_list, [call(7, 0x0703, 0x3C), call(7, 0x0702, 10)]
        )
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", OK_COMMAND, 0),
                ("write", DATA, None),
                ("read", OK_DATA, 0),
                ("write", PANEL_QUERY, None),
                ("read", PANEL_REPLY, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.bus.write_data(b"\x01\x02")
        self.assertEqual(self.bus.read_byte(), 0x81)
        self.assertFalse(self.events)

    def test_sync_retries_when_device_still_reports_need_sync(self):
        self.events.extend(
            (
                ("write", SYNC, None),
                ("read", NEED_SYNC, 0),
                ("write", SYNC, None),
                ("read", SYNCED, 0),
            )
        )
        self.bus.open()
        self.assertEqual(self.clock, 12_000_000)
        self.assertFalse(self.events)

    def test_sync_waits_for_idle_before_accepting_synced_snapshot(self):
        self.events.extend(
            (
                ("write", SYNC, None),
                ("read", append_crc(b"\x41\x00\x00\x00"), 0),
                ("read", SYNCED, 0),
            )
        )
        self.bus.open()
        self.assertEqual(self.clock, 7_000_000)
        self.assertFalse(self.events)

    def test_timeout_and_corrupted_reply_trigger_reads_before_any_replay(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, OSError(errno.ETIMEDOUT, "超时")),
                ("read", OSError(errno.EREMOTEIO, "NACK"), 0),
                ("read", OK_COMMAND[:3], 0),
                ("read", OK_COMMAND[:-1] + b"\x61", 0),
                ("read", OK_COMMAND, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.assertEqual([frame for _, frame in self.writes], [SYNC, COMMAND])
        self.assertFalse(self.events)

    def test_short_write_replays_whole_frame_after_result_window(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, 2),
                ("read", OK_COMMAND[:3], 100_000_000),
                ("read", OK_COMMAND[:3], 0),
                ("write", COMMAND, None),
                ("read", OK_COMMAND, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.assertEqual(self.writes[2][0] - self.writes[1][0], 104_000_000)
        self.assertFalse(self.events)

    def test_replay_waits_for_read_to_return_bytes_after_transport_errors(self):
        self._open()
        timeout = OSError(errno.ETIMEDOUT, "超时")
        self.events.extend(
            (
                ("write", COMMAND, timeout),
                ("read", timeout, 100_000_000),
                ("read", timeout, 0),
                ("read", b"", 0),
                ("read", OK_COMMAND[:3], 0),
                ("write", COMMAND, None),
                ("read", OK_COMMAND, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.assertEqual(self.writes[2][0] - self.writes[1][0], 108_000_000)
        self.assertFalse(self.events)

    def test_stale_ok_and_crc_error_cannot_confirm_new_sequence(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", OK_COMMAND, 0),
                ("write", DATA, None),
                ("read", OK_COMMAND, 0),
                ("read", CRC_ERROR, 0),
                ("read", CRC_ERROR, 5_000_000),
                ("write", DATA, None),
                ("read", OK_DATA, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.bus.write_data(b"\x01\x02")
        self.assertEqual(self.writes[-1][0] - self.writes[-2][0], 11_000_000)
        self.assertFalse(self.events)

    def test_known_busy_prevents_replay_and_deadline_invalidates_session(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", OK_COMMAND, 0),
                ("write", DATA, None),
                ("read", BUSY_DROP, 0),
                ("read", b"", 100_000_000),
                ("read", OK_DATA, 396_000_000),
            )
        )
        self.bus.write_command(b"\xae")
        with self.assertRaisesRegex(I2cError, "确认超时，结果未知") as failure:
            self.bus.write_data(b"\x01\x02")
        self.assertNotIsInstance(failure.exception, FrameRejectedError)
        self.assertEqual(self.clock, 505_000_000)
        with self.assertRaisesRegex(I2cError, "会话失效"):
            self.bus.read_byte()
        self.assertFalse(self.events)

    def test_link_reset_requires_reopen_and_restarts_sequence(self):
        self._open()
        self.events.extend((("write", COMMAND, None), ("read", NEED_SYNC, 0)))
        with self.assertRaisesRegex(I2cError, "NEED_SYNC"):
            self.bus.write_command(b"\xae")
        self._open()
        self.close.assert_called_once_with(7)
        self.events.extend((("write", COMMAND, None), ("read", OK_COMMAND, 0)))
        self.bus.write_command(b"\xae")
        self.assertFalse(self.events)

    def test_unknown_state_is_protocol_error_even_for_matching_sequence(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", append_crc(b"\xbf\x01\x00\x00"), 0),
            )
        )
        with self.assertRaisesRegex(I2cError, "状态码未知"):
            self.bus.write_command(b"\xae")
        self.assertFalse(self.events)

    def test_inconsistent_receipt_is_protocol_error(self):
        for body in (b"\x81\x01\x00\x00", b"\x03\x00\x00\x00", b"\x04\x01\x00\x00"):
            with self.subTest(body=body):
                self._open()
                self.events.extend(
                    (
                        ("write", COMMAND, None),
                        ("read", append_crc(body), 0),
                    )
                )
                with self.assertRaisesRegex(I2cError, "回执字段不一致"):
                    self.bus.write_command(b"\xae")
                self.assertFalse(self.events)

    def test_invalid_payload_does_not_advance_sequence_and_max_size_is_accepted(self):
        self._open()
        with self.assertRaises(InvalidDataError):
            self.bus.write_command(b"")
        with self.assertRaises(InvalidDataError):
            self.bus.write_data(bytes(256))
        payload = bytes(range(255))
        frame = append_crc(bytes((0x40, 255, 1, 0)) + payload)
        self.events.extend((("write", frame, None), ("read", OK_COMMAND, 0)))
        self.bus.write_data(payload)
        self.assertEqual(len(frame), 261)
        self.assertEqual(strip_and_verify(self.writes[-1][1])[4:], payload)
        self.assertFalse(self.events)

    def test_open_failure_closes_descriptor(self):
        self.ioctl.side_effect = [0, OSError(errno.EIO, "设置超时失败")]
        with self.assertRaises(I2cError):
            self.bus.open()
        self.close.assert_called_once_with(7)
        self.assertFalse(self.events)


if __name__ == "__main__":
    unittest.main()
