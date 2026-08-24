//! 面板方向与高级参数演示。
//!
//! 运行：cargo run --example panel_tuning
//!
//! 分步演示：
//! 1. Rotate0 方向显示
//! 2. Rotate180 方向显示
//! 3. 高级寄存器调节（对比度/预充电/VCOMH/电荷泵）
//! 4. 硬件滚动兼容入口（当前项目屏幕实测不响应，仅用于其他面板验证）

use std::thread;
use std::time::Duration;

use i2c_display_driver::DriverError;
use i2c_display_driver::display::{Display, DisplayBuilder, DisplayRotation, ScrollDirection};
use i2c_display_driver::graphics::{canvas, text};

fn pause(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

fn draw_marker(display: &mut Display, label: &str) {
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, label);
    text::draw_text(&mut display.framebuffer, 0, 12, "UP");
    text::draw_text(&mut display.framebuffer, 0, 24, "↑");
    canvas::draw_rect(&mut display.framebuffer, 100, 0, 20, 20);
    canvas::fill_circle(&mut display.framebuffer, 110, 50, 8);
}

fn main() -> Result<(), DriverError> {
    let mut display = DisplayBuilder::new(1, 0x3C)
        .with_rotation(DisplayRotation::Rotate0)
        .build()?;
    display.set_logger(|msg| println!("  [驱动日志] {msg}"));

    println!("[1/4] Rotate0：屏幕应按当前安装方向显示");
    draw_marker(&mut display, "ROT 0");
    display.render()?;
    pause(3000);

    println!("[2/4] Rotate180：内容应上下左右翻转");
    display.set_rotation(DisplayRotation::Rotate180)?;
    draw_marker(&mut display, "ROT 180");
    display.render()?;
    pause(3000);

    println!("[3/4] 高级参数调节：对比度/预充电/VCOMH/电荷泵");
    display.set_rotation(DisplayRotation::Rotate0)?;
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, "ADVANCED");
    text::draw_text(&mut display.framebuffer, 0, 12, "TUNING");
    display.set_contrast(0x80)?;
    display.set_precharge_period(0x02, 0x0F)?;
    display.set_vcomh_level(0x50)?;
    display.set_charge_pump(true)?;
    display.render()?;
    pause(3000);

    println!("[4/4] 硬件滚动兼容入口：本屏可能无效果，观察 3 秒");
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, "HW SCROLL?");
    display.render()?;
    display.hardware_scroll_horizontal(ScrollDirection::Left, 0, 7, 0x00)?;
    pause(3000);
    display.deactivate_scroll()?;

    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 28, "DONE");
    display.render()?;
    println!("演示结束");
    Ok(())
}
