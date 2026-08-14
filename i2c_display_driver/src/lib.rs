//! I2C 显示屏驱动库。
//!
//! 目前提供 SSD1309（128×64 单色 OLED）驱动，基于 Linux I2C：
//! * [`display::Display`] — 顶层句柄（open / render / set_contrast / sleep / recover）
//! * [`display::Framebuffer`] — 1-bit 帧缓冲区，实现 embedded-graphics `DrawTarget`

pub mod display;
pub mod utils;
