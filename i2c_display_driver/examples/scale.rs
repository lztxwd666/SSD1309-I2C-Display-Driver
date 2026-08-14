//! 诊断示例：把 4×6 小字体放大 4 倍显示，验证字形数据是否正确。
//!
//! 运行：cargo run --example scale
//! 若放大后的字形清晰正确，说明字形数据没问题，问题在 4px 物理尺寸太小；
//! 若放大后仍然乱，说明渲染或数据有问题。

use i2c_display_driver::display::{Display, Framebuffer};
use i2c_display_driver::graphics::font;
use i2c_display_driver::DriverError;

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;
    display.framebuffer.clear();

    // 每字符放大 4 倍（16×24 像素），上排数字，下排字母
    draw_string_scaled(&mut display.framebuffer, 0, 0, "0123", 4);
    draw_string_scaled(&mut display.framebuffer, 0, 28, "Abg8", 4);

    display.render()?;
    println!("[诊断] 已放大显示 4×6 字形，请观察字形是否正确");
    Ok(())
}

/// 以 scale 倍放大绘制一串小字体字符。
fn draw_string_scaled(fb: &mut Framebuffer, x: usize, y: usize, s: &str, scale: usize) {
    let mut cx = x;
    for ch in s.chars() {
        let glyph = font::glyph_small(ch);
        draw_glyph_scaled(fb, cx, y, glyph, 4, 6, scale);
        cx += 4 * scale + scale;
    }
}

/// 把单个字形（列主序字节）放大绘制。
fn draw_glyph_scaled(
    fb: &mut Framebuffer,
    x: usize,
    y: usize,
    glyph: &[u8],
    w: usize,
    h: usize,
    scale: usize,
) {
    for col in 0..w {
        for row in 0..h {
            if (glyph[col] & (1 << row)) != 0 {
                for dy in 0..scale {
                    for dx in 0..scale {
                        fb.set_pixel(x + col * scale + dx, y + row * scale + dy, true);
                    }
                }
            }
        }
    }
}
