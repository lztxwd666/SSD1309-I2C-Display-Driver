//! 统一错误类型。

use std::fmt;
use std::io;

/// 驱动全局错误。
#[derive(Debug)]
pub enum DriverError {
    /// 底层 I/O 错误。
    Io(io::Error),
    /// 接收端拒绝写入帧（NACK），或响应帧 CRC/长度校验失败。
    FrameRejected(io::Error),
    /// 驱动未初始化（recover 失败后）。
    NotInitialized,
}

impl DriverError {
    /// 是否为帧校验失败。
    ///
    /// 写帧被接收端 NACK（长度或 CRC 校验失败），或状态读响应帧
    /// CRC/长度不符时返回 `true`。
    pub fn is_frame_rejected(&self) -> bool {
        matches!(self, Self::FrameRejected(_))
    }
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O 错误: {e}"),
            Self::FrameRejected(e) => write!(f, "I2C 帧校验失败: {e}"),
            Self::NotInitialized => write!(f, "驱动未初始化"),
        }
    }
}

impl std::error::Error for DriverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) | Self::FrameRejected(e) => Some(e),
            Self::NotInitialized => None,
        }
    }
}

impl From<io::Error> for DriverError {
    fn from(e: io::Error) -> Self {
        // I2cBus 使用 InvalidData 表示接收端 NACK 或响应帧校验失败
        if e.kind() == io::ErrorKind::InvalidData {
            Self::FrameRejected(e)
        } else {
            Self::Io(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_data_maps_to_frame_rejected() {
        let err: DriverError = io::Error::new(io::ErrorKind::InvalidData, "crc").into();
        assert!(err.is_frame_rejected());
        assert!(matches!(err, DriverError::FrameRejected(_)));
    }

    #[test]
    fn other_io_error_stays_io() {
        let err: DriverError = io::Error::other("bus").into();
        assert!(!err.is_frame_rejected());
        assert!(matches!(err, DriverError::Io(_)));
    }
}
