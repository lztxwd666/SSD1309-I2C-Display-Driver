//! 1-bit 帧缓冲区 —— 1024 字节，与 SSD1309 GDDRAM 布局一致。
//!
//! 布局：8 页 × 128 列。
//! 每个字节代表一列中的 8 个垂直像素，bit 0 为顶部像素。
//!
//! 实现 embedded-graphics 的 DrawTarget trait，可直接接收
//! Circle / Rectangle / Text / Image 等原语的像素迭代器。
//!
//! 帧缓冲区自动跟踪脏矩形：所有写入（set_pixel / draw_iter / clear / fill_all）
//! 都会记录被修改的区域，配合 [`Display::render_dirty`](crate::display::Display::render_dirty)
//! 可只推送变化区域，节省 I2C 带宽。

use core::convert::Infallible;

use embedded_graphics::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::BinaryColor,
};

/// 显示宽度（像素）。
pub const WIDTH: usize = 128;
/// 显示高度（像素）。
pub const HEIGHT: usize = 64;

/// SSD1309 128×64 单色帧缓冲区。
///
/// 内部布局与 GDDRAM 完全相同：
/// * `buffer[page * WIDTH + col]` 寻址列 `col`、页 `page` 的 8 个垂直像素
/// * bit 0 对应页内顶部像素（row 0），bit 7 对应页内底部像素（row 7）
///
/// 尺寸：1024 字节，栈分配，无堆开销。
pub struct Framebuffer {
    pub buffer: [u8; WIDTH * HEIGHT / 8],
    /// 脏矩形：(x0, y0, x1, y1) 半开区间，记录自上次推帧以来被修改的区域。
    dirty: Option<(usize, usize, usize, usize)>,
}

impl Framebuffer {
    /// 创建全零（全黑）帧缓冲区。
    pub fn new() -> Self {
        Self {
            buffer: [0u8; WIDTH * HEIGHT / 8],
            dirty: None,
        }
    }

    /// 清空整个缓冲区（全黑）。
    pub fn clear(&mut self) {
        self.buffer.fill(0);
        self.mark_all_dirty();
    }

    /// 填充整个缓冲区（全白 / 全亮）。
    pub fn fill_all(&mut self) {
        self.buffer.fill(0xFF);
        self.mark_all_dirty();
    }

