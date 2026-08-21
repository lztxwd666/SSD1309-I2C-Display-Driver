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
use std::fmt;

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
        write_pixel(&mut self.buffer, x, y, on);
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

    /// 将线性 1-bit 位图绘制到帧缓冲指定位置。
    ///
    /// `data` 为逐行打包的位图：每行 `w` 位按 8 位一组（MSB 优先）存入字节，
    /// 行末不足 8 位时高位补零；`data` 长度须 ≥ `w.div_ceil(8) * h`。
    /// 越界部分自动裁剪（只绘制屏幕内区域）。
    ///
    /// 脏矩形按实际写入区域一次性标记，不逐像素更新。
    pub fn blit(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        data: &[u8],
        mode: BlitMode,
    ) -> Result<(), BlitError> {
        if w == 0 || h == 0 {
            return Ok(());
        }
        // 裁剪到屏幕范围
        let x0 = x.min(WIDTH);
        let y0 = y.min(HEIGHT);
        let x1 = x.saturating_add(w).min(WIDTH);
        let y1 = y.saturating_add(h).min(HEIGHT);
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }
        // 只有在确实需要读取数据时才校验长度，避免全离屏空操作被误报
        let row_bytes = w.div_ceil(8);
        let required = row_bytes
            .checked_mul(h)
            .ok_or(BlitError::DimensionOverflow)?;
        if data.len() < required {
            return Err(BlitError::InsufficientData {
                required,
                actual: data.len(),
            });
        }
        // 裁剪后的源偏移（位图坐标系）
        let src_x = x0 - x;
        let src_y = y0 - y;
        for row in 0..(y1 - y0) {
            for col in 0..(x1 - x0) {
                let sx = src_x + col;
                let byte = data[(src_y + row) * row_bytes + sx / 8];
                let on = byte & (0x80 >> (sx % 8)) != 0;
                if on {
                    write_pixel(&mut self.buffer, x0 + col, y0 + row, true);
                } else if mode == BlitMode::Overwrite {
                    write_pixel(&mut self.buffer, x0 + col, y0 + row, false);
                }
            }
        }
        // 脏矩形：实际写入区域（两个对角点即可扩展覆盖）
        self.mark_dirty(x0, y0);
        self.mark_dirty(x1 - 1, y1 - 1);
        Ok(())
    }

    pub(crate) fn mark_all_dirty(&mut self) {
        self.dirty = Some((0, 0, WIDTH, HEIGHT));
    }
}

/// 写入单个像素位（调用方需保证坐标在屏幕范围内）。
///
/// 页布局：`buffer[page * WIDTH + col]`，bit0 为页内顶部像素。
#[inline]
fn write_pixel(buffer: &mut [u8; WIDTH * HEIGHT / 8], x: usize, y: usize, on: bool) {
    let page = y >> 3; // y / 8
    let bit = (y & 0x07) as u8; // y % 8
    let idx = page * WIDTH + x;
    if on {
        buffer[idx] |= 1 << bit;
    } else {
        buffer[idx] &= !(1 << bit);
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// 位图 blit 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlitMode {
    /// 覆盖：位图的 0/1 均写入目标（1 点亮、0 熄灭）。
    Overwrite,
    /// 点亮：仅置位为 1 的像素，0 不改变目标（适合在已有背景上绘制字形）。
    Set,
}

/// 位图 blit 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlitError {
    /// `data` 长度不足以表示完整位图。
    InsufficientData {
        /// 按 `w * h` 位图布局计算出的所需字节数。
        required: usize,
        /// 实际传入的 `data` 长度。
        actual: usize,
    },
    /// 位图尺寸在计算所需字节数时溢出 `usize`。
    DimensionOverflow,
}

impl fmt::Display for BlitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientData { required, actual } => {
                write!(
                    f,
                    "位图数据长度不足：需要 {required} 字节，实际 {actual} 字节"
                )
            }
            Self::DimensionOverflow => write!(f, "位图尺寸过大，计算所需字节数时溢出"),
        }
    }
}

impl std::error::Error for BlitError {}

/// 多页帧缓冲：N 个独立页面，支持循环翻页。
///
/// 每页是完整的 128×64 帧缓冲，页面内容在切换间保留；
/// 翻页只切换"当前页"索引。配合
/// [`Display::show_page`](crate::display::Display::show_page) 将当前页推帧到屏幕。
///
/// 典型用法（多页仪表）：绘制各页内容 → 定时/按键翻页 → 推帧显示。
pub struct PageBuffer<const N: usize> {
    pages: [Framebuffer; N],
    current: usize,
}

