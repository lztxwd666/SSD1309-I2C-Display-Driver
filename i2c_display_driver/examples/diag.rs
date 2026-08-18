//! 硬件诊断 + 软件滚动演示。
//!
//! 运行：cargo run --example diag
//!
//! 诊断结论（已实测）：本屏幕不响应硬件滚动命令（0x26/0x27/0x2F），
//! 需使用软件滚动（帧缓冲平移 + 推帧）。
//!
//! 分步演示：
//! 1. 全帧推两行文字 → 验证全帧路径
//! 2. 脏矩形局部推第一行（page0）→ 验证局部推送
//! 3. 脏矩形局部推第二行（page1-2）→ 验证跨页局部推送
//! 4. 软件滚动循环右移 4 秒 → 观察跑马灯效果
//! 5. 软件滚动循环左移 4 秒 → 对比反向
//! 6. 恢复正常显示

use std::thread;
use std::time::Duration;

use i2c_display_driver::DriverError;
use i2c_display_driver::display::{Display, ScrollDirection};
use i2c_display_driver::graphics::text;

fn pause(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;
    display.set_logger(|msg| println!("  [驱动日志] {msg}"));

    // 1. 全帧推两行文字
    println!("[1/6] 全帧 render() 推两行文字，观察 8 秒");
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, "FULL PAGE0");
    text::draw_text(&mut display.framebuffer, 0, 12, "FULL PAGE1");
    display.render()?;
    pause(8000);

    // 2. 局部推第一行（page0）
    println!("[2/6] render_dirty() 只推第一行（page0），观察 6 秒");
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, "DIRTY PAGE0");
    println!("      脏矩形: {:?}", display.framebuffer.dirty_rect());
    display.render_dirty()?;
    pause(6000);

    // 3. 局部推第二行（page1-2）
    println!("[3/6] 追加第二行后 render_dirty()，观察 6 秒");
    text::draw_text(&mut display.framebuffer, 0, 12, "DIRTY PAGE1");
    println!("      脏矩形: {:?}", display.framebuffer.dirty_rect());
    display.render_dirty()?;
    pause(6000);

    // 4. 软件滚动：循环右移（80ms/像素，持续 4 秒）
    println!("[4/6] 软件滚动循环右移 4 秒");
    for _ in 0..50 {
        display.software_scroll_horizontal(ScrollDirection::Right, 1)?;
        pause(80);
    }

    // 5. 软件滚动：循环左移
    println!("[5/6] 软件滚动循环左移 4 秒");
    for _ in 0..50 {
        display.software_scroll_horizontal(ScrollDirection::Left, 1)?;
        pause(80);
    }

    // 6. 恢复正常显示
    println!("[6/6] 恢复正常显示");
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 24, "DIAG DONE");
    display.render()?;
    println!("诊断完成。请反馈：每步观察到的屏幕现象。");
    Ok(())
}
