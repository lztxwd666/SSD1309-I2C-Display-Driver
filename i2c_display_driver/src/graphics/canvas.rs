//! 绘图原语 —— 在帧缓冲区上绘制几何图形与填充。
//!
//! 提供矩形、直线、圆、三角形及其填充，以及点线变体。
//! 所有坐标越界均静默裁剪，与 `Framebuffer::set_pixel` 保持一致。
//! 对超大/离屏输入会先做屏幕空间裁剪，避免无效的长时间循环或整数溢出。

use crate::display::{Framebuffer, HEIGHT, WIDTH};

/// 绘制矩形边框（空心）。
pub fn draw_rect(fb: &mut Framebuffer, x: usize, y: usize, w: usize, h: usize) {
    if w == 0 || h == 0 || x >= WIDTH || y >= HEIGHT {
        return;
    }
    let x2 = x.saturating_add(w.saturating_sub(1)).min(WIDTH - 1);
    let y2 = y.saturating_add(h.saturating_sub(1)).min(HEIGHT - 1);

    for px in x..=x2 {
        fb.set_pixel(px, y, true);
        fb.set_pixel(px, y2, true);
    }
    for py in y..=y2 {
        fb.set_pixel(x, py, true);
        fb.set_pixel(x2, py, true);
    }
}

/// 绘制实心矩形（填充）。
pub fn fill_rect(fb: &mut Framebuffer, x: usize, y: usize, w: usize, h: usize) {
    if w == 0 || h == 0 || x >= WIDTH || y >= HEIGHT {
        return;
    }
    let x2 = x.saturating_add(w).min(WIDTH);
    let y2 = y.saturating_add(h).min(HEIGHT);
    for py in y..y2 {
        for px in x..x2 {
            fb.set_pixel(px, py, true);
        }
    }
}

/// 绘制水平实线。
pub fn draw_hline(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    if y >= HEIGHT || len == 0 || x >= WIDTH {
        return;
    }
    let end = x.saturating_add(len).min(WIDTH);
    for px in x..end {
        fb.set_pixel(px, y, true);
    }
}

/// 绘制垂直实线。
pub fn draw_vline(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    if x >= WIDTH || len == 0 || y >= HEIGHT {
        return;
    }
    let end = y.saturating_add(len).min(HEIGHT);
    for py in y..end {
        fb.set_pixel(x, py, true);
    }
}

/// 绘制任意直线（Bresenham 算法）。
///
/// 先使用 Cohen-Sutherland 将线段裁剪到屏幕矩形，再执行 Bresenham，
/// 避免超大坐标导致整数溢出或无效长循环。
pub fn draw_line(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32) {
    if let Some(((cx0, cy0), (cx1, cy1))) = clip_line(x0, y0, x1, y1) {
        draw_line_raw(fb, cx0, cy0, cx1, cy1);
    }
}

/// 普通半径使用中点圆算法时的上限。
///
/// 超过该值说明圆远大于屏幕，继续用中点算法会带来无意义的巨大循环；
/// 此时改用屏幕空间扫描线，保证超大半径也不会拖垮调用方。
const LARGE_RADIUS_FALLBACK: i32 = 4096;