    /// 设置单个像素。
    ///
    /// `x`: 列坐标 (0..128)，`y`: 行坐标 (0..64)。
    /// `on`: `true` 点亮，`false` 熄灭。
    ///
    /// 超出边界的坐标会被静默忽略。
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, on: bool) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let page = y >> 3; // y / 8
        let bit = (y & 0x07) as u8; // y % 8
        let idx = (page << 7) + x; // page * WIDTH + x
        if on {
            self.buffer[idx] |= 1 << bit;
        } else {
            self.buffer[idx] &= !(1 << bit);
        }
        self.mark_dirty(x, y);
    }

    /// 读取单个像素。
    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> bool {
        if x >= WIDTH || y >= HEIGHT {
            return false;
        }
        let page = y >> 3;
        let bit = (y & 0x07) as u8;
        let idx = (page << 7) + x;
        (self.buffer[idx] & (1 << bit)) != 0
    }

    /// 获取指向内部缓冲区的不可变引用。
    #[inline]
    pub fn as_bytes(&self) -> &[u8; WIDTH * HEIGHT / 8] {
        &self.buffer
    }

    /// 返回自上次推帧以来被修改的区域（x, y, w, h）。
    ///
    /// 无修改时返回 `None`。注意：直接修改 `buffer` 字段不会更新脏矩形，
    /// 请通过 `set_pixel` / embedded-graphics 绘制接口修改。
    pub fn dirty_rect(&self) -> Option<(usize, usize, usize, usize)> {
        self.dirty
            .map(|(x0, y0, x1, y1)| (x0, y0, x1 - x0, y1 - y0))
    }

    /// 清除脏矩形记录。
    pub fn clear_dirty(&mut self) {
        self.dirty = None;
    }

    fn mark_dirty(&mut self, x: usize, y: usize) {
        let d = self.dirty.get_or_insert((x, y, x + 1, y + 1));
        d.0 = d.0.min(x);
        d.1 = d.1.min(y);
        d.2 = d.2.max(x + 1);
        d.3 = d.3.max(y + 1);
    }

    fn mark_all_dirty(&mut self) {
        self.dirty = Some((0, 0, WIDTH, HEIGHT));
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ── embedded-graphics 集成 ────────────────────────────────

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Framebuffer {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            let x = coord.x;
            let y = coord.y;
            // 忽略越界像素（embedded-graphics 可能产生负坐标或超出范围的坐标）
            if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
                let page = y as usize >> 3;
                let bit = (y & 0x07) as u8;
                let idx = (page << 7) + x as usize;
                if color.is_on() {
                    self.buffer[idx] |= 1 << bit;
                } else {
                    self.buffer[idx] &= !(1 << bit);
                }
                self.mark_dirty(x as usize, y as usize);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_graphics::{geometry::Point, prelude::*, primitives::Rectangle};

    #[test]
    fn new_buffer_is_black() {
        let fb = Framebuffer::new();
        assert_eq!(fb.buffer, [0u8; WIDTH * HEIGHT / 8]);
    }

    #[test]
    fn set_and_get_pixel() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(0, 0, true);
        assert!(fb.get_pixel(0, 0));
        assert!(!fb.get_pixel(1, 0));
    }

    #[test]
    fn out_of_bounds_ignored() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(200, 200, true); // 不应 panic
        assert!(fb.buffer.iter().all(|&b| b == 0));
    }

    #[test]
    fn clear_resets_all() {
        let mut fb = Framebuffer::new();
        fb.fill_all();
        fb.clear();
        assert_eq!(fb.buffer, [0u8; WIDTH * HEIGHT / 8]);
    }

    #[test]
    fn draw_target_writes_pixels() {
        let mut fb = Framebuffer::new();
        // 用 e-g 原语直接绘制 10×10 填充矩形
        Rectangle::new(Point::new(0, 0), Size::new(10, 10))
            .into_styled(embedded_graphics::primitives::PrimitiveStyle::with_fill(
                BinaryColor::On,
            ))
            .draw(&mut fb)
            .unwrap();
        assert!(fb.get_pixel(0, 0));
        assert!(fb.get_pixel(9, 9));
        assert!(!fb.get_pixel(10, 0));
    }

    #[test]
    fn draw_target_clips_negative_coords() {
        let mut fb = Framebuffer::new();
        // e-g 可能产生负坐标，draw_iter 应静默忽略
        Rectangle::new(Point::new(-5, -5), Size::new(10, 10))
            .into_styled(embedded_graphics::primitives::PrimitiveStyle::with_fill(
                BinaryColor::On,
            ))
            .draw(&mut fb)
            .unwrap();
        // (0,0) 到 (4,4) 被绘制的部分应有像素
        assert!(fb.get_pixel(0, 0));
        assert!(fb.get_pixel(4, 4));
    }

    #[test]
    fn dirty_tracks_writes() {
        let mut fb = Framebuffer::new();
        assert_eq!(fb.dirty_rect(), None);
        fb.set_pixel(5, 5, true);
        assert_eq!(fb.dirty_rect(), Some((5, 5, 1, 1)));
        fb.set_pixel(20, 30, true);
        assert_eq!(fb.dirty_rect(), Some((5, 5, 16, 26)));
        fb.clear_dirty();
        assert_eq!(fb.dirty_rect(), None);
    }

    #[test]
    fn dirty_full_on_clear() {
        let mut fb = Framebuffer::new();
        fb.clear();
        assert_eq!(fb.dirty_rect(), Some((0, 0, WIDTH, HEIGHT)));
    }
}
