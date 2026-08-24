"""SSD1309 OLED 控制器驱动"""

import time
from enum import Enum
from typing import Any

from .framebuffer import Framebuffer, HEIGHT, WIDTH


class DisplayRotation(Enum):
    """显示旋转方向"""

    ROTATE_0 = 0
    ROTATE_180 = 1


class ScrollDirection(Enum):
    """水平滚动方向"""

    RIGHT = 0
    LEFT = 1


class VerticalDirection(Enum):
    """垂直滚动方向"""

    UP = 0
    DOWN = 1


class Ssd1309:
    """SSD1309 控制器，泛型底层 I2C 设备"""

    def __init__(self, bus: Any):
        self.bus = bus

    @classmethod
    def init(
        cls,
        bus: Any,
        contrast: int = 0xCF,
        inverted: bool = False,
        display_on: bool = True,
        rotation: DisplayRotation = DisplayRotation.ROTATE_0,
        display_offset: int = 0,
        start_line: int = 0,
        multiplex_ratio: int = 0x3F,
        clock_divide_ratio: int = 0,
        clock_frequency: int = 8,
        precharge_phase1: int = 0x01,
        precharge_phase2: int = 0x0F,
        vcomh_level: int = 0x40,
        com_pins_config: int = 0x12,
        charge_pump_enabled: bool = True,
    ) -> "Ssd1309":
        """初始化控制器"""
        bus.write_command(bytes([0xAE]))
        clock = ((clock_frequency & 0x0F) << 4) | (clock_divide_ratio & 0x0F)
        bus.write_command(bytes([0xD5, clock]))
        bus.write_command(bytes([0xA8, multiplex_ratio]))
        bus.write_command(bytes([0xD3, display_offset]))
        bus.write_command(bytes([0x40 | (start_line & 0x3F)]))
        bus.write_command(bytes([0x8D, 0x14 if charge_pump_enabled else 0x10]))
        bus.write_command(bytes([0xAD, 0x8A]))
        time.sleep(0.1)
        bus.write_command(bytes([0x20, 0x02]))
        if rotation == DisplayRotation.ROTATE_0:
            bus.write_command(bytes([0xA1]))
            bus.write_command(bytes([0xC8]))
        else:
            bus.write_command(bytes([0xA0]))
            bus.write_command(bytes([0xC0]))
        bus.write_command(bytes([0xDA, com_pins_config]))
        bus.write_command(bytes([0x81, contrast]))
        precharge = ((precharge_phase2 & 0x0F) << 4) | (precharge_phase1 & 0x0F)
        bus.write_command(bytes([0xD9, precharge]))
        bus.write_command(bytes([0xDB, vcomh_level]))
        bus.write_command(bytes([0xA4]))
        bus.write_command(bytes([0xA7 if inverted else 0xA6]))
        if display_on:
            bus.write_command(bytes([0xAF]))
        return cls(bus)

    @classmethod
    def init_with_config(cls, bus: Any, config: Any) -> "Ssd1309":
        """按配置对象初始化"""
        return cls.init(
            bus,
            contrast=config.contrast,
            inverted=config.inverted,
            display_on=config.display_on,
            rotation=config.rotation,
            display_offset=config.display_offset,
            start_line=config.start_line,
            multiplex_ratio=config.multiplex_ratio,
            clock_divide_ratio=config.clock_divide_ratio,
            clock_frequency=config.clock_frequency,
            precharge_phase1=config.precharge_phase1,
            precharge_phase2=config.precharge_phase2,
            vcomh_level=config.vcomh_level,
            com_pins_config=config.com_pins_config,
            charge_pump_enabled=config.charge_pump_enabled,
        )

    def push_frame(self, fb: Framebuffer) -> None:
        """逐页推送 1024 字节帧数据"""
        data = bytes(fb.buffer)
        for page in range(HEIGHT // 8):
            self.bus.write_command(bytes([0xB0 | page]))
            self.bus.write_command(bytes([0x00, 0x10]))
            start = page * WIDTH
            self.bus.write_data(data[start:start + WIDTH])

    def render_region(self, fb: Framebuffer, x: int, y: int, w: int, h: int) -> None:
        """局部推帧"""
        x = min(x, WIDTH)
        w = min(w, WIDTH - x)
        if w <= 0:
            return
        y0 = min(y, HEIGHT)
        y1 = min(y + h, HEIGHT)
        if y0 >= y1:
            return
        page0 = y0 // 8
        page1 = (y1 - 1) // 8
        data = bytes(fb.buffer)
        for page in range(page0, page1 + 1):
            self.bus.write_command(bytes([0xB0 | page]))
            col_low = x & 0x0F
            col_high = 0x10 | ((x >> 4) & 0x07)
            self.bus.write_command(bytes([col_low, col_high]))
            start = page * WIDTH + x
            self.bus.write_data(data[start:start + w])

    def set_contrast(self, val: int) -> None:
        self.bus.write_command(bytes([0x81, val]))

    def set_inverted(self, inverted: bool) -> None:
        self.bus.write_command(bytes([0xA7 if inverted else 0xA6]))

    def set_entire_display_on(self, on: bool) -> None:
        self.bus.write_command(bytes([0xA5 if on else 0xA4]))

    def sleep(self) -> None:
        self.bus.write_command(bytes([0xAE]))

    def wake(self) -> None:
        self.bus.write_command(bytes([0xAF]))

    def read_status(self) -> int:
        return self.bus.read_byte()

    def set_rotation(self, rotation: DisplayRotation) -> None:
        if rotation == DisplayRotation.ROTATE_0:
            self.bus.write_command(bytes([0xA1]))
            self.bus.write_command(bytes([0xC8]))
        else:
            self.bus.write_command(bytes([0xA0]))
            self.bus.write_command(bytes([0xC0]))

    def set_display_offset(self, offset: int) -> None:
        self.bus.write_command(bytes([0xD3, offset & 0x3F]))

    def set_start_line(self, line: int) -> None:
        self.bus.write_command(bytes([0x40 | (line & 0x3F)]))

    def set_multiplex_ratio(self, ratio: int) -> None:
        self.bus.write_command(bytes([0xA8, ratio]))

    def set_clock(self, divide_ratio: int, frequency: int) -> None:
        value = ((frequency & 0x0F) << 4) | (divide_ratio & 0x0F)
        self.bus.write_command(bytes([0xD5, value]))

    def set_precharge_period(self, phase1: int, phase2: int) -> None:
        value = ((phase2 & 0x0F) << 4) | (phase1 & 0x0F)
        self.bus.write_command(bytes([0xD9, value]))

    def set_vcomh_level(self, level: int) -> None:
        self.bus.write_command(bytes([0xDB, level]))

    def set_com_pins_config(self, config: int) -> None:
        self.bus.write_command(bytes([0xDA, config]))

    def set_charge_pump(self, enabled: bool) -> None:
        self.bus.write_command(bytes([0x8D, 0x14 if enabled else 0x10]))

    def hardware_scroll_horizontal(
        self,
        direction: ScrollDirection,
        start_page: int,
        end_page: int,
        speed: int,
    ) -> None:
        cmd = 0x26 if direction == ScrollDirection.RIGHT else 0x27
        self.bus.write_command(
            bytes([cmd, 0x00, start_page & 0x07, speed, end_page & 0x07, 0x00, 0xFF])
        )
        self.bus.write_command(bytes([0x2F]))

    def deactivate_scroll(self) -> None:
        self.bus.write_command(bytes([0x2E]))