/// 绘制圆（空心）。
///
/// 常规半径沿用中点圆算法，保持既有像素行为；超大半径或完全离屏时
/// 走屏幕空间裁剪路径，避免无效长循环或整数溢出。
pub fn draw_circle(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32) {
    if r < 0 || !circle_intersects_screen(cx, cy, r) {
        return;
    }
    if r > LARGE_RADIUS_FALLBACK {
        draw_circle_large(fb, cx, cy, r);
        return;
    }
    let mut x = r;
    let mut y = 0;
    let mut err = 1 - r;
    while x >= y {
        plot8(fb, cx, cy, x, y);
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

/// 绘制实心圆（填充）。
///
/// 常规半径沿用中点圆算法生成的扫描区间，保持既有像素行为；超大半径
/// 或完全离屏时走屏幕空间裁剪路径。
pub fn fill_circle(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32) {
    if r < 0 || !circle_intersects_screen(cx, cy, r) {
        return;
    }
    if r > LARGE_RADIUS_FALLBACK {
        fill_circle_large(fb, cx, cy, r);
        return;
    }
    let mut x = r;
    let mut y = 0;
    let mut err = 1 - r;
    while x >= y {
        fill_row(fb, (cx - x) as i64, (cx + x) as i64, (cy + y) as i64);
        fill_row(fb, (cx - x) as i64, (cx + x) as i64, (cy - y) as i64);
        fill_row(fb, (cx - y) as i64, (cx + y) as i64, (cy + x) as i64);
        fill_row(fb, (cx - y) as i64, (cx + y) as i64, (cy - x) as i64);
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

/// 绘制三角形边框（空心）。
pub fn draw_triangle(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32) {
    draw_line(fb, x0, y0, x1, y1);
    draw_line(fb, x1, y1, x2, y2);
    draw_line(fb, x2, y2, x0, y0);
}

/// 绘制实心三角形（填充，扫描线算法）。
///
/// 先将 y 扫描范围裁剪到屏幕，再用 i64 做插值，避免离屏大坐标导致
/// 无效长循环或整数溢出。
pub fn fill_triangle(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32) {
    // 按 y 升序排序三个顶点
    let mut v = [
        (x0 as i64, y0 as i64),
        (x1 as i64, y1 as i64),
        (x2 as i64, y2 as i64),
    ];
    v.sort_by_key(|&(_, y)| y);
    let (ax, ay) = v[0];
    let (bx, by) = v[1];
    let (cx, cy) = v[2];

    // 三点共线（退化为水平线）直接返回
    if cy == ay {
        return;
    }

    // 只扫描屏幕可见的 y 范围
    let y_start = ay.max(0);
    let y_end = cy.min((HEIGHT - 1) as i64);
    if y_start > y_end {
        return;
    }

    for y in y_start..=y_end {
        // 左边界：顶点 A → C 的插值
        let left = interpolate_x(ax, ay, cx, cy, y);
        // 右边界：上半段 A → B，下半段 B → C
        let right = if y < by {
            interpolate_x(ax, ay, bx, by, y)
        } else {
            interpolate_x(bx, by, cx, cy, y)
        };
        let (l, r) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        fill_row(fb, l, r, y);
    }
}

/// 绘制水平点线（每隔一个像素）。
pub fn draw_hline_dotted(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    if y >= HEIGHT || len == 0 || x >= WIDTH {
        return;
    }
    let end = x.saturating_add(len).min(WIDTH);
    for px in (x..end).step_by(2) {
        fb.set_pixel(px, y, true);
    }
}

/// 绘制垂直点线（每隔一个像素）。
pub fn draw_vline_dotted(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    if x >= WIDTH || len == 0 || y >= HEIGHT {
        return;
    }
    let end = y.saturating_add(len).min(HEIGHT);
    for py in (y..end).step_by(2) {
        fb.set_pixel(x, py, true);
    }
}

// 内部辅助

/// 若坐标在屏幕内则点亮该像素。
#[inline]
fn plot(fb: &mut Framebuffer, x: i32, y: i32) {
    if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
        fb.set_pixel(x as usize, y as usize, true);
    }
}

/// i64 版本的点亮辅助，用于大坐标计算后的安全裁剪。
#[inline]
fn plot_i64(fb: &mut Framebuffer, x: i64, y: i64) {
    if (0..WIDTH as i64).contains(&x) && (0..HEIGHT as i64).contains(&y) {
        fb.set_pixel(x as usize, y as usize, true);
    }
}

/// 绘制圆的八分对称点。
#[inline]
fn plot8(fb: &mut Framebuffer, cx: i32, cy: i32, x: i32, y: i32) {
    for &(px, py) in &[
        (cx + x, cy + y),
        (cx - x, cy + y),
        (cx + x, cy - y),
        (cx - x, cy - y),
        (cx + y, cy + x),
        (cx - y, cy + x),
        (cx + y, cy - x),
        (cx - y, cy - x),
    ] {
        plot(fb, px, py);
    }
}

/// 判断圆与屏幕矩形是否相交。
fn circle_intersects_screen(cx: i32, cy: i32, r: i32) -> bool {
    let cx = cx as i64;
    let cy = cy as i64;
    let r = r as i64;
    let left = cx - r;
    let right = cx + r;
    let top = cy - r;
    let bottom = cy + r;
    left < WIDTH as i64 && right >= 0 && top < HEIGHT as i64 && bottom >= 0
}

/// 超大半径空心圆的屏幕空间绘制：按行计算边界像素。
fn draw_circle_large(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32) {
    let r = r as i64;
    let cx = cx as i64;
    let cy = cy as i64;
    let r2 = r * r;
    for y in 0..HEIGHT as i64 {
        let dy = y - cy;
        let dy2 = dy * dy;
        if dy2 > r2 {
            continue;
        }
        let rem = r2 - dy2;
        let half = isqrt(rem as u64) as i64;
        let x_left = cx - half;
        let x_right = cx + half;
        plot_i64(fb, x_left, y);
        if x_right != x_left {
            plot_i64(fb, x_right, y);
        }
    }
}

/// 超大半径实心圆的屏幕空间绘制：按行填充圆内区间。
fn fill_circle_large(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32) {
    let r = r as i64;
    let cx = cx as i64;
    let cy = cy as i64;
    let r2 = r * r;
    for y in 0..HEIGHT as i64 {
        let dy = y - cy;
        let dy2 = dy * dy;
        if dy2 > r2 {
            continue;
        }
        let rem = r2 - dy2;
        let half = isqrt(rem as u64) as i64;
        fill_row(fb, cx - half, cx + half, y);
    }
}

/// 在 y 行填充 [x0, x1] 区间（越界裁剪）。
#[inline]
fn fill_row(fb: &mut Framebuffer, x0: i64, x1: i64, y: i64) {
    if !(0..HEIGHT as i64).contains(&y) {
        return;
    }
    if x1 < 0 || x0 >= WIDTH as i64 {
        return;
    }
    let x0 = x0.max(0) as usize;
    let x1 = x1.min((WIDTH - 1) as i64) as usize;
    for x in x0..=x1 {
        fb.set_pixel(x, y as usize, true);
    }
}

/// 计算扫描线 y 处在 (x_a, y_a)-(x_b, y_b) 边上的 x 坐标（整数插值）。
#[inline]
fn interpolate_x(x_a: i64, y_a: i64, x_b: i64, y_b: i64, y: i64) -> i64 {
    let dy = y_b - y_a;
    if dy == 0 {
        return x_a;
    }
    x_a + (x_b - x_a) * (y - y_a) / dy
}

/// 无符号整数平方根（牛顿迭代），用于圆的光栅化。
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Cohen-Sutherland 线段裁剪到屏幕矩形。
///
/// 返回裁剪后的端点；线段完全在屏幕外时返回 `None`。
fn clip_line(x0: i32, y0: i32, x1: i32, y1: i32) -> Option<((i32, i32), (i32, i32))> {
    const INSIDE: u8 = 0;
    const LEFT: u8 = 1;
    const RIGHT: u8 = 2;
    const TOP: u8 = 4;
    const BOTTOM: u8 = 8;

    fn out_code(x: i64, y: i64) -> u8 {
        let mut code = INSIDE;
        if x < 0 {
            code |= LEFT;
        } else if x >= WIDTH as i64 {
            code |= RIGHT;
        }
        if y < 0 {
            code |= TOP;
        } else if y >= HEIGHT as i64 {
            code |= BOTTOM;
        }
        code
    }

    let x_min = 0i64;
    let y_min = 0i64;
    let x_max = (WIDTH - 1) as i64;
    let y_max = (HEIGHT - 1) as i64;
    let mut x0 = x0 as i64;
    let mut y0 = y0 as i64;
    let mut x1 = x1 as i64;
    let mut y1 = y1 as i64;

    loop {
        let c0 = out_code(x0, y0);
        let c1 = out_code(x1, y1);

        // 两端点都在同一外部区域，线段完全不可见
        if c0 & c1 != 0 {
            return None;
        }
        // 两端点都在屏幕内
        if c0 == 0 && c1 == 0 {
            return Some(((x0 as i32, y0 as i32), (x1 as i32, y1 as i32)));
        }

        // 选择需要裁剪的端点
        let out = if c0 != 0 { c0 } else { c1 };
        let (x, y) = if out & TOP != 0 {
            (x0 + (x1 - x0) * (y_min - y0) / (y1 - y0), y_min)
        } else if out & BOTTOM != 0 {
            (x0 + (x1 - x0) * (y_max - y0) / (y1 - y0), y_max)
        } else if out & RIGHT != 0 {
            (x_max, y0 + (y1 - y0) * (x_max - x0) / (x1 - x0))
        } else if out & LEFT != 0 {
            (x_min, y0 + (y1 - y0) * (x_min - x0) / (x1 - x0))
        } else {
            unreachable!("out_code 必须包含至少一个外部区域位");
        };

        if out == c0 {
            x0 = x;
            y0 = y;
        } else {
            x1 = x;
            y1 = y;
        }
    }
}

/// 裁剪后的 Bresenham 直线绘制；调用方保证坐标已位于屏幕内。
fn draw_line_raw(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        plot(fb, x, y);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_does_not_panic_at_bounds() {
        let mut fb = Framebuffer::new();
        draw_rect(&mut fb, 0, 0, 128, 64);
        draw_rect(&mut fb, 200, 200, 50, 50);
    }

    #[test]
    fn fill_rect_works() {
        let mut fb = Framebuffer::new();
        fill_rect(&mut fb, 0, 0, 10, 10);
        assert!(fb.get_pixel(5, 5));
        assert!(!fb.get_pixel(15, 5));
    }

    #[test]
    fn line_draws_endpoints() {
        let mut fb = Framebuffer::new();
        draw_line(&mut fb, 0, 0, 10, 10);
        assert!(fb.get_pixel(0, 0));
        assert!(fb.get_pixel(10, 10));
    }

    #[test]
    fn line_clips_extreme_coordinates() {
        let mut fb = Framebuffer::new();
        // 远距离水平线只应绘制屏幕内部分，且不应溢出或长时间循环
        draw_line(&mut fb, -1_000_000_000, 10, 1_000_000_000, 10);
        assert!(fb.get_pixel(0, 10));
        assert!(fb.get_pixel(127, 10));
    }

    #[test]
    fn circle_draws_cardinals() {
        let mut fb = Framebuffer::new();
        draw_circle(&mut fb, 32, 32, 10);
        assert!(fb.get_pixel(42, 32));
        assert!(fb.get_pixel(32, 42));
    }

    #[test]
    fn fill_circle_center_filled() {
        let mut fb = Framebuffer::new();
        fill_circle(&mut fb, 32, 32, 10);
        assert!(fb.get_pixel(32, 32));
        assert!(!fb.get_pixel(32, 10)); // r=10 圆外
    }

    #[test]
    fn fill_circle_offscreen_left_no_hang() {
        let mut fb = Framebuffer::new();
        // 回归测试：完全位于屏幕左侧的圆不得进入巨大循环
        fill_circle(&mut fb, -100, 10, 5);
        assert!(fb.buffer.iter().all(|&b| b == 0));
    }

    #[test]
    fn triangle_draws_edges() {
        let mut fb = Framebuffer::new();
        draw_triangle(&mut fb, 10, 10, 60, 10, 35, 50);
        assert!(fb.get_pixel(10, 10));
        assert!(fb.get_pixel(35, 50));
    }

    #[test]
    fn fill_triangle_fills_center() {
        let mut fb = Framebuffer::new();
        fill_triangle(&mut fb, 10, 10, 60, 10, 35, 50);
        assert!(fb.get_pixel(35, 30));
    }

    #[test]
    fn fill_triangle_offscreen_left_no_hang() {
        let mut fb = Framebuffer::new();
        // 回归测试：完全位于屏幕左侧的三角形不得进入巨大循环
        fill_triangle(&mut fb, -100, 0, -90, 10, -110, 20);
        assert!(fb.buffer.iter().all(|&b| b == 0));
    }

    #[test]
    fn dotted_line_spacing() {
        let mut fb = Framebuffer::new();
        draw_hline_dotted(&mut fb, 0, 0, 10);
        assert!(fb.get_pixel(0, 0));
        assert!(!fb.get_pixel(1, 0));
        assert!(fb.get_pixel(2, 0));
    }

    #[test]
    fn empty_rect_noop() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(10, 10, true);
        draw_rect(&mut fb, 10, 10, 0, 0);
        draw_rect(&mut fb, 10, 10, 1, 0);
        assert!(fb.get_pixel(10, 10));
    }

    #[test]
    fn fill_rect_extreme_size_no_panic() {
        // 回归测试：极端 w/h 不应导致加法溢出（saturating 裁剪）
        let mut fb = Framebuffer::new();
        fill_rect(&mut fb, 100, 50, usize::MAX, usize::MAX);
        fill_rect(&mut fb, 0, 0, usize::MAX, usize::MAX);
        // 边界填充应被裁剪到屏幕内
        assert!(fb.get_pixel(127, 63));
    }

    #[test]
    fn hline_huge_len_no_hang() {
        let mut fb = Framebuffer::new();
        draw_hline(&mut fb, 0, 0, usize::MAX);
        assert!(fb.get_pixel(127, 0));
        // 不应绘制到屏幕外
        assert!(!fb.get_pixel(128, 0));
    }
}
