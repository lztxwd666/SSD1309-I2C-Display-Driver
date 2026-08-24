"""软件绘制层：文字与基础图形"""

from .font import FONT
from .framebuffer import Framebuffer, HEIGHT, WIDTH

CHAR_WIDTH = 5
CHAR_HEIGHT = 7


def _glyph(ch: str):
    if ch in FONT:
        return FONT[ch]
    return FONT[" "]


def draw_text(
    fb: Framebuffer,
    x: int,
    y: int,
    text: str,
    gap: int = 1,
    inverted: bool = False,
) -> None:
    """绘制 5x7 文本,inverted=True 时清除字形像素，用于反色背景"""
    if y >= HEIGHT:
        return
    cx = x
    cy = y
    for ch in text:
        if ch == "\n":
            cx = x
            cy += CHAR_HEIGHT + 1
            continue
        if cy >= HEIGHT or cx >= WIDTH:
            break
        cols = _glyph(ch)
        for col, data in enumerate(cols):
            px = cx + col
            if px >= WIDTH:
                break
            for bit in range(CHAR_HEIGHT):
                py = cy + bit
                if py >= HEIGHT:
                    break
                if data & (1 << bit):
                    fb.set_pixel(px, py, not inverted)
        cx += CHAR_WIDTH + gap


def draw_text_packed(fb: Framebuffer, x: int, y: int, text: str) -> None:
    """紧凑文本，字符间距为 0"""
    draw_text(fb, x, y, text, gap=0)


def draw_text_inverted(fb: Framebuffer, x: int, y: int, text: str) -> None:
    """反色文本，需先填充白色背景"""
    draw_text(fb, x, y, text, gap=1, inverted=True)


def text_width(text: str, gap: int = 1) -> int:
    """估算文本像素宽度，多行取最长行"""
    best = 0
    for line in text.split("\n"):
        n = len(line)
        if n == 0:
            width = 0
        else:
            width = n * CHAR_WIDTH + (n - 1) * gap
        best = max(best, width)
    return best


def draw_rect(fb: Framebuffer, x: int, y: int, w: int, h: int) -> None:
    if w == 0 or h == 0 or x >= WIDTH or y >= HEIGHT:
        return
    x2 = min(x + max(w - 1, 0), WIDTH - 1)
    y2 = min(y + max(h - 1, 0), HEIGHT - 1)
    for px in range(x, x2 + 1):
        fb.set_pixel(px, y, True)
        fb.set_pixel(px, y2, True)
    for py in range(y, y2 + 1):
        fb.set_pixel(x, py, True)
        fb.set_pixel(x2, py, True)


def fill_rect(fb: Framebuffer, x: int, y: int, w: int, h: int) -> None:
    if w == 0 or h == 0 or x >= WIDTH or y >= HEIGHT:
        return
    x2 = min(x + w, WIDTH)
    y2 = min(y + h, HEIGHT)
    for py in range(y, y2):
        for px in range(x, x2):
            fb.set_pixel(px, py, True)


def draw_hline(fb: Framebuffer, x: int, y: int, length: int) -> None:
    if y >= HEIGHT or length == 0 or x >= WIDTH:
        return
    end = min(x + length, WIDTH)
    for px in range(x, end):
        fb.set_pixel(px, y, True)


def draw_vline(fb: Framebuffer, x: int, y: int, length: int) -> None:
    if x >= WIDTH or length == 0 or y >= HEIGHT:
        return
    end = min(y + length, HEIGHT)
    for py in range(y, end):
        fb.set_pixel(x, py, True)


def _plot(fb: Framebuffer, x: int, y: int) -> None:
    if 0 <= x < WIDTH and 0 <= y < HEIGHT:
        fb.set_pixel(x, y, True)


def _out_code(x: int, y: int) -> int:
    code = 0
    if x < 0:
        code |= 1
    elif x >= WIDTH:
        code |= 2
    if y < 0:
        code |= 4
    elif y >= HEIGHT:
        code |= 8
    return code


def _clip_line(x0: int, y0: int, x1: int, y1: int):
    x_min, y_min = 0, 0
    x_max, y_max = WIDTH - 1, HEIGHT - 1
    while True:
        c0 = _out_code(x0, y0)
        c1 = _out_code(x1, y1)
        if c0 & c1:
            return None
        if c0 == 0 and c1 == 0:
            return (x0, y0, x1, y1)
        out = c0 if c0 else c1
        if out & 4:
            x = x0 + (x1 - x0) * (y_min - y0) // (y1 - y0)
            y = y_min
        elif out & 8:
            x = x0 + (x1 - x0) * (y_max - y0) // (y1 - y0)
            y = y_max
        elif out & 2:
            y = y0 + (y1 - y0) * (x_max - x0) // (x1 - x0)
            x = x_max
        elif out & 1:
            y = y0 + (y1 - y0) * (x_min - x0) // (x1 - x0)
            x = x_min
        else:
            return None
        if out == c0:
            x0, y0 = x, y
        else:
            x1, y1 = x, y


