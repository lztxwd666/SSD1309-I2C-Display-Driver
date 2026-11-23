"""Linux I2C v3 发送端，使用独立写事务和读事务。

非查询帧的匹配回执要求 value 为 0。帧格式见 docs/i2c-crc-protocol.md。
"""

import fcntl
import os
import time
from enum import IntEnum
from threading import Lock
from typing import Optional

from .crc import append_crc, strip_and_verify
from .errors import I2cError, InvalidDataError

I2C_SLAVE = 0x0703
I2C_TIMEOUT = 0x0702
T_IO_10MS = 10

COMMAND = 0x00
DATA = 0x40
PANEL_STATUS = 0x01
SYNC = 0x7F
MAX_PAYLOAD = 255
RECEIPT_LEN = 6
LAST_OK_VALID = 0x80
BUSY = 0x40

T_PUBLISH_NS = 5_000_000
T_RESULT_NS = 100_000_000
T_POLL_NS = 2_000_000
T_FRAME_NS = 500_000_000


class _Status(IntEnum):
    NEED_SYNC = 0x00
    SYNCED = 0x01
    PENDING = 0x02
    OK = 0x03
    CRC_ERROR = 0x04
    FORMAT_ERROR = 0x05
    BUSY_DROP = 0x06
    RX_OVERFLOW = 0x07
    RX_TIMEOUT = 0x08
    OUT_OF_SEQUENCE = 0x09
    DOWNSTREAM_UNKNOWN = 0x0A


_SYNC_REQUIRED = (
    _Status.NEED_SYNC,
    _Status.OUT_OF_SEQUENCE,
    _Status.DOWNSTREAM_UNKNOWN,
)
_EARLY_RETRY = (
    _Status.CRC_ERROR,
    _Status.FORMAT_ERROR,
    _Status.BUSY_DROP,
    _Status.RX_TIMEOUT,
)


def _exchange(fd: int, kind: int, seq: Optional[int], payload: bytes) -> int:
    syncing = seq is None
    sequence = 0 if seq is None else seq
    frame = append_crc(
        bytes((kind, len(payload))) + sequence.to_bytes(2, "little") + payload
    )
    deadline = time.monotonic_ns() + T_FRAME_NS
    detail = "未读取到回执"
    cause = None
    while time.monotonic_ns() < deadline:
        # 短写时整帧重试，剩余字节不能作为新事务发送。
        try:
            count = os.write(fd, frame)
            if count != len(frame):
                detail, cause = f"短写 {count}/{len(frame)}", None
        except OSError as exc:
            detail, cause = "写事务失败", exc
        written_at = time.monotonic_ns()
        if syncing and written_at < deadline:
            time.sleep(min(T_PUBLISH_NS, deadline - written_at) / 1_000_000_000)

        busy = False
        failed_reads = 0
        while time.monotonic_ns() < deadline:
            body = None
            bus_readable = False
            try:
                reply = os.read(fd, RECEIPT_LEN)
                bus_readable = bool(reply)
                if len(reply) != RECEIPT_LEN:
                    detail, cause = f"回执短读 {len(reply)}/{RECEIPT_LEN}", None
                else:
                    body = strip_and_verify(reply)
            except OSError as exc:
                detail, cause = "读事务失败", exc
            except ValueError as exc:
                detail, cause = str(exc), None
            now = time.monotonic_ns()
            if now >= deadline:
                break

            retry_after = T_RESULT_NS
            if body is not None:
                failed_reads = 0
                flags, seq_l, seq_h, value = body
                code = flags & 0x3F
                try:
                    status = _Status(code)
                except ValueError as exc:
                    raise I2cError(
                        f"I2C v3 回执状态码未知: 0x{code:02X}，结果未知"
                    ) from exc
                last_ok_valid = bool(flags & LAST_OK_VALID)
                busy = bool(flags & BUSY)
                if (
                    not last_ok_valid
                    and (status == _Status.OK or seq_l or seq_h or value)
                ) or (status == _Status.SYNCED and last_ok_valid):
                    raise I2cError("I2C v3 回执字段不一致，结果未知")
                if not syncing and status in _SYNC_REQUIRED:
                    raise I2cError(
                        f"I2C v3 会话失效（{status.name}），结果未知，须重新打开并初始化面板"
                    )
                if last_ok_valid and seq == (seq_l | (seq_h << 8)):
                    if kind != PANEL_STATUS and value != 0:
                        raise I2cError("I2C v3 非查询帧回执的 value 非零，结果未知")
                    return value
                if syncing and status == _Status.SYNCED and not busy:
                    return 0
                if status in _EARLY_RETRY or (syncing and status in _SYNC_REQUIRED):
                    retry_after = T_PUBLISH_NS
                detail, cause = f"回执 {status.name}，未确认当前操作", None
            else:
                failed_reads += 1

            # 坏回执保留已知的 BUSY 状态。读事务恢复返回字节后才可重发。
            time.sleep(min(T_POLL_NS, deadline - now) / 1_000_000_000)
            if (
                bus_readable
                and not busy
                and failed_reads != 1
                and now - written_at >= retry_after
            ):
                break

    raise I2cError(f"I2C v3 确认超时，结果未知: {detail}", cause)


