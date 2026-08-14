//! 统一错误类型 —— 全项目唯一错误枚举。
//!
//! 所有模块（monitor / display / engine / animation / config）
//! 统一使用 [`AppError`]，避免跨层转换。

use std::fmt;

/// 应用程序全局错误。
#[derive(Debug)]
pub enum AppError {
    /// 底层 I/O 错误。
    Io(std::io::Error),
    /// 解析失败：数据格式不符合预期。
    Parse(String),
    /// 资源未找到：文件、设备、接口等。
    NotFound(String),
    /// 配置错误：配置文件格式或值无效。
    Config(String),
    /// 系统级错误：sysconf 等系统调用返回错误。
    System(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O 错误: {e}"),
            Self::Parse(s) => write!(f, "解析错误: {s}"),
            Self::NotFound(s) => write!(f, "未找到: {s}"),
            Self::Config(s) => write!(f, "配置错误: {s}"),
            Self::System(s) => write!(f, "系统错误: {s}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
