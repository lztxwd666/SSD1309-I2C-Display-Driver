//! 文本渲染器 —— 支持 5×7 和 4×6 两种字号。
//!
//! * `draw_text` / `draw_text_packed` — 5×7 标准字体
//! * `draw_small` / `draw_small_packed` — 4×6 小号字体（密集日志 / 告警）
//! * `draw_text_inverted` — 5×7 反色模式（需先填充背景）

use super::font;
use crate::display::Framebuffer;

enum DrawMode {
    Set,
    Clear,
}

// 5×7 标准字体

pub fn draw_text(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 1, DrawMode::Set, false);
}

pub fn draw_text_packed(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 0, DrawMode::Set, false);
}

pub fn draw_text_inverted(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 1, DrawMode::Clear, false);
}

// 4×6 小号字体

pub fn draw_small(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 1, DrawMode::Set, true);
}

pub fn draw_small_packed(fb: &mut Framebuffer, x: usize, y: usize, text: &str) {
    draw_impl(fb, x, y, text, 0, DrawMode::Set, true);
}

fn draw_impl(
    fb: &mut Framebuffer,
    x: usize,
    y: usize,
    text: &str,
    gap: usize,
    mode: DrawMode,
    small: bool,
) {
    let char_w = if small {
        font::CHAR_WIDTH_SMALL
    } else {
        font::CHAR_WIDTH
    };
    let char_h = if small {
        font::CHAR_HEIGHT_SMALL
    } else {
        font::CHAR_HEIGHT
    };

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
        let glyph: &[u8] = if small {
            font::glyph_small(ch)
        } else {
            font::glyph(ch)
        };
        for (col, &col_data) in glyph.iter().enumerate() {
            let px = cx + col;
            if px >= 128 {
                break;
            }
            for bit in 0..char_h {
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
        cx += char_w + gap;
    }
}

/// 估算文本像素宽度（5×7）。
#[inline]
pub fn text_width(text: &str, gap: usize) -> usize {
    text.chars().count() * (font::CHAR_WIDTH + gap)
}

/// 小号字体文本宽度（4×6）。
#[inline]
pub fn text_width_small(text: &str, gap: usize) -> usize {
    text.chars().count() * (font::CHAR_WIDTH_SMALL + gap)
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
    fn draw_small_ok() {
        let mut fb = Framebuffer::new();
        draw_small(&mut fb, 0, 0, "ok");
    }

    #[test]
    fn draw_small_packed_ok() {
        let mut fb = Framebuffer::new();
        draw_small_packed(&mut fb, 0, 0, "CPU 48.5°C");
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
    fn width_comparison() {
        assert!(text_width("CPU", 0) < text_width("CPU", 1));
        assert!(text_width_small("CPU", 0) < text_width("CPU", 0));
    }

    #[test]
    fn oob_no_panic() {
        let mut fb = Framebuffer::new();
        draw_text(&mut fb, 200, 200, "X");
    }
}
