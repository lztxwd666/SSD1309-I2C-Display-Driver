//! 冒烟测试：初始化显示 → 绘制文字与基础图形 → 推帧。
//!
//! 运行：cargo run --example smoke
//! 用于快速验证 I2C 通信、帧缓冲、文字、基础图形是否正常。

use i2c_display_driver::display::Display;
use i2c_display_driver::graphics::{canvas, text};
use i2c_display_driver::DriverError;

fn main() -> Result<(), DriverError> {
    // I2C 总线 1，SSD1309 地址 0x3C
    let mut display = Display::open(1, 0x3C)?;

    // 清屏
    display.framebuffer.clear();

    // 5×7 标准字体（每字符 6px 宽，含 1px 间距）
    text::draw_text(&mut display.framebuffer, 0, 0, "Hello SSD1309!");
    text::draw_text(&mut display.framebuffer, 0, 12, "i2c_display_driver");
    text::draw_text(&mut display.framebuffer, 0, 20, "128x64 monochrome");

    // 基础图形
    canvas::draw_rect(&mut display.framebuffer, 0, 32, 60, 20);
    canvas::fill_rect(&mut display.framebuffer, 4, 36, 30, 12);
    canvas::draw_circle(&mut display.framebuffer, 90, 42, 14);
    canvas::fill_circle(&mut display.framebuffer, 110, 20, 6);

    // 推帧到屏幕
    display.render()?;

    println!("[冒烟测试] 已绘制并推帧，程序退出后屏幕保持显示");
    Ok(())
}
