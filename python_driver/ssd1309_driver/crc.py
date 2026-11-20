"""Modbus CRC16 帧校验

参数与 Modbus RTU 一致：多项式 0x8005（反射值 0xA001）、初值 0xFFFF、
输入/输出反射、最终不取反。CRC 按低字节在前追加到帧末尾。
"""

INIT = 0xFFFF
POLY = 0xA001
MASK = 0xFFFF


def modbus_crc16(data: bytes) -> int:
    """计算 Modbus CRC16（CRC-16/MODBUS）。

    标准检查值：modbus_crc16(b"123456789") == 0x4B37。
    """
    crc = INIT
    for byte in data:
        crc ^= byte
        for _ in range(8):
            if crc & 0x0001:
                crc = (crc >> 1) ^ POLY
            else:
                crc >>= 1
    return crc & MASK


def append_crc(data: bytes) -> bytes:
    """在帧末尾追加 CRC，低字节在前。"""
    crc = modbus_crc16(data)
    return data + bytes((crc & 0xFF, crc >> 8))


def strip_and_verify(frame: bytes) -> bytes:
    """校验末尾 2 字节 CRC。

    成功时返回去掉 CRC 的帧体；长度不足或 CRC 不符时抛出 ValueError。
    """
    if len(frame) < 2:
        raise ValueError(f"帧长度不足: {len(frame)} < 2")
    body = frame[:-2]
    tail = frame[-2:]
    expected = modbus_crc16(body)
    received = tail[0] | (tail[1] << 8)
    if received != expected:
        raise ValueError(f"CRC 校验失败: 接收 0x{received:04X}，计算 0x{expected:04X}")
    return body