impl<const N: usize> PageBuffer<N> {
    /// 创建 N 页全黑帧缓冲，当前页为 0。
    pub fn new() -> Self {
        Self {
            pages: std::array::from_fn(|_| Framebuffer::new()),
            current: 0,
        }
    }

    /// 当前页的可变引用（用于绘制）。
    ///
    /// 注意：返回的是**当前页**。初始化多页内容时应使用
    /// [`page_at_mut`](Self::page_at_mut) 显式指定页，或先
    /// [`show`](Self::show) 切换再调用本方法——连续调用本方法
    /// 会反复绘制同一页，页面之间不会自动前进。
    ///
    /// 调用前提：`N >= 1`（`N == 0` 时无页可绘制，会 panic）。
    pub fn page(&mut self) -> &mut Framebuffer {
        &mut self.pages[self.current]
    }

    /// 指定页的只读引用；索引越界时返回 `None`。
    pub fn page_at(&self, index: usize) -> Option<&Framebuffer> {
        self.pages.get(index)
    }

    /// 指定页的可变引用（用于绘制）；索引越界时返回 `None`。
    ///
    /// 多页初始化绘制的推荐入口：无需切换当前页即可逐页填充内容。
    pub fn page_at_mut(&mut self, index: usize) -> Option<&mut Framebuffer> {
        self.pages.get_mut(index)
    }

    /// 当前页码（0 起始）。
    pub fn current(&self) -> usize {
        self.current
    }

    /// 页总数。
    pub fn len(&self) -> usize {
        N
    }

    /// 是否无页（`N == 0` 时恒为 `true`）。
    pub fn is_empty(&self) -> bool {
        N == 0
    }

    /// 切换到指定页；索引越界时不切换并返回 `false`。
    pub fn show(&mut self, index: usize) -> bool {
        if index < N {
            self.current = index;
            true
        } else {
            false
        }
    }

    /// 翻到下一页（循环：末页回到首页），返回新页码。
    ///
    /// `N == 0` 时恒返回 0（无页可翻，不 panic）。
    pub fn next_page(&mut self) -> usize {
        if N == 0 {
            return 0;
        }
        self.current = (self.current + 1) % N;
        self.current
    }

    /// 翻到上一页（循环：首页回到末页），返回新页码。
    ///
    /// `N == 0` 时恒返回 0（无页可翻，不 panic）。
    pub fn prev_page(&mut self) -> usize {
        if N == 0 {
            return 0;
        }
        self.current = (self.current + N - 1) % N;
        self.current
    }
}

impl<const N: usize> Default for PageBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

