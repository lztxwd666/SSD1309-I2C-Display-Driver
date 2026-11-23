"""Modbus CRC16 和 v3 写帧、回执的一致性测试。"""

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
            append_crc(bytes.fromhex("00 01 01 00 AE")),
            bytes.fromhex("00 01 01 00 AE F5 80"),
        )

    def test_frame_vectors(self):
        vectors = {
            "7F 00 00 00": "7F 00 00 00 19 F0",
            "00 01 01 00 AE": "00 01 01 00 AE F5 80",
            "40 02 02 00 01 02": "40 02 02 00 01 02 F6 F2",
            "01 00 03 00": "01 00 03 00 01 28",
            "01 00 00 00": "01 00 00 00 01 D8",
            "83 03 00 81": "83 03 00 81 19 C0",
        }
        for plain, framed in vectors.items():
            with self.subTest(plain=plain):
                self.assertEqual(
                    append_crc(bytes.fromhex(plain)), bytes.fromhex(framed)
                )

    def test_strip_and_verify_roundtrip(self):
        self.assertEqual(
            strip_and_verify(bytes.fromhex("83 03 00 81 19 C0")),
            bytes.fromhex("83 03 00 81"),
        )

    def test_strip_rejects_corrupted_frame(self):
        with self.assertRaises(ValueError):
            strip_and_verify(bytes.fromhex("83 03 00 81 19 C1"))

    def test_strip_rejects_short_frame(self):
        with self.assertRaises(ValueError):
            strip_and_verify(b"")
        with self.assertRaises(ValueError):
            strip_and_verify(b"\x00")


if __name__ == "__main__":
    unittest.main()
