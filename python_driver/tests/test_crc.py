"""Modbus CRC16 与协议 v2 帧格式测试，无需硬件"""

import unittest

from ssd1309_driver.crc import append_crc, modbus_crc16, strip_and_verify


class ModbusCrc16Test(unittest.TestCase):
    def test_check_value(self):
        self.assertEqual(modbus_crc16(b"123456789"), 0x4B37)

    def test_empty_input_returns_init_value(self):
        self.assertEqual(modbus_crc16(b""), 0xFFFF)

    def test_modbus_rtu_reference_vector(self):
        self.assertEqual(modbus_crc16(bytes.fromhex("01 03 00 00 00 0A")), 0xCDC5)

    def test_append_writes_low_byte_first(self):
        self.assertEqual(
            append_crc(bytes.fromhex("00 01 AE")),
            bytes.fromhex("00 01 AE F1 EC"),
        )

    def test_frame_vectors(self):
        vectors = {
            "00 01 AE": "00 01 AE F1 EC",
            "00 02 81 CF": "00 02 81 CF 81 E0",
            "00 02 20 02": "00 02 20 02 39 E5",
            "00 01 B0": "00 01 B0 71 E4",
            "00 02 00 10": "00 02 00 10 A0 28",
            "40 02 01 02": "40 02 01 02 34 75",
            "00 00": "00 00 01 B0",
        }
        for plain, framed in vectors.items():
            with self.subTest(plain=plain):
                self.assertEqual(
                    append_crc(bytes.fromhex(plain)), bytes.fromhex(framed)
                )

    def test_strip_and_verify_roundtrip(self):
        self.assertEqual(
            strip_and_verify(bytes.fromhex("40 02 01 02 34 75")),
            bytes.fromhex("40 02 01 02"),
        )

    def test_strip_rejects_corrupted_frame(self):
        with self.assertRaises(ValueError):
            strip_and_verify(bytes.fromhex("00 01 AE F1 ED"))

    def test_strip_rejects_short_frame(self):
        with self.assertRaises(ValueError):
            strip_and_verify(b"")
        with self.assertRaises(ValueError):
            strip_and_verify(b"\x00")


if __name__ == "__main__":
    unittest.main()
