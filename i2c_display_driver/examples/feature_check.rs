//! 新功能硬件验证：逐项演示驱动新增功能。
//!
//! 运行：cargo run --example feature_check
//!
//! 顺序演示：clear/fill → 全屏点亮测试 → render_dirty 局部推帧（含脏矩形输出）
//! → 硬件滚动 → recover 状态保持 → 统计与日志回调。每步停留数秒，观察屏幕变化。
//!
//! 对比预期：每步的屏幕表现与末尾统计（推帧数应为 6）详见各步说明。

use std::thread;
use std::time::Duration;

use i2c_display_driver::display::{Display, ScrollDirection, ScrollFrameInterval};
use i2c_display_driver::graphics::text;
use i2c_display_driver::DriverError;

fn pause(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;

    // 日志回调：驱动内部日志改由 println 输出，便于观察 recover 过程
    display.set_logger(|msg| println!("  [驱动日志] {msg}"));
    println!("=== SSD1309 新功能硬件验证 ===\n");

    // ── 1. clear() / fill() ──
    println!("[1/6] clear() 清屏 → 屏幕应全黑");
    display.clear()?;
    pause(2000);

    println!("[1/6] fill() 全屏点亮 → 屏幕应全白");
    display.fill()?;
    pause(2000);
    display.clear()?;
    pause(500);

    // ── 2. set_entire_display_on（0xA5 全屏点亮测试模式）──
    println!("[2/6] set_entire_display_on(true) → 即使 GDDRAM 为空，屏幕也应全亮（测试模式）");
    display.set_entire_display_on(true)?;
    pause(2000);
    println!("[2/6] set_entire_display_on(false) → 恢复正常（回到黑屏）");
    display.set_entire_display_on(false)?;
    pause(500);

    // ── 3. render_dirty() 局部推帧（只推送变化区域）──
    println!("[3/6] render_dirty() 局部推帧");
    text::draw_text(&mut display.framebuffer, 0, 0, "PARTIAL UPDATE");
    println!("  脏矩形（应只覆盖文字区域）: {:?}", display.framebuffer.dirty_rect());
    display.render_dirty()?;
    pause(2000);
    text::draw_text(&mut display.framebuffer, 0, 12, "dirty rect OK");
    println!("  追加一行后的脏矩形: {:?}", display.framebuffer.dirty_rect());
    display.render_dirty()?;
    pause(2000);

    // ── 4. 硬件滚动 ──
    println!("[4/6] 激活水平滚动（右移，整屏参与）→ 内容应开始滚动");
    display.scroll_horizontal(ScrollDirection::Right, 0, 7, ScrollFrameInterval::Frames5)?;
    display.activate_scroll()?;
    pause(5000);
    println!("[4/6] deactivate_scroll() → 滚动停止");
    display.deactivate_scroll()?;
    pause(1000);

    // ── 5. recover() 状态保持（对比度/反色）──
    println!("[5/6] set_contrast(0x40) + set_inverted(true)，然后 recover() 重置总线");
    display.set_contrast(0x40)?;
    display.set_inverted(true)?;
    display.recover()?;
    println!("  recover 后若设置被保持：屏幕为反色且明显变暗（对比度 0x40）");
    pause(3000);

    // ── 6. 恢复默认并输出统计 ──
    display.set_inverted(false)?;
    display.set_contrast(0xCF)?;
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 28, "FEATURES OK");
    display.render()?;

    let s = display.stats();
    println!("\n=== 统计（预期：推帧数 6，其余为 0）===");
    println!("  推帧数: {}，跳过帧: {}，恢复次数: {}，错误: {}",
        s.frames_pushed, s.frames_skipped, s.recoveries, s.errors);
    println!("\n验证结束，屏幕保持显示 FEATURES OK");
    Ok(())
}
