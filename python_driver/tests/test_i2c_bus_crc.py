"""验证 v3 帧封装、回执确认和会话恢复，无需 I2C 硬件。"""

import errno
import unittest
from collections import deque
from contextlib import ExitStack
from unittest.mock import call, patch

from ssd1309_driver.crc import append_crc, strip_and_verify
from ssd1309_driver.errors import FrameRejectedError, I2cError, InvalidDataError
from ssd1309_driver.i2c_bus import (
    I2cBus,
    T_FRAME_NS,
    T_POLL_NS,
    T_PUBLISH_NS,
    T_RESULT_NS,
)

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
        self.default_read = None
        module = "ssd1309_driver.i2c_bus"
        self.open_fd = stack.enter_context(patch(f"{module}.os.open", return_value=7))
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
        if self.events:
            kind, reply, delay = self.events.popleft()
            self.assertEqual(kind, "read")
        else:
            self.assertIsNotNone(self.default_read, "意外的读事务")
            reply, delay = self.default_read
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

    def test_no_receipt_times_out_without_replay(self):
        self._open()
        started = self.clock
        self.events.append(("write", COMMAND, None))
        self.default_read = (OSError(errno.EREMOTEIO, "地址未应答"), 0)

        with self.assertRaisesRegex(I2cError, "确认超时，结果未知"):
            self.bus.write_command(b"\xae")
        self.assertEqual(self.clock - started, T_FRAME_NS)
        self.assertEqual([frame for _, frame in self.writes], [SYNC, COMMAND])
        self.assertIsNone(self.bus._next_seq)
        with self.assertRaisesRegex(I2cError, "会话失效"):
            self.bus.write_command(b"\xae")
        self.assertFalse(self.events)

    def test_open_sync_failure_closes_fd_and_allows_retry(self):
        no_receiver = OSError(errno.ENXIO, "地址未应答")
        self.events.append(("write", SYNC, no_receiver))
        self.default_read = (no_receiver, 0)

        with self.assertRaisesRegex(I2cError, "确认超时，结果未知"):
            self.bus.open()
        self.assertEqual(self.clock, T_FRAME_NS)
        self.assertEqual([frame for _, frame in self.writes], [SYNC])
        self.close.assert_called_once_with(7)
        self.assertIsNone(self.bus._fd)
        self.assertIsNone(self.bus._next_seq)

        self.default_read = None
        self._open()
        self.assertEqual(self.open_fd.call_count, 2)
        self.events.extend((("write", COMMAND, None), ("read", OK_COMMAND, 0)))
        self.bus.write_command(b"\xae")
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

    def test_pending_polls_until_success_without_replay(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", append_crc(b"\x42\x00\x00\x00"), T_RESULT_NS),
                ("read", OK_COMMAND, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.assertEqual([frame for _, frame in self.writes], [SYNC, COMMAND])
        self.assertFalse(self.events)

    def test_receiver_errors_retry_same_frame_after_their_wait_window(self):
        for code, delay in (
            (0x05, T_PUBLISH_NS),
            (0x08, T_PUBLISH_NS),
            (0x07, T_RESULT_NS),
        ):
            with self.subTest(code=code):
                self.bus.close()
                self._open()
                reply = append_crc(bytes((code, 0, 0, 0)))
                self.events.extend(
                    (
                        ("write", COMMAND, None),
                        ("read", reply, 0),
                        ("read", reply, delay - T_POLL_NS),
                        ("write", COMMAND, None),
                        ("read", OK_COMMAND, 0),
                    )
                )
                self.bus.write_command(b"\xae")
                self.assertEqual(
                    self.writes[-1][0] - self.writes[-2][0], delay + T_POLL_NS
                )
                self.assertFalse(self.events)

    def test_busy_with_matching_success_confirms_immediately(self):
        self._open()
        started = self.clock
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", append_crc(b"\xc3\x01\x00\x00"), 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.assertEqual(self.clock, started)
        self.assertEqual(self.bus._next_seq, 2)
        self.assertFalse(self.events)

    def test_busy_drop_replays_only_after_busy_clears(self):
        self._open()
        self.events.extend(
            (
                ("write", COMMAND, None),
                ("read", OK_COMMAND, 0),
                ("write", DATA, None),
                ("read", BUSY_DROP, T_RESULT_NS),
                ("read", append_crc(b"\x86\x01\x00\x00"), 0),
                ("write", DATA, None),
                ("read", OK_DATA, 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.bus.write_data(b"\x01\x02")
        self.assertEqual(
            self.writes[-1][0] - self.writes[-2][0], T_RESULT_NS + 2 * T_POLL_NS
        )
        self.assertFalse(self.events)

    def test_persistent_bad_crc_replays_same_frame_then_invalidates_session(self):
        self._open()
        started = self.clock
        bad_reply = OK_COMMAND[:-1] + b"\x61"
        replay_interval = T_RESULT_NS + 2 * T_POLL_NS
        replays, remaining = divmod(T_FRAME_NS, replay_interval)
        for _ in range(replays):
            self.events.extend(
                (
                    ("write", COMMAND, None),
                    ("read", bad_reply, T_RESULT_NS),
                    ("read", bad_reply, 0),
                )
            )
        self.events.extend((("write", COMMAND, None), ("read", bad_reply, remaining)))

        with self.assertRaisesRegex(I2cError, "确认超时，结果未知"):
            self.bus.write_command(b"\xae")
        self.assertEqual(self.clock - started, T_FRAME_NS)
        self.assertEqual(
            [frame for _, frame in self.writes[1:]], [COMMAND] * (replays + 1)
        )
        self.assertIsNone(self.bus._next_seq)
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

    def test_out_of_sequence_and_downstream_unknown_invalidate_session(self):
        for flags, status in ((0x89, "OUT_OF_SEQUENCE"), (0x8A, "DOWNSTREAM_UNKNOWN")):
            with self.subTest(status=status):
                self.bus.close()
                self._open()
                self.events.extend(
                    (
                        ("write", COMMAND, None),
                        ("read", append_crc(bytes((flags, 1, 0, 0))), 0),
                    )
                )
                with self.assertRaisesRegex(I2cError, status):
                    self.bus.write_command(b"\xae")
                self.assertIsNone(self.bus._next_seq)
                with self.assertRaisesRegex(I2cError, "会话失效"):
                    self.bus.read_byte()
                self.assertFalse(self.events)

    def test_sequence_wraps_after_ffff(self):
        self._open()
        self.bus._next_seq = 0xFFFF
        self.events.extend(
            (
                ("write", append_crc(bytes.fromhex("00 01 FF FF AE")), None),
                ("read", append_crc(bytes.fromhex("83 FF FF 00")), 0),
                ("write", append_crc(bytes.fromhex("00 01 00 00 AE")), None),
                ("read", append_crc(bytes.fromhex("42 00 00 00")), 0),
                ("read", bytes.fromhex("83 00 00 00 29 A0"), 0),
            )
        )
        self.bus.write_command(b"\xae")
        self.bus.write_command(b"\xae")
        self.assertEqual(self.bus._next_seq, 1)
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

    def test_nonquery_receipt_with_matching_sequence_requires_zero_value(self):
        for kind in (0x00, 0x40):
            with self.subTest(kind=kind):
                self.bus.close()
                self._open()
                expected = append_crc(bytes((kind, 1, 1, 0, 0xAE)))
                self.events.extend(
                    (
                        ("write", expected, None),
                        ("read", append_crc(b"\x83\x01\x00\x81"), 0),
                    )
                )
                write = self.bus.write_command if kind == 0x00 else self.bus.write_data
                with self.assertRaisesRegex(I2cError, "非查询帧回执的 value 非零"):
                    write(b"\xae")
                self.assertIsNone(self.bus._next_seq)
                self.assertFalse(self.events)

    def test_stale_panel_status_value_does_not_invalidate_current_write(self):
        self._open()
        query_reply = append_crc(b"\x83\x01\x00\x81")
        self.events.extend(
            (
                ("write", append_crc(b"\x01\x00\x01\x00"), None),
                ("read", query_reply, 0),
                ("write", DATA, None),
                ("read", query_reply, 0),
                ("read", OK_DATA, 0),
            )
        )
        self.assertEqual(self.bus.read_byte(), 0x81)
        self.bus.write_data(b"\x01\x02")
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
