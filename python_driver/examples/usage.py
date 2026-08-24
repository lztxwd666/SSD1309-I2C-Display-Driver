#!/usr/bin/env python3
"""完整 API 使用参考，覆盖全部公共接口。"""

import time

from ssd1309_driver import (
    BlitMode,
    Display,
    DisplayBuilder,
    DisplayConfig,
    DisplayRotation,
    I2cError,
    NotInitializedError,
    PageBuffer,
    ScrollDirection,
    VerticalDirection,
)
from ssd1309_driver.graphics import (
    draw_circle,
    draw_hline,
    draw_hline_dotted,
    draw_line,
    draw_rect,
    draw_text,
    draw_text_inverted,
    draw_text_packed,
    draw_triangle,
    draw_vline,
    draw_vline_dotted,
    fill_circle,
    fill_rect,
    fill_triangle,
    text_width,
)


def pause(ms: int) -> None:
    time.sleep(ms / 1000.0)


def main() -> None:
    # 1. 初始化方式
    # 方式一：直接打开
    display = Display.open(1, 0x3C)

    # 方式二：Builder
    display = (
        DisplayBuilder(1, 0x3C)
        .with_contrast(0xCF)
        .with_inverted(False)
        .with_display_on(True)
        .with_rotation(DisplayRotation.ROTATE_0)
        .build()
    )

    # 方式三：DisplayConfig
    config = DisplayConfig(bus_id=1, addr=0x3C)
    # display = Display.open_config(config)

    # 日志回调
    display.set_logger(lambda msg: print(f"[驱动日志] {msg}"))

    # 2. 帧缓冲基础操作
    fb = display.framebuffer
    fb.clear()
    fb.set_pixel(10, 10, True)
    print("get_pixel:", fb.get_pixel(10, 10))
    fb.set_pixel(10, 10, False)
    fb.fill_all()
    fb.clear()
    print("dirty_rect:", fb.dirty_rect())
    fb.clear_dirty()

    # 3. 文字与图形
    draw_text(fb, 0, 0, "TEXT")
    draw_text_packed(fb, 0, 10, "PACKED")
    fill_rect(fb, 0, 20, 128, 9)
    draw_text_inverted(fb, 2, 21, "INVERTED")
    draw_rect(fb, 0, 32, 40, 20)
    fill_rect(fb, 50, 32, 40, 20)
    draw_hline(fb, 0, 55, 60)
    draw_vline(fb, 70, 30, 20)
    draw_hline_dotted(fb, 0, 60, 50)
    draw_vline_dotted(fb, 100, 30, 20)
    draw_line(fb, 0, 60, 127, 60)
    draw_circle(fb, 20, 10, 8)
    fill_circle(fb, 60, 10, 8)
    draw_triangle(fb, 80, 20, 110, 20, 95, 5)
    fill_triangle(fb, 80, 40, 110, 40, 95, 25)
    print("text_width:", text_width("HELLO", 1))
    display.render()
    pause(1000)

    # 4. 推帧方式
    fb.clear()
    draw_text(fb, 0, 0, "RENDER")
    display.render()              # 全帧
    pause(300)
    draw_text(fb, 0, 12, "DIRTY")
    display.render_dirty()        # 脏矩形
    pause(300)
    display.render_region(0, 0, 128, 64)
    pause(300)
    status = display.render_robust()
    print("render_robust:", status)
    pause(300)

    # 5. 多页显示
    pages = PageBuffer(2)
    pages.page_at_mut(0).clear()
    draw_text(pages.page_at_mut(0), 0, 0, "PAGE 1")
    pages.page_at_mut(1).clear()
    draw_text(pages.page_at_mut(1), 0, 0, "PAGE 2")
    display.show_page(pages.page_at(0))
    pause(500)
    display.show_page(pages.page_at(1))
    pause(500)
    display.scroll_to_page(pages.page_at(0), 8, 30)
    pause(500)
    display.scroll_to_page_frame(pages.page_at(1), 8, 4)
    pause(500)

    # 6. 软件滚动
    display.software_scroll_horizontal(ScrollDirection.LEFT, 1)
    display.software_scroll_vertical(VerticalDirection.UP, 1)
    pause(300)

    # 7. 显示控制
    display.set_contrast(0x80)
    display.set_inverted(True)
    display.set_entire_display_on(True)
    display.set_entire_display_on(False)
    display.set_inverted(False)
    display.set_contrast(0xCF)
    display.set_rotation(DisplayRotation.ROTATE_0)
    display.set_display_offset(0)
    display.set_start_line(0)
    display.set_multiplex_ratio(0x3F)
    display.set_clock(0, 8)
    display.set_precharge_period(0x01, 0x0F)
    display.set_vcomh_level(0x40)
    display.set_com_pins_config(0x12)
    display.set_charge_pump(True)
    display.render()
    pause(500)

    # 8. 状态读取
    st = display.read_status()
    print(
        f"status=0x{st:02X} busy={Display.status_busy(st)} "
        f"booster={Display.status_booster(st)}"
    )

    # 9. 硬件滚动兼容入口
    display.hardware_scroll_horizontal(ScrollDirection.LEFT, 0, 7, 0x00)
    display.deactivate_scroll()
    pause(300)

    # 10. 位图 blit
    fb.clear()
    bitmap = bytes([0b11110000, 0b11000000])
    fb.blit(0, 0, 10, 1, bitmap, BlitMode.SET)
    fb.blit(0, 2, 10, 1, bitmap, BlitMode.OVERWRITE)

    # 11. 恢复
    display.recover()
    display.render()

    # 12. 统计
    print("stats:", display.stats)

    # 13. 休眠/唤醒（示例中最后唤醒保持画面）
    # display.sleep()
    # display.wake()

    # 结束画面
    fb.clear()
    draw_text(fb, 0, 28, "API OK")
    display.render()
    print("done")


if __name__ == "__main__":
    try:
        main()
    except (I2cError, NotInitializedError) as e:
        print(f"驱动错误: {e}")
