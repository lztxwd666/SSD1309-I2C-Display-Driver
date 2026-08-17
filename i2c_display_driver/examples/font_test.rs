//! 字体测试：对比标准字形（小写已优化）与斜体效果。
//!
//! 运行：cargo run --example font_test
//! 上半屏为标准字体（小写字形已重新设计），下半屏为斜体（oblique 斜切）。

use i2c_display_driver::display::Display;
use i2c_display_driver::graphics::{canvas, text};
use i2c_display_driver::DriverError;

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;
    display.framebuffer.clear();

    // ── 上半屏：标准字体（小写字形已优化）──
    text::draw_text(&mut display.framebuffer, 0, 0, "abcdefghijklmnopqrst");
    text::draw_text(&mut display.framebuffer, 0, 9, "uvwxyz");
    text::draw_text(&mut display.framebuffer, 0, 18, "quick brown fox");

    // 分隔线
    canvas::draw_hline_dotted(&mut display.framebuffer, 0, 27, 128);

    // ── 下半屏：斜体（oblique 斜切）──
    text::draw_text_italic(&mut display.framebuffer, 0, 30, "abcdefghijklmno");
    text::draw_text_italic(&mut display.framebuffer, 0, 39, "pqrstuvwxyz");
    text::draw_text_italic(&mut display.framebuffer, 0, 48, "quick brown fox");

    display.render_robust();
    println!("[字体测试] 上半屏标准字体，下半屏斜体，已推帧");
    Ok(())
}
