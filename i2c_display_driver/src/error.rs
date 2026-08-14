//! 统一错误类型。

use std::fmt;

/// 驱动全局错误。
#[derive(Debug)]
pub enum DriverError {
    /// 底层 I/O 错误。
    Io(std::io::Error),
    /// 驱动未初始化（recover 失败后）。
    NotInitialized,
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O 错误: {e}"),
            Self::NotInitialized => write!(f, "驱动未初始化"),
        }
    }
}

impl std::error::Error for DriverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::NotInitialized => None,
        }
    }
}

impl From<std::io::Error> for DriverError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
