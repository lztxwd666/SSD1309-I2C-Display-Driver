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

fn draw_impl(fb: &mut Framebuffer, x: usize, y: usize, text: &str, gap: usize, mode: DrawMode) {
    let mut cx = x;
    let mut cy = y;
    for ch in text.chars() {
        if ch == '\n' {
            // 换行：回到行首，下一行下移一行（字符高度 + 1px 行距）
            cx = x;
            cy += font::CHAR_HEIGHT + 1;
            continue;
        }
        // 整行已超出屏幕下方或右侧 → 提前退出
        if cy >= 64 || cx >= 128 {
            break;
        }
        let glyph = font::glyph(ch);
        for (col, &col_data) in glyph.iter().enumerate() {
            let px = cx + col;
            if px >= 128 {
                break;
            }
            for bit in 0..font::CHAR_HEIGHT {
                let py = cy + bit;
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

/// 估算文本像素宽度（5×7）：多行取最长一行。
///
/// 与 `draw_impl` 的实际绘制一致：n 个字符占 `n×5 + (n-1)×gap` 像素
/// （末字符之后没有间距）。
#[inline]
pub fn text_width(text: &str, gap: usize) -> usize {
    text.lines()
        .map(|line| {
            let n = line.chars().count();
            if n == 0 {
                0
            } else {
                n * font::CHAR_WIDTH + (n - 1) * gap
            }
        })
        .max()
        .unwrap_or(0)
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

    #[test]
    fn newline_advances_row() {
        let mut fb = Framebuffer::new();
        draw_text(&mut fb, 0, 0, "A\nB");
        // 'B' 在第二行：y = 0 + 7 + 1 = 8。'B' 字形第一列 0x7F 的 bit0 点亮 (0,8)。
        assert!(fb.get_pixel(0, 8));
        // 无换行时同一位置应无像素
        let mut fb2 = Framebuffer::new();
        draw_text(&mut fb2, 0, 0, "AB");
        assert!(!fb2.get_pixel(0, 8));
    }

    #[test]
    fn text_width_takes_wide_line() {
        assert_eq!(text_width("AB\nCDE", 0), 5 * 3); // 最长行 "CDE"
        assert_eq!(text_width("", 1), 0);
    }

    #[test]
    fn text_width_matches_draw_impl_formula() {
        // 公式验证：n×5 + (n-1)×gap（末字符后无间距）
        assert_eq!(text_width("AB", 1), 11);
        assert_eq!(text_width("ABC", 2), 19);
        assert_eq!(text_width("A", 10), 5); // 单字符无间距
        assert_eq!(text_width("AB\nCD", 3), 13); // 多行取最长（两行等宽）
    }
}
