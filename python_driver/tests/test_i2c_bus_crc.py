"""I2cBus 帧封装、NACK 重试与状态读校验测试，无需硬件"""

import errno
import unittest
from unittest import mock

from ssd1309_driver.errors import FrameRejectedError, I2cError, InvalidDataError
from ssd1309_driver.i2c_bus import I2cBus


class I2cBusFramingTest(unittest.TestCase):
    def _open_bus(self, response: bytes = b"\x01\x7e\x80", write_behavior=None):
        writes = []

        def fake_write(_fd, data):
            writes.append(bytes(data))
            return len(data)

        patchers = [
            mock.patch("ssd1309_driver.i2c_bus.os.open", return_value=99),
            mock.patch("ssd1309_driver.i2c_bus.fcntl.ioctl", return_value=0),
            mock.patch(
                "ssd1309_driver.i2c_bus.os.write",
                side_effect=(
                    write_behavior if write_behavior is not None else fake_write
                ),
            ),
            mock.patch("ssd1309_driver.i2c_bus.os.read", return_value=response),
            mock.patch("ssd1309_driver.i2c_bus.os.close"),
        ]
        for patcher in patchers:
            patcher.start()
            self.addCleanup(patcher.stop)
        bus = I2cBus(1, 0x3C).open()
        return bus, writes

    def test_write_command_frame(self):
        bus, writes = self._open_bus()
        bus.write_command(bytes.fromhex("AE"))
        self.assertEqual(writes, [bytes.fromhex("00 01 AE F1 EC")])

    def test_write_data_frame(self):
        bus, writes = self._open_bus()
        bus.write_data(bytes.fromhex("01 02"))
        self.assertEqual(writes, [bytes.fromhex("40 02 01 02 34 75")])

    def test_read_byte_sends_request_and_verifies(self):
        bus, writes = self._open_bus(response=bytes.fromhex("01 7E 80"))
        self.assertEqual(bus.read_byte(), 0x01)
        self.assertEqual(writes, [bytes.fromhex("00 00 01 B0")])

    def test_read_byte_rejects_corrupted_response(self):
        bus, writes = self._open_bus(response=bytes.fromhex("01 00 00"))
        with self.assertRaises(FrameRejectedError):
            bus.read_byte()
        self.assertEqual(writes, [bytes.fromhex("00 00 01 B0")])

    def test_read_byte_rejects_short_response(self):
        bus, _writes = self._open_bus(response=b"\x01")
        with self.assertRaises(FrameRejectedError):
            bus.read_byte()

    def test_write_frame_retries_after_nack(self):
        calls = {"n": 0}
        writes = []

        def flaky_write(_fd, data):
            calls["n"] += 1
            if calls["n"] <= 2:
                raise OSError(errno.EREMOTEIO, "nack")
            writes.append(bytes(data))
            return len(data)

        bus, _ = self._open_bus(write_behavior=flaky_write)
        bus.write_command(bytes.fromhex("AE"))
        self.assertEqual(calls["n"], 3)
        self.assertEqual(writes, [bytes.fromhex("00 01 AE F1 EC")])

    def test_write_frame_reports_rejection_after_retries(self):
        calls = {"n": 0}

        def always_nack(_fd, _data):
            calls["n"] += 1
            raise OSError(errno.EREMOTEIO, "nack")

        bus, _ = self._open_bus(write_behavior=always_nack)
        with self.assertRaises(FrameRejectedError):
            bus.write_command(bytes.fromhex("AE"))
        self.assertEqual(calls["n"], 3)

    def test_non_nack_error_is_not_retried(self):
        calls = {"n": 0}

        def broken_write(_fd, _data):
            calls["n"] += 1
            raise OSError(errno.EIO, "bus error")

        bus, _ = self._open_bus(write_behavior=broken_write)
        with self.assertRaises(I2cError) as ctx:
            bus.write_command(bytes.fromhex("AE"))
        self.assertNotIsInstance(ctx.exception, FrameRejectedError)
        self.assertEqual(calls["n"], 1)

    def test_oversize_payload_is_rejected(self):
        bus, writes = self._open_bus()
        with self.assertRaises(InvalidDataError):
            bus.write_data(b"\x00" * 256)
        self.assertEqual(writes, [])


if __name__ == "__main__":
    unittest.main()
