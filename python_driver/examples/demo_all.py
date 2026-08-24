#!/usr/bin/env python3
"""全功能演示：一个文件覆盖驱动主要能力。

运行前先安装：
    cd python_driver
    python3 -m venv .venv
    source .venv/bin/activate
    pip install -r requirements.txt

运行：
    python examples/demo_all.py
"""

import time

from ssd1309_driver import (
    DisplayBuilder,
    DisplayRotation,
    PageBuffer,
    ScrollDirection,
    VerticalDirection,
)
from ssd1309_driver.graphics import (
    draw_circle,
    draw_line,
    draw_rect,
    draw_text,
    draw_text_inverted,
    fill_circle,
    fill_rect,
    fill_triangle,
)


def pause(ms: int) -> None:
    time.sleep(ms / 1000.0)


def main() -> None:
    display = (
        DisplayBuilder(1, 0x3C)
        .with_rotation(DisplayRotation.ROTATE_0)
        .build()
    )
    display.set_logger(lambda msg: print(f"[驱动日志] {msg}"))

    # 1. 文字与基础图形
    print("[1/7] 文字与基础图形")
    display.framebuffer.clear()
    draw_text(display.framebuffer, 0, 0, "Hello SSD1309!")
    draw_text(display.framebuffer, 0, 12, "PYTHON DRIVER")
    draw_rect(display.framebuffer, 0, 30, 50, 20)
    fill_rect(display.framebuffer, 60, 30, 40, 20)
    draw_line(display.framebuffer, 0, 55, 127, 55)
    draw_circle(display.framebuffer, 90, 12, 8)
    fill_circle(display.framebuffer, 110, 40, 8)
    fill_triangle(display.framebuffer, 100, 55, 120, 55, 110, 40)
    display.render()
    pause(3000)

    # 2. 脏矩形局部推帧
    print("[2/7] 脏矩形局部推帧")
    display.framebuffer.clear()
    draw_text(display.framebuffer, 0, 0, "DIRTY RECT")
    display.render_dirty()
    pause(2000)
    draw_text(display.framebuffer, 0, 12, "SECOND LINE")
    display.render_dirty()
    pause(2000)

    # 3. 软件滚动
    print("[3/7] 软件滚动")
    for _ in range(20):
        display.software_scroll_horizontal(ScrollDirection.LEFT, 1)
        pause(50)
    for _ in range(20):
        display.software_scroll_vertical(VerticalDirection.UP, 1)
        pause(50)

    # 4. 多页显示
    print("[4/7] 多页显示")
    pages = PageBuffer(2)
    pages.page_at_mut(0).clear()
    draw_text(pages.page_at_mut(0), 0, 0, "PAGE 1")
    pages.page_at_mut(1).clear()
    draw_text(pages.page_at_mut(1), 0, 0, "PAGE 2")
    display.show_page(pages.page_at(0))
    pause(1500)
    display.show_page(pages.page_at(1))
    pause(1500)

    # 5. 显示控制：对比度/反色/旋转/高级寄存器
    print("[5/7] 显示控制")
    display.set_rotation(DisplayRotation.ROTATE_0)
    display.framebuffer.clear()
    draw_text(display.framebuffer, 0, 20, "CONTROL")
    display.set_contrast(0x80)
    display.set_inverted(True)
    display.render()
    pause(2000)
    display.set_inverted(False)
    display.set_precharge_period(0x02, 0x0F)
    display.set_vcomh_level(0x50)
    display.set_charge_pump(True)
    display.render()
    pause(2000)
    # 恢复默认显示参数，避免后续画面异常
    display.set_contrast(0xCF)
    display.set_precharge_period(0x01, 0x0F)
    display.set_vcomh_level(0x40)
    display.set_charge_pump(True)

    # 6. 硬件滚动兼容入口
    print("[6/7] 硬件滚动兼容入口")
    display.framebuffer.clear()
    draw_text(display.framebuffer, 0, 0, "HW SCROLL?")
    display.render()
    display.hardware_scroll_horizontal(ScrollDirection.LEFT, 0, 7, 0x00)
    pause(2000)
    display.deactivate_scroll()

    # 7. 统计与结束画面
    print("[7/7] 统计")
    display.framebuffer.clear()
    draw_text(display.framebuffer, 0, 28, "ALL DONE")
    display.render()
    stats = display.stats
    print(
        f"推帧 {stats.frames_pushed}, 跳过 {stats.frames_skipped}, "
        f"恢复 {stats.recoveries}, 错误 {stats.errors}"
    )
    pause(3000)


if __name__ == "__main__":
    main()
