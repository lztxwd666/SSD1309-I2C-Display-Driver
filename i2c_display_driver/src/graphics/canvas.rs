//! 绘图原语 —— 在帧缓冲区上绘制几何图形与填充。
//!
//! 提供矩形、直线、圆、三角形及其填充，以及点线变体。
//! 所有坐标越界均静默裁剪，与 `Framebuffer::set_pixel` 保持一致。

use crate::display::Framebuffer;

/// 绘制矩形边框（空心）。
pub fn draw_rect(fb: &mut Framebuffer, x: usize, y: usize, w: usize, h: usize) {
    if w == 0 || h == 0 || x >= 128 || y >= 64 {
        return;
    }
    let x2 = x.saturating_add(w.saturating_sub(1)).min(127);
    let y2 = y.saturating_add(h.saturating_sub(1)).min(63);

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
    if w == 0 || h == 0 || x >= 128 || y >= 64 {
        return;
    }
    let x2 = (x + w).min(128);
    let y2 = (y + h).min(64);
    for py in y..y2 {
        for px in x..x2 {
            fb.set_pixel(px, py, true);
        }
    }
}

/// 绘制水平实线。
pub fn draw_hline(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    for i in 0..len {
        fb.set_pixel(x.saturating_add(i), y, true);
    }
}

/// 绘制垂直实线。
pub fn draw_vline(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    for i in 0..len {
        fb.set_pixel(x, y.saturating_add(i), true);
    }
}

/// 绘制任意直线（Bresenham 算法）。
pub fn draw_line(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32) {
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

/// 绘制圆（空心，中点圆算法）。
pub fn draw_circle(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32) {
    if r < 0 {
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

/// 绘制实心圆（填充，按行填充中点圆算法生成的边界）。
pub fn fill_circle(fb: &mut Framebuffer, cx: i32, cy: i32, r: i32) {
    if r < 0 {
        return;
    }
    let mut x = r;
    let mut y = 0;
    let mut err = 1 - r;
    while x >= y {
        fill_row(fb, cx - x, cx + x, cy + y);
        fill_row(fb, cx - x, cx + x, cy - y);
        fill_row(fb, cx - y, cx + y, cy + x);
        fill_row(fb, cx - y, cx + y, cy - x);
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
pub fn fill_triangle(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32) {
    // 按 y 升序排序三个顶点
    let mut v = [(x0, y0), (x1, y1), (x2, y2)];
    v.sort_by_key(|&(_, y)| y);
    let (ax, ay) = v[0];
    let (bx, by) = v[1];
    let (cx, cy) = v[2];

    // 三点共线（退化为水平线）直接返回
    if cy == ay {
        return;
    }

    for y in ay..=cy {
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
    for i in (0..len).step_by(2) {
        fb.set_pixel(x.saturating_add(i), y, true);
    }
}

/// 绘制垂直点线（每隔一个像素）。
pub fn draw_vline_dotted(fb: &mut Framebuffer, x: usize, y: usize, len: usize) {
    for i in (0..len).step_by(2) {
        fb.set_pixel(x, y.saturating_add(i), true);
    }
}

// 内部辅助

/// 若坐标在屏幕内则点亮该像素。
#[inline]
fn plot(fb: &mut Framebuffer, x: i32, y: i32) {
    if (0..128).contains(&x) && (0..64).contains(&y) {
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

/// 在 y 行填充 [x0, x1] 区间（越界裁剪）。
#[inline]
fn fill_row(fb: &mut Framebuffer, x0: i32, x1: i32, y: i32) {
    if y < 0 || y >= 64 {
        return;
    }
    let x0 = x0.max(0) as usize;
    let x1 = x1.min(127) as usize;
    for x in x0..=x1 {
        fb.set_pixel(x, y as usize, true);
    }
}

/// 计算扫描线 y 处在 (x_a, y_a)-(x_b, y_b) 边上的 x 坐标（整数插值）。
#[inline]
fn interpolate_x(x_a: i32, y_a: i32, x_b: i32, y_b: i32, y: i32) -> i32 {
    let dy = y_b - y_a;
    if dy == 0 {
        return x_a;
    }
    x_a + (x_b - x_a) * (y - y_a) / dy
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
}
