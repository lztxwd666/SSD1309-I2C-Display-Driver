//! SSD1309 I2C OLED 显示屏驱动库。
//!
//! 针对项目控制器特化（128×64 单色 OLED），面向 Linux I2C 子系统。
//!
//! * [`display::Display`] — 顶层句柄（open / render / render_robust / recover / sleep / wake / set_contrast / set_inverted）
//! * [`display::Framebuffer`] — 1-bit 帧缓冲，实现 embedded-graphics `DrawTarget`
//! * [`graphics`] — 文字（5×7 + 4×6）与基础图形绘制
//! * [`DriverError`] — 统一错误类型

pub mod display;
pub mod error;
pub mod graphics;

pub use error::DriverError;
