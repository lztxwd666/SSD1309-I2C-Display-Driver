//! SSD1309 I2C OLED 显示屏驱动库。
//!
//! 针对项目控制器特化（128×64 单色 OLED），面向 Linux I2C 子系统。
//!
//! # 能力概览
//!
//! * [`display::Display`] — 顶层句柄：打开/推帧/局部推帧/自动恢复/对比度/反色/休眠/唤醒/水平与垂直滚动
//! * [`display::DisplayConfig`] — 可配置初始化（总线、地址、初始对比度/反色/显示开关）
//! * [`display::Framebuffer`] — 1-bit 帧缓冲（含脏矩形跟踪），实现 embedded-graphics `DrawTarget`
//! * [`display::DriverStats`] — 运行统计（帧数/错误数/恢复次数）
//! * [`graphics`] — 文字（5×7）与基础图形绘制
//! * [`DriverError`] — 统一错误类型
//!
//! # 示例
//!
//! ```no_run
//! use i2c_display_driver::display::Display;
//! use i2c_display_driver::graphics::text;
//!
//! // 打开显示（总线 1，地址 0x3C）
//! let mut display = Display::open(1, 0x3C)?;
//!
//! // 绘制并推帧
//! display.framebuffer.clear();
//! text::draw_text(&mut display.framebuffer, 0, 0, "Hello SSD1309!");
//! display.render()?;
//!
//! // 程序退出前休眠（可选）
//! display.sleep()?;
//! # Ok::<(), i2c_display_driver::DriverError>(())
//! ```

pub mod display;
pub mod error;
pub mod graphics;

pub use error::DriverError;
