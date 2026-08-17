//! 文本渲染器 —— 5×7 字体。
//!
//! * `draw_text` / `draw_text_packed` — 5×7 标准字体
//! * `draw_text_inverted` — 反色模式（需先填充背景）

use super::font;
use crate::display::Framebuffer;

enum DrawMode {
    Set,
    Clear,
}

pub fn draw_text(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 1, DrawMode::Set);
}

pub fn draw_text_packed(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 0, DrawMode::Set);
}

pub fn draw_text_inverted(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 1, DrawMode::Clear);
}

fn draw_impl(
    fb: &mut Framebuffer,
    x: usize,
    y: usize,
    text: &str,
    gap: usize,
    mode: DrawMode,
) {
    let mut cx = x;
    for ch in text.chars() {
        if ch == '\n' {
            cx = x;
            continue;
        }
        // 字符完全在屏幕右侧之外 → 整行提前退出
        if cx >= 128 {
            break;
        }
        let glyph = font::glyph(ch);
        for (col, &col_data) in glyph.iter().enumerate() {
            let px = cx + col;
            if px >= 128 {
                break;
            }
            for bit in 0..font::CHAR_HEIGHT {
                let py = y + bit;
                if py >= 64 {
                    break;
                }
                if (col_data & (1 << bit)) != 0 {
                    let on = matches!(mode, DrawMode::Set);
                    fb.set_pixel(px, py, on);
                }
            }
        }
        cx += font::CHAR_WIDTH + gap;
    }
}

/// 估算文本像素宽度（5×7）。
#[inline]
pub fn text_width(text: &str, gap: usize) -> usize {
    text.chars().count() * (font::CHAR_WIDTH + gap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_text_ok() {
        let mut fb = Framebuffer::new();
        draw_text(&mut fb, 0, 0, "OK");
    }

    #[test]
    fn degree_renders() {
        let mut fb = Framebuffer::new();
        draw_text(&mut fb, 0, 0, "48.5°C");
    }

    #[test]
    fn inverted_works() {
        let mut fb = Framebuffer::new();
        for px in 0..40 {
            for py in 0..10 {
                fb.set_pixel(px, py, true);
            }
        }
        draw_text_inverted(&mut fb, 0, 0, "OK");
        assert!(!fb.get_pixel(0, 3));
    }

    #[test]
    fn packed_is_narrower() {
        assert!(text_width("CPU", 0) < text_width("CPU", 1));
    }

    #[test]
    fn oob_no_panic() {
        let mut fb = Framebuffer::new();
        draw_text(&mut fb, 200, 200, "X");
    }
}
