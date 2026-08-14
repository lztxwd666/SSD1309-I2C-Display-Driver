//! 综合演示：循环展示文字、基础图形、日志/告警、反色与对比度。
//!
//! 运行：cargo run --example demo
//! 使用 render_robust() 推帧，演示自动恢复策略（Ctrl+C 停止）。

use std::thread;
use std::time::Duration;

use i2c_display_driver::DriverError;
use i2c_display_driver::display::Display;
use i2c_display_driver::graphics::{canvas, text};

/// 每帧停留时长（毫秒）。
const FRAME_MS: u64 = 2000;

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;

    println!("[演示] 开始循环显示，Ctrl+C 停止");

    loop {
        show_text(&mut display)?;
        show_shapes(&mut display)?;
        show_logs(&mut display)?;
        show_inversion(&mut display)?;
    }
}

/// 帧 1：文字 —— 5×7 标准 / 紧凑 / 反色。
fn show_text(display: &mut Display) -> Result<(), DriverError> {
    display.framebuffer.clear();

    text::draw_text(&mut display.framebuffer, 0, 0, "Text 5x7");
    text::draw_text_packed(&mut display.framebuffer, 0, 10, "PACKED");
    text::draw_text(&mut display.framebuffer, 0, 20, "normal text");

    // 反色文字：先填充白色背景，再清除字形像素
    canvas::fill_rect(&mut display.framebuffer, 0, 40, 128, 16);
    text::draw_text_inverted(&mut display.framebuffer, 2, 44, "INVERTED");

    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS));
    Ok(())
}

/// 帧 2：基础图形 —— 矩形 / 直线 / 圆 / 三角形 / 点线。
fn show_shapes(display: &mut Display) -> Result<(), DriverError> {
    display.framebuffer.clear();

    canvas::draw_rect(&mut display.framebuffer, 0, 0, 40, 20);
    canvas::fill_rect(&mut display.framebuffer, 50, 0, 40, 20);
    canvas::draw_line(&mut display.framebuffer, 0, 30, 40, 50);
    canvas::draw_hline(&mut display.framebuffer, 50, 30, 40);
    canvas::draw_vline(&mut display.framebuffer, 100, 30, 20);
    canvas::draw_circle(&mut display.framebuffer, 20, 40, 12);
    canvas::fill_circle(&mut display.framebuffer, 60, 40, 10);
    canvas::draw_triangle(&mut display.framebuffer, 80, 50, 120, 50, 100, 25);
    canvas::fill_triangle(&mut display.framebuffer, 80, 60, 120, 60, 100, 35);
    canvas::draw_hline_dotted(&mut display.framebuffer, 0, 63, 128);

    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS));
    Ok(())
}

/// 帧 3：日志 / 告警模拟 —— 反色标题 + 5×7 日志行 + 进度条。
fn show_logs(display: &mut Display) -> Result<(), DriverError> {
    display.framebuffer.clear();

    // 告警标题（反色条）
    canvas::fill_rect(&mut display.framebuffer, 0, 0, 128, 9);
    text::draw_text_inverted(&mut display.framebuffer, 2, 1, "ALERT");

    // 日志行（5×7）
    text::draw_text(&mut display.framebuffer, 0, 12, "[12:00:01] CPU 72C");
    text::draw_text(&mut display.framebuffer, 0, 20, "[12:00:02] mem 85%");
    text::draw_text(&mut display.framebuffer, 0, 28, "[12:00:03] disk 90%");
    text::draw_text(&mut display.framebuffer, 0, 36, "[12:00:04] ssh login");

    // 进度条（矩形边框 + 填充）
    canvas::draw_rect(&mut display.framebuffer, 0, 50, 100, 8);
    canvas::fill_rect(&mut display.framebuffer, 1, 51, 70, 6);

    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS));
    Ok(())
}

/// 帧 4：反色显示 + 对比度 —— 硬件级反色与亮度切换。
fn show_inversion(display: &mut Display) -> Result<(), DriverError> {
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 24, "CONTRAST");
    canvas::fill_rect(&mut display.framebuffer, 0, 40, 128, 16);

    // 正常显示 + 最高对比度
    display.set_inverted(false)?;
    display.set_contrast(0xCF)?;
    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS / 2));

    // 反色显示
    display.set_inverted(true)?;
    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS / 2));

    // 降低对比度
    display.set_contrast(0x40)?;
    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS / 2));

    // 恢复
    display.set_inverted(false)?;
    display.set_contrast(0xCF)?;
    display.render_robust();
    thread::sleep(Duration::from_millis(FRAME_MS / 2));

    Ok(())
}
