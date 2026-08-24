"""1-bit 帧缓冲区

布局：8 页 × 128 列，buffer[page * 128 + col]
每个字节代表一列中的 8 个垂直像素，bit0 为顶部像素
"""

from typing import List, Optional, Tuple

from .errors import InvalidDataError

WIDTH = 128
HEIGHT = 64
BUFFER_SIZE = WIDTH * HEIGHT // 8


class BlitMode:
    """位图 blit 模式"""

    OVERWRITE = "overwrite"
    SET = "set"


class Framebuffer:
    """1-bit 帧缓冲，带脏矩形跟踪"""

    def __init__(self) -> None:
        self.buffer = bytearray(BUFFER_SIZE)
        self._dirty: Optional[Tuple[int, int, int, int]] = None

    def set_pixel(self, x: int, y: int, on: bool) -> None:
        """设置单个像素，越界静默忽略"""
        if not (0 <= x < WIDTH and 0 <= y < HEIGHT):
            return
        idx = (y >> 3) * WIDTH + x
        bit = y & 0x07
        if on:
            self.buffer[idx] |= 1 << bit
        else:
            self.buffer[idx] &= ~(1 << bit)
        self._mark_dirty(x, y)

    def get_pixel(self, x: int, y: int) -> bool:
        """读取单个像素，越界返回 False"""
        if not (0 <= x < WIDTH and 0 <= y < HEIGHT):
            return False
        idx = (y >> 3) * WIDTH + x
        return bool(self.buffer[idx] & (1 << (y & 0x07)))

    def clear(self) -> None:
        """清空整个缓冲区"""
        self.buffer = bytearray(BUFFER_SIZE)
        self._dirty = (0, 0, WIDTH, HEIGHT)

    def fill_all(self) -> None:
        """填充整个缓冲区（全白）"""
        self.buffer = bytearray([0xFF]) * BUFFER_SIZE
        self._dirty = (0, 0, WIDTH, HEIGHT)

    def as_bytes(self) -> bytes:
        """返回内部缓冲区副本"""
        return bytes(self.buffer)

    def dirty_rect(self) -> Optional[Tuple[int, int, int, int]]:
        """返回脏矩形 (x, y, w, h)，无修改时返回 None"""
        if self._dirty is None:
            return None
        x0, y0, x1, y1 = self._dirty
        return (x0, y0, x1 - x0, y1 - y0)

    def clear_dirty(self) -> None:
        """清除脏矩形记录"""
        self._dirty = None

    def mark_all_dirty(self) -> None:
        """标记整个屏幕为脏"""
        self._dirty = (0, 0, WIDTH, HEIGHT)

    def blit(
        self,
        x: int,
        y: int,
        w: int,
        h: int,
        data: bytes,
        mode: str = BlitMode.SET,
    ) -> None:
        """绘制线性 1-bit 位图

        data 为逐行打包位图，MSB 优先；长度必须足够，否则抛出 ValueError
        """
        if w == 0 or h == 0:
            return
        x0 = min(x, WIDTH)
        y0 = min(y, HEIGHT)
        x1 = min(x + w, WIDTH)
        y1 = min(y + h, HEIGHT)
        if x0 >= x1 or y0 >= y1:
            return

        row_bytes = (w + 7) // 8
        required = row_bytes * h
        if len(data) < required:
            raise InvalidDataError(
                f"位图数据长度不足：需要 {required} 字节，实际 {len(data)} 字节"
            )

        src_x = x0 - x
        src_y = y0 - y
        for row in range(y1 - y0):
            for col in range(x1 - x0):
                sx = src_x + col
                byte = data[(src_y + row) * row_bytes + sx // 8]
                on = bool(byte & (0x80 >> (sx % 8)))
                if on:
                    self.set_pixel(x0 + col, y0 + row, True)
                elif mode == BlitMode.OVERWRITE:
                    self.set_pixel(x0 + col, y0 + row, False)

    def _mark_dirty(self, x: int, y: int) -> None:
        if self._dirty is None:
            self._dirty = (x, y, x + 1, y + 1)
            return
        x0, y0, x1, y1 = self._dirty
        self._dirty = (
            min(x0, x),
            min(y0, y),
            max(x1, x + 1),
            max(y1, y + 1),
        )


class PageBuffer:
    """多页帧缓冲，支持循环翻页"""

    def __init__(self, count: int):
        if count < 0:
            raise ValueError("页数不能为负数")
        self.pages: List[Framebuffer] = [Framebuffer() for _ in range(count)]
        self.current = 0

    def page(self) -> Framebuffer:
        """当前页的可变引用"""
        if not self.pages:
            raise IndexError("没有可用页面")
        return self.pages[self.current]

    def page_at(self, index: int) -> Optional[Framebuffer]:
        """指定页的引用，越界返回 None"""
        if 0 <= index < len(self.pages):
            return self.pages[index]
        return None

    def page_at_mut(self, index: int) -> Optional[Framebuffer]:
        """指定页的可变引用，越界返回 None"""
        return self.page_at(index)

    def current_index(self) -> int:
        return self.current

    def __len__(self) -> int:
        return len(self.pages)

    def show(self, index: int) -> bool:
        """切换到指定页，越界返回 False"""
        if 0 <= index < len(self.pages):
            self.current = index
            return True
        return False

    def next_page(self) -> int:
        """翻到下一页（循环）"""
        if not self.pages:
            return 0
        self.current = (self.current + 1) % len(self.pages)
        return self.current

    def prev_page(self) -> int:
        """翻到上一页（循环）"""
        if not self.pages:
            return 0
        self.current = (self.current - 1) % len(self.pages)
        return self.current