class I2cBus:
    """Linux I2C 总线。每个设备地址应由一个 v3 会话独占。"""

    def __init__(self, bus_id: int, addr: int):
        self.bus_id = bus_id
        self.addr = addr
        self._lock = Lock()
        self._fd: Optional[int] = None
        self._next_seq: Optional[int] = None

    def open(self) -> "I2cBus":
        """打开设备，设置适配器级事务超时并同步 v3 会话。

        超时为适配器级设置，关闭连接不会恢复原值。
        """
        with self._lock:
            if self._fd is not None:
                if self._next_seq is not None:
                    return self
                self._close_locked()
            path = f"/dev/i2c-{self.bus_id}"
            fd = None
            try:
                fd = os.open(path, os.O_RDWR)
                fcntl.ioctl(fd, I2C_SLAVE, self.addr)
                fcntl.ioctl(fd, I2C_TIMEOUT, T_IO_10MS)
                _exchange(fd, SYNC, None, b"")
                self._next_seq, self._fd = 1, fd
            except OSError as exc:
                raise I2cError(
                    f"无法打开或配置 I2C 设备 {path} 地址 {self.addr:#x}", exc
                ) from exc
            finally:
                if fd is not None and self._fd is None:
                    os.close(fd)
            return self

    def _close_locked(self) -> None:
        fd, self._fd = self._fd, None
        self._next_seq = None
        if fd is not None:
            os.close(fd)

    def close(self) -> None:
        """关闭 I2C 设备并结束会话。"""
        with self._lock:
            self._close_locked()

    def __enter__(self) -> "I2cBus":
        return self.open()

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    def _send(self, kind: int, payload: bytes) -> int:
        if kind != PANEL_STATUS and not 1 <= len(payload) <= MAX_PAYLOAD:
            raise InvalidDataError(
                f"I2C 载荷长度必须为 1–{MAX_PAYLOAD} 字节，实际 {len(payload)}"
            )
        with self._lock:
            if self._fd is None:
                raise I2cError("I2C 总线未打开")
            if self._next_seq is None:
                raise I2cError("I2C v3 会话失效，须重新打开并初始化面板")
            seq, self._next_seq = self._next_seq, None
            value = _exchange(self._fd, kind, seq, payload)
            self._next_seq = (seq + 1) & 0xFFFF
            return value

    def write_command(self, data: bytes) -> None:
        """发送 SSD1309 命令并等待确认。"""
        self._send(COMMAND, data)

    def write_data(self, data: bytes) -> None:
        """发送 GDDRAM 数据并等待确认。"""
        self._send(DATA, data)

    def read_byte(self) -> int:
        """查询 SSD1309 状态寄存器并返回一个已确认的状态字节。"""
        return self._send(PANEL_STATUS, b"")
