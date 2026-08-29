#!/usr/bin/env python3
"""开机动画演示：底部进度条 + 中央微软经典四方块 Logo

用法：
    python examples/boot_animation.py [classic|italic]
"""

import sys
import time

from ssd1309_driver import Display
from ssd1309_driver.graphics import draw_rect, fill_rect

LOGO_X = 44
LOGO_Y = 8
SQUARE = 18
GAP = 3
ICON_W = 2 * SQUARE + GAP
ICON_H = 2 * SQUARE + GAP

BAR_X = 12
BAR_Y = 56
BAR_W = 104
BAR_H = 5

STEPS = 40
STEP_MS = 0
SKEW = 5


def in_block(col: int, row: int) -> bool:
    left_col = 0 <= col < SQUARE
    right_col = SQUARE + GAP <= col < ICON_W
    top_row = 0 <= row < SQUARE
    bottom_row = SQUARE + GAP <= row < ICON_H
    return (left_col or right_col) and (top_row or bottom_row)


def draw_logo(fb, progress: int, style: str) -> None:
    fill_w = ICON_W * progress // 100

    for row in range(ICON_H):
        py = LOGO_Y + row
        if py >= 64:
            break

        if style == "italic":
            offset = SKEW * (ICON_H - row - 1) // (ICON_H - 1)
        else:
            offset = 0

        for col in range(fill_w):
            if in_block(col, row):
                fb.set_pixel(LOGO_X + col + offset, py, True)


def draw_progress(fb, progress: int) -> None:
    draw_rect(fb, BAR_X, BAR_Y, BAR_W, BAR_H)
    fill = (BAR_W - 2) * progress // 100
    if fill > 0:
        fill_rect(fb, BAR_X + 1, BAR_Y + 1, fill, BAR_H - 2)


def main() -> None:
    style = sys.argv[1] if len(sys.argv) > 1 else "classic"
    if style not in ("classic", "italic"):
        print("用法: python boot_animation.py [classic|italic]")
        return

    display = Display.open(1, 0x3C)

    for step in range(STEPS + 1):
        progress = step * 100 // STEPS

        display.framebuffer.clear()
        draw_logo(display.framebuffer, progress, style)
        draw_progress(display.framebuffer, progress)
        display.render()

        if step < STEPS:
            time.sleep(STEP_MS / 1000.0)

    print("boot animation done")


if __name__ == "__main__":
    main()
