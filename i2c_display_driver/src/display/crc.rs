//! Modbus CRC16 帧校验。
//!
//! 参数与 Modbus RTU 一致：多项式 0x8005（反射值 0xA001）、初值 0xFFFF、
//! 输入/输出反射、最终不取反。CRC 按低字节在前追加到帧末尾。

/// 计算 Modbus CRC16（CRC-16/MODBUS）。
///
/// 标准检查值：`modbus_crc16(b"123456789") == 0x4B37`。
pub(crate) fn modbus_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            if crc & 0x0001 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

/// 在 `buf[..len]` 后追加 CRC（低字节在前），返回新的帧长度。
///
/// 调用方需保证 `buf` 在 `len` 之后至少还有 2 字节空间。
pub(crate) fn append_crc(buf: &mut [u8], len: usize) -> usize {
    let crc = modbus_crc16(&buf[..len]);
    buf[len] = crc as u8;
    buf[len + 1] = (crc >> 8) as u8;
    len + 2
}

/// 校验末尾带 2 字节 CRC 的帧。
///
/// CRC 按低字节在前读取；帧长度不足 2 字节时返回 `false`。
pub(crate) fn verify_crc(frame: &[u8]) -> bool {
    if frame.len() < 2 {
        return false;
    }
    let body_len = frame.len() - 2;
    let received = u16::from_le_bytes([frame[body_len], frame[body_len + 1]]);
    received == modbus_crc16(&frame[..body_len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value_matches_modbus() {
        assert_eq!(modbus_crc16(b"123456789"), 0x4B37);
    }

    #[test]
    fn empty_input_returns_init_value() {
        assert_eq!(modbus_crc16(&[]), 0xFFFF);
    }

    #[test]
    fn modbus_rtu_reference_vector() {
        // 标准 Modbus RTU 请求帧 01 03 00 00 00 0A 的 CRC 为 0xCDC5（线上 C5 CD）
        let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
        assert_eq!(modbus_crc16(&data), 0xCDC5);
    }

    #[test]
    fn append_writes_low_byte_first() {
        let mut buf = [0u8; 4];
        buf[0] = 0x00;
        buf[1] = 0xAE;
        assert_eq!(append_crc(&mut buf, 2), 4);
        assert_eq!(buf, [0x00, 0xAE, 0x80, 0x0C]);
    }

    #[test]
    fn appended_frame_verifies_and_whole_crc_is_zero() {
        let mut frame = [0u8; 5];
        frame[0] = 0x40;
        frame[1] = 0x01;
        frame[2] = 0x02;
        let len = append_crc(&mut frame, 3);
        assert_eq!(len, 5);
        assert_eq!(frame, [0x40, 0x01, 0x02, 0xF0, 0x45]);
        assert!(verify_crc(&frame));
        // Modbus CRC 性质：含 CRC 的完整帧再次计算结果为 0
        assert_eq!(modbus_crc16(&frame), 0);
    }

    #[test]
    fn verify_rejects_corrupted_frame() {
        let mut frame = [0u8; 4];
        frame[0] = 0x00;
        frame[1] = 0xAE;
        append_crc(&mut frame, 2);
        frame[1] ^= 0x01;
        assert!(!verify_crc(&frame));
    }

    #[test]
    fn verify_rejects_short_frame() {
        assert!(!verify_crc(&[]));
        assert!(!verify_crc(&[0x00]));
    }
}
