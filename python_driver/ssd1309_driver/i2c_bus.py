"""Linux I2C 总线底层封装

直接操作 /dev/i2c-N，通过 ioctl 绑定从机地址后使用 write/read 通信。

写帧格式（协议 v2）：
- 命令/数据帧：控制字节 + 1 字节载荷长度 + 载荷 + Modbus CRC16
- 状态读请求：0x00 0x00 + CRC
- 状态读响应：状态字节 + CRC（3 字节）

CRC 覆盖控制字节、长度字节与载荷，低字节在前。接收端根据长度字段确定
帧尾，在最后一个字节（CRC_H）的 ACK 周期校验长度与 CRC，失败时 NACK；
Linux i2c-dev 将数据 NACK 映射为 EREMOTEIO，本层据此重试并上报。
帧格式见项目 docs/i2c-crc-protocol.md。
"""

import errno
import fcntl
import os
from typing import Optional

from .crc import append_crc, strip_and_verify
from .errors import FrameRejectedError, I2cError, InvalidDataError

I2C_SLAVE = 0x0703

# 单帧命令/数据载荷上限
MAX_PAYLOAD = 255
# 接收端 NACK 后的重试次数（不含首次发送）
FRAME_RETRIES = 2
# Linux 数据 NACK 的 errno；非 Linux 平台回退到 121
EREMOTEIO = getattr(errno, "EREMOTEIO", 121)


class I2cBus:
    """Linux I2C 设备总线"""

    def __init__(self, bus_id: int, addr: int):
        self.bus_id = bus_id
        self.addr = addr
        self._fd: Optional[int] = None

    def open(self) -> "I2cBus":
        """打开 /dev/i2c-{bus} 并绑定从机地址"""
        path = f"/dev/i2c-{self.bus_id}"
        fd = os.open(path, os.O_RDWR)
        try:
            fcntl.ioctl(fd, I2C_SLAVE, self.addr)
        except OSError as e:
            os.close(fd)
            raise I2cError(
                f"无法打开 I2C 设备 {path} 或设置从机地址 {self.addr:#x}", e
            ) from e
        self._fd = fd
        return self

    def close(self) -> None:
        """关闭 I2C 设备"""
        if self._fd is not None:
            os.close(self._fd)
            self._fd = None

    def __enter__(self) -> "I2cBus":
        return self.open()

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    @staticmethod
    def _build_frame(control: int, data: bytes) -> bytes:
        """组装写帧：控制字节 + 载荷长度 + 载荷 + CRC（低字节在前）。"""
        if len(data) > MAX_PAYLOAD:
            raise InvalidDataError(f"I2C 帧载荷过长: {len(data)} > {MAX_PAYLOAD}")
        body = bytes((control, len(data))) + data
        return append_crc(body)

    @staticmethod
    def _is_data_nack(exc: I2cError) -> bool:
        """判断异常是否由接收端数据 NACK（长度或 CRC 拒绝）引起。"""
        cause = exc.cause
        return isinstance(cause, OSError) and cause.errno == EREMOTEIO

    def _write(self, buf: bytes) -> None:
        if self._fd is None:
            raise I2cError("I2C 总线未打开")
        try:
            written = os.write(self._fd, buf)
        except OSError as e:
            raise I2cError("I2C 写入失败", e) from e
        if written != len(buf):
            raise I2cError(f"I2C 写入不完整: {written}/{len(buf)}")

    def _write_frame(self, control: int, data: bytes) -> None:
        frame = self._build_frame(control, data)
        for attempt in range(FRAME_RETRIES + 1):
            try:
                self._write(frame)
                return
            except I2cError as e:
                if not self._is_data_nack(e):
                    raise
                if attempt >= FRAME_RETRIES:
                    raise FrameRejectedError(
                        "I2C 帧被接收端拒绝"
                        f"（长度或 CRC 校验失败，已重试 {FRAME_RETRIES} 次）",
                        e,
                    ) from e

    def write_command(self, data: bytes) -> None:
        """发送命令（0x00 控制字节前缀，末尾 Modbus CRC16）"""
        self._write_frame(0x00, data)

    def write_data(self, data: bytes) -> None:
        """发送 GDDRAM 数据（0x40 控制字节前缀，末尾 Modbus CRC16）"""
        self._write_frame(0x40, data)

    def read_byte(self) -> int:
        """读取一个字节（SSD1309 状态寄存器）

        先发送状态读请求帧（0x00 0x00 + CRC，线上 00 00 01 B0），再读取
        3 字节响应（状态字节 + CRC_L + CRC_H），校验通过后返回状态字节。
        """
        if self._fd is None:
            raise I2cError("I2C 总线未打开")
        self._write_frame(0x00, b"")
        try:
            buf = os.read(self._fd, 3)
        except OSError as e:
            raise I2cError("I2C 读取失败", e) from e
        if len(buf) != 3:
            raise FrameRejectedError(
                f"I2C 状态帧长度错误: 期望 3 字节，实际 {len(buf)} 字节"
            )
        try:
            body = strip_and_verify(buf)
        except ValueError as e:
            raise FrameRejectedError("I2C 状态帧 CRC 校验失败", e) from e
        return body[0]