// embedded-graphics 集成

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
                write_pixel(&mut self.buffer, x as usize, y as usize, color.is_on());
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

    #[test]
    fn blit_draws_bitmap_and_tracks_dirty() {
        let mut fb = Framebuffer::new();
        // 2×2 位图：行1 = 11，行2 = 10
        let data = [0b1100_0000, 0b1000_0000];
        fb.blit(10, 10, 2, 2, &data, BlitMode::Overwrite).unwrap();

        assert!(fb.get_pixel(10, 10));
        assert!(fb.get_pixel(11, 10));
        assert!(fb.get_pixel(10, 11));
        assert!(
            !fb.get_pixel(11, 11),
            "位图 0 位在 Overwrite 模式应熄灭目标像素"
        );
        // 脏矩形 = 实际写入区域
        assert_eq!(fb.dirty_rect(), Some((10, 10, 2, 2)));
    }

    #[test]
    fn blit_set_mode_preserves_zero_bits() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(11, 11, true); // 预置像素在位图 0 位处
        let data = [0b1100_0000, 0b1000_0000];
        fb.blit(10, 10, 2, 2, &data, BlitMode::Set).unwrap();

        assert!(fb.get_pixel(10, 10));
        assert!(fb.get_pixel(11, 10));
        assert!(fb.get_pixel(10, 11));
        assert!(fb.get_pixel(11, 11), "Set 模式不应清除预置像素");
    }

    #[test]
    fn blit_clips_to_screen() {
        let mut fb = Framebuffer::new();
        // 8×1 位图从 x=124 开始：只画 124-127 四个像素
        let data = [0b1111_0000];
        fb.blit(124, 0, 8, 1, &data, BlitMode::Overwrite).unwrap();
        assert!(fb.get_pixel(124, 0));
        assert!(fb.get_pixel(127, 0));
        assert!(!fb.get_pixel(123, 0)); // 屏外不写
        // 完全在屏外 → 空操作
        let mut fb2 = Framebuffer::new();
        fb2.blit(200, 200, 8, 8, &data, BlitMode::Overwrite)
            .unwrap();
        assert_eq!(fb2.dirty_rect(), None);
        // 零尺寸 → 空操作
        fb2.blit(0, 0, 0, 8, &data, BlitMode::Overwrite).unwrap();
        assert_eq!(fb2.dirty_rect(), None);
    }

    #[test]
    fn blit_multi_byte_row() {
        let mut fb = Framebuffer::new();
        // 10 位宽 1 行（MSB 优先）：字节0 = 11110000 → 列 0-3 亮；字节1 = 11000000 → 列 8-9 亮
        let data = [0b1111_0000, 0b1100_0000];
        fb.blit(0, 0, 10, 1, &data, BlitMode::Overwrite).unwrap();
        for x in [0, 1, 2, 3, 8, 9] {
            assert!(fb.get_pixel(x, 0), "列 {} 应点亮", x);
        }
        for x in [4, 5, 6, 7] {
            assert!(!fb.get_pixel(x, 0), "列 {} 应为灭", x);
        }
        assert!(!fb.get_pixel(10, 0));
    }

    #[test]
    fn blit_rejects_insufficient_data() {
        let mut fb = Framebuffer::new();
        // 10 位宽 1 行需要 2 字节，只给 1 字节应返回错误且不产生部分绘制
        let data = [0b1111_0000];
        let err = fb
            .blit(0, 0, 10, 1, &data, BlitMode::Overwrite)
            .unwrap_err();
        assert!(matches!(
            err,
            BlitError::InsufficientData {
                required: 2,
                actual: 1
            }
        ));
        assert!(fb.buffer.iter().all(|&b| b == 0));
    }

    #[test]
    fn page_buffer_isolates_pages() {
        let mut pages = PageBuffer::<3>::new();
        assert_eq!(pages.len(), 3);
        assert_eq!(pages.current(), 0);

        // 当前页绘制不影响其他页
        pages.page().set_pixel(5, 5, true);
        assert!(pages.page_at(0).unwrap().get_pixel(5, 5));
        assert!(!pages.page_at(1).unwrap().get_pixel(5, 5));
        assert!(!pages.page_at(2).unwrap().get_pixel(5, 5));
    }

    /// 回归测试：多页初始化绘制（page_at_mut 逐页填充）时各页内容必须独立。
    /// 曾因误用 `page()`（始终返回当前页）导致所有内容画进同一页。
    #[test]
    fn page_buffer_draws_each_page_independently() {
        let mut pages = PageBuffer::<3>::new();
        // 逐页绘制不同像素
        pages.page_at_mut(0).unwrap().set_pixel(0, 0, true);
        pages.page_at_mut(1).unwrap().set_pixel(10, 10, true);
        pages.page_at_mut(2).unwrap().set_pixel(20, 20, true);

        // 每页只含自己的像素
        assert!(pages.page_at(0).unwrap().get_pixel(0, 0));
        assert!(!pages.page_at(0).unwrap().get_pixel(10, 10));
        assert!(pages.page_at(1).unwrap().get_pixel(10, 10));
        assert!(!pages.page_at(1).unwrap().get_pixel(20, 20));
        assert!(pages.page_at(2).unwrap().get_pixel(20, 20));
        assert!(!pages.page_at(2).unwrap().get_pixel(0, 0));

        // page_at_mut 越界返回 None
        assert!(pages.page_at_mut(3).is_none());
    }

    #[test]
    fn page_buffer_wraps_around() {
        let mut pages = PageBuffer::<3>::new();
        // 下一页循环：0 → 1 → 2 → 0
        assert_eq!(pages.next_page(), 1);
        assert_eq!(pages.next_page(), 2);
        assert_eq!(pages.next_page(), 0);
        // 上一页循环：0 → 2 → 1 → 0
        assert_eq!(pages.prev_page(), 2);
        assert_eq!(pages.prev_page(), 1);
        assert_eq!(pages.prev_page(), 0);
    }

    #[test]
    fn page_buffer_zero_pages_no_panic() {
        // 回归测试：N=0 时翻页方法不应除零 panic
        let mut pages = PageBuffer::<0>::new();
        assert!(pages.is_empty());
        assert_eq!(pages.len(), 0);
        assert_eq!(pages.next_page(), 0);
        assert_eq!(pages.prev_page(), 0);
        assert!(!pages.show(0));
        assert!(pages.page_at(0).is_none());
        assert!(pages.page_at_mut(0).is_none());
    }

    #[test]
    fn page_buffer_show_validates_index() {
        let mut pages = PageBuffer::<3>::new();
        assert!(pages.show(2));
        assert_eq!(pages.current(), 2);
        assert!(!pages.show(3)); // 越界不切换
        assert_eq!(pages.current(), 2);
        assert!(pages.page_at(3).is_none());
    }
}
