"""驱动统一异常。"""

from typing import Optional


class DriverError(Exception):
    """驱动基础异常。"""

    def __init__(self, message: str, cause: Optional[Exception] = None):
        super().__init__(message)
        self.message = message
        self.cause = cause

    def __str__(self) -> str:
        if self.cause is not None:
            return f"{self.message}: {self.cause}"
        return self.message


class I2cError(DriverError):
    """I2C 通信错误，包含协议会话失效或执行结果未知。"""


class FrameRejectedError(I2cError):
    """传输实现明确报告可安全跳过的帧校验失败。"""


class NotInitializedError(DriverError):
    """驱动未初始化。"""


class InvalidDataError(DriverError):
    """传入数据不合法或长度不足。"""
