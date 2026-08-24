"""Linux I2C 总线底层封装

直接操作 /dev/i2c-N，通过 ioctl 绑定从机地址后使用 write/read 通信
"""

import fcntl
import os
from typing import Optional

from .errors import DriverError, I2cError

I2C_SLAVE = 0x0703


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
            raise I2cError(f"无法打开 I2C 设备 {path} 或设置从机地址 {self.addr:#x}", e) from e
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

    def _write(self, buf: bytes) -> None:
        if self._fd is None:
            raise I2cError("I2C 总线未打开")
        try:
            written = os.write(self._fd, buf)
        except OSError as e:
            raise I2cError("I2C 写入失败", e) from e
        if written != len(buf):
            raise I2cError(f"I2C 写入不完整: {written}/{len(buf)}")

    def write_command(self, data: bytes) -> None:
        """发送命令（0x00 控制字节前缀）"""
        self._write(b"\x00" + data)

    def write_data(self, data: bytes) -> None:
        """发送 GDDRAM 数据（0x40 控制字节前缀）"""
        self._write(b"\x40" + data)

    def read_byte(self) -> int:
        """读取一个字节（SSD1309 状态寄存器）"""
        if self._fd is None:
            raise I2cError("I2C 总线未打开")
        self._write(b"\x00")
        try:
            buf = os.read(self._fd, 1)
        except OSError as e:
            raise I2cError("I2C 读取失败", e) from e
        if not buf:
            raise I2cError("I2C 读取无数据返回")
        return buf[0]