def draw_line(fb: Framebuffer, x0: int, y0: int, x1: int, y1: int) -> None:
    clipped = _clip_line(x0, y0, x1, y1)
    if clipped is None:
        return
    x0, y0, x1, y1 = clipped
    dx = abs(x1 - x0)
    sx = 1 if x0 < x1 else -1
    dy = -abs(y1 - y0)
    sy = 1 if y0 < y1 else -1
    err = dx + dy
    x, y = x0, y0
    while True:
        _plot(fb, x, y)
        if x == x1 and y == y1:
            break
        e2 = 2 * err
        if e2 >= dy:
            err += dy
            x += sx
        if e2 <= dx:
            err += dx
            y += sy


def _plot8(fb: Framebuffer, cx: int, cy: int, x: int, y: int) -> None:
    for px, py in (
        (cx + x, cy + y),
        (cx - x, cy + y),
        (cx + x, cy - y),
        (cx - x, cy - y),
        (cx + y, cy + x),
        (cx - y, cy + x),
        (cx + y, cy - x),
        (cx - y, cy - x),
    ):
        _plot(fb, px, py)


def _circle_intersects(cx: int, cy: int, r: int) -> bool:
    return cx - r < WIDTH and cx + r >= 0 and cy - r < HEIGHT and cy + r >= 0


def _isqrt(n: int) -> int:
    if n <= 0:
        return 0
    x = n
    y = (x + 1) // 2
    while y < x:
        x = y
        y = (x + n // x) // 2
    return x


def _fill_row(fb: Framebuffer, x0: int, x1: int, y: int) -> None:
    if not (0 <= y < HEIGHT):
        return
    if x1 < 0 or x0 >= WIDTH:
        return
    x0 = max(x0, 0)
    x1 = min(x1, WIDTH - 1)
    for x in range(x0, x1 + 1):
        fb.set_pixel(x, y, True)


def draw_circle(fb: Framebuffer, cx: int, cy: int, r: int) -> None:
    if r < 0 or not _circle_intersects(cx, cy, r):
        return
    # 大半径使用屏幕空间扫描线，避免无意义的大循环
    if r > 4096:
        r2 = r * r
        for y in range(HEIGHT):
            dy = y - cy
            if dy * dy > r2:
                continue
            half = _isqrt(r2 - dy * dy)
            _plot(fb, cx - half, y)
            if half:
                _plot(fb, cx + half, y)
        return
    x = r
    y = 0
    err = 1 - r
    while x >= y:
        _plot8(fb, cx, cy, x, y)
        y += 1
        if err < 0:
            err += 2 * y + 1
        else:
            x -= 1
            err += 2 * (y - x) + 1


def fill_circle(fb: Framebuffer, cx: int, cy: int, r: int) -> None:
    if r < 0 or not _circle_intersects(cx, cy, r):
        return
    if r > 4096:
        r2 = r * r
        for y in range(HEIGHT):
            dy = y - cy
            if dy * dy > r2:
                continue
            half = _isqrt(r2 - dy * dy)
            _fill_row(fb, cx - half, cx + half, y)
        return
    x = r
    y = 0
    err = 1 - r
    while x >= y:
        _fill_row(fb, cx - x, cx + x, cy + y)
        _fill_row(fb, cx - x, cx + x, cy - y)
        _fill_row(fb, cx - y, cx + y, cy + x)
        _fill_row(fb, cx - y, cx + y, cy - x)
        y += 1
        if err < 0:
            err += 2 * y + 1
        else:
            x -= 1
            err += 2 * (y - x) + 1


def draw_triangle(
    fb: Framebuffer,
    x0: int,
    y0: int,
    x1: int,
    y1: int,
    x2: int,
    y2: int,
) -> None:
    draw_line(fb, x0, y0, x1, y1)
    draw_line(fb, x1, y1, x2, y2)
    draw_line(fb, x2, y2, x0, y0)


def _interpolate_x(x_a: int, y_a: int, x_b: int, y_b: int, y: int) -> int:
    dy = y_b - y_a
    if dy == 0:
        return x_a
    return x_a + (x_b - x_a) * (y - y_a) // dy


def fill_triangle(
    fb: Framebuffer,
    x0: int,
    y0: int,
    x1: int,
    y1: int,
    x2: int,
    y2: int,
) -> None:
    v = sorted([(x0, y0), (x1, y1), (x2, y2)], key=lambda p: p[1])
    (ax, ay), (bx, by), (cx, cy) = v
    if cy == ay:
        return
    y_start = max(ay, 0)
    y_end = min(cy, HEIGHT - 1)
    if y_start > y_end:
        return
    for y in range(y_start, y_end + 1):
        left = _interpolate_x(ax, ay, cx, cy, y)
        right = _interpolate_x(ax, ay, bx, by, y) if y < by else _interpolate_x(bx, by, cx, cy, y)
        if left > right:
            left, right = right, left
        _fill_row(fb, left, right, y)


def draw_hline_dotted(fb: Framebuffer, x: int, y: int, length: int) -> None:
    if y >= HEIGHT or length == 0 or x >= WIDTH:
        return
    end = min(x + length, WIDTH)
    for px in range(x, end, 2):
        fb.set_pixel(px, y, True)


def draw_vline_dotted(fb: Framebuffer, x: int, y: int, length: int) -> None:
    if x >= WIDTH or length == 0 or y >= HEIGHT:
        return
    end = min(y + length, HEIGHT)
    for py in range(y, end, 2):
        fb.set_pixel(x, py, True)
