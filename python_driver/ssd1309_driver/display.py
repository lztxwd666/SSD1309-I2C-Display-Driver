"""OLED 显示器顶层句柄"""

import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Callable, Optional

from .errors import DriverError, NotInitializedError
from .framebuffer import Framebuffer, PageBuffer, WIDTH, HEIGHT
from .i2c_bus import I2cBus
from .ssd1309 import DisplayRotation, ScrollDirection, VerticalDirection, Ssd1309

RECOVER_COOLDOWN = 2.0


class RenderStatus(Enum):
    """render_robust 返回状态"""

    OK = "ok"
    RECOVERED = "recovered"
    SKIPPED = "skipped"


@dataclass
class DriverStats:
    """驱动运行统计"""

    frames_pushed: int = 0
    frames_skipped: int = 0
    recoveries: int = 0
    errors: int = 0


@dataclass
class DisplayConfig:
    """显示初始化配置"""

    bus_id: int
    addr: int
    contrast: int = 0xCF
    inverted: bool = False
    display_on: bool = True
    rotation: DisplayRotation = DisplayRotation.ROTATE_0
    display_offset: int = 0
    start_line: int = 0
    multiplex_ratio: int = 0x3F
    clock_divide_ratio: int = 0
    clock_frequency: int = 8
    precharge_phase1: int = 0x01
    precharge_phase2: int = 0x0F
    vcomh_level: int = 0x40
    com_pins_config: int = 0x12
    charge_pump_enabled: bool = True


class DisplayBuilder:
    """链式初始化配置"""

    def __init__(self, bus_id: int, addr: int):
        self.config = DisplayConfig(bus_id=bus_id, addr=addr)

    def with_contrast(self, value: int) -> "DisplayBuilder":
        self.config.contrast = value
        return self

    def with_inverted(self, value: bool) -> "DisplayBuilder":
        self.config.inverted = value
        return self

    def with_display_on(self, value: bool) -> "DisplayBuilder":
        self.config.display_on = value
        return self

    def with_rotation(self, value: DisplayRotation) -> "DisplayBuilder":
        self.config.rotation = value
        return self

    def with_display_offset(self, value: int) -> "DisplayBuilder":
        self.config.display_offset = value
        return self

    def with_start_line(self, value: int) -> "DisplayBuilder":
        self.config.start_line = value
        return self

    def with_multiplex_ratio(self, value: int) -> "DisplayBuilder":
        self.config.multiplex_ratio = value
        return self

    def with_clock(self, divide_ratio: int, frequency: int) -> "DisplayBuilder":
        self.config.clock_divide_ratio = divide_ratio
        self.config.clock_frequency = frequency
        return self

    def with_precharge_period(self, phase1: int, phase2: int) -> "DisplayBuilder":
        self.config.precharge_phase1 = phase1
        self.config.precharge_phase2 = phase2
        return self

    def with_vcomh_level(self, value: int) -> "DisplayBuilder":
        self.config.vcomh_level = value
        return self

    def with_com_pins_config(self, value: int) -> "DisplayBuilder":
        self.config.com_pins_config = value
        return self

    def with_charge_pump(self, enabled: bool) -> "DisplayBuilder":
        self.config.charge_pump_enabled = enabled
        return self

    def build(self) -> "Display":
        return Display.open_config(self.config)


class Display:
    """SSD1309 顶层句柄"""

    def __init__(
        self,
        driver: Ssd1309,
        config: DisplayConfig,
        framebuffer: Optional[Framebuffer] = None,
    ):
        self._driver = driver
        self.config = config
        self.framebuffer = framebuffer if framebuffer is not None else Framebuffer()
        self._entire_display_on = False
        self.stats = DriverStats()
        self._logger: Optional[Callable[[str], None]] = None
        self._last_recover_failed: Optional[float] = None
        self._scroll_source: Optional[tuple] = None

    @classmethod
    def open(cls, bus_id: int, addr: int) -> "Display":
        return cls.open_config(DisplayConfig(bus_id=bus_id, addr=addr))

    @classmethod
    def open_config(cls, config: DisplayConfig) -> "Display":
        bus = I2cBus(config.bus_id, config.addr).open()
        try:
            driver = Ssd1309.init_with_config(bus, config)
        except Exception:
            bus.close()
            raise
        return cls(driver, config)

    @classmethod
    def from_device(cls, bus, config: DisplayConfig) -> "Display":
        driver = Ssd1309.init_with_config(bus, config)
        return cls(driver, config)

    def set_logger(self, logger: Callable[[str], None]) -> None:
        self._logger = logger

    def _log(self, msg: str) -> None:
        if self._logger is not None:
            self._logger(msg)
        else:
            print(f"[显示] {msg}")

    def render(self) -> None:
        if self._driver is None:
            raise NotInitializedError("驱动未初始化")
        try:
            self._driver.push_frame(self.framebuffer)
            self.stats.frames_pushed += 1
            self.framebuffer.clear_dirty()
        except Exception as e:
            self.stats.errors += 1
            raise DriverError(str(e)) from e

    def render_region(self, x: int, y: int, w: int, h: int) -> None:
        if w == 0 or h == 0 or x >= WIDTH or y >= HEIGHT:
            return
        if self._driver is None:
            raise NotInitializedError("驱动未初始化")
        try:
            self._driver.render_region(self.framebuffer, x, y, w, h)
            self.stats.frames_pushed += 1
            dirty = self.framebuffer.dirty_rect()
            if dirty is not None:
                dx, dy, dw, dh = dirty
                if x <= dx and y <= dy and x + w >= dx + dw and y + h >= dy + dh:
                    self.framebuffer.clear_dirty()
        except Exception as e:
            self.stats.errors += 1
            raise DriverError(str(e)) from e

    def render_dirty(self) -> None:
        dirty = self.framebuffer.dirty_rect()
        if dirty is not None:
            self.render_region(*dirty)

    def render_robust(self) -> RenderStatus:
        try:
            self.render()
            return RenderStatus.OK
        except DriverError as e:
            self._log(f"帧推送失败: {e}")
            if self._recover_in_cooldown():
                self.stats.frames_skipped += 1
                return RenderStatus.SKIPPED
            try:
                self.recover()
                self.stats.recoveries += 1
            except DriverError as e2:
                self._log(f"I2C 总线重置失败: {e2}")
                self.stats.frames_skipped += 1
                return RenderStatus.SKIPPED
            try:
                self.render()
                return RenderStatus.RECOVERED
            except DriverError as e3:
                self._log(f"恢复后首帧仍失败: {e3}，跳过本帧")
                self.stats.frames_skipped += 1
                return RenderStatus.SKIPPED

    def _recover_in_cooldown(self) -> bool:
        if self._last_recover_failed is None:
            return False
        return time.monotonic() - self._last_recover_failed < RECOVER_COOLDOWN

    def clear(self) -> None:
        self.framebuffer.clear()
        self.render()

    def fill(self) -> None:
        self.framebuffer.fill_all()
        self.render()

    def set_contrast(self, val: int) -> None:
        self._require_driver().set_contrast(val)
        self.config.contrast = val

    def set_inverted(self, inverted: bool) -> None:
        self._require_driver().set_inverted(inverted)
        self.config.inverted = inverted

    def set_entire_display_on(self, on: bool) -> None:
        self._require_driver().set_entire_display_on(on)
        self._entire_display_on = on

    def sleep(self) -> None:
        self._require_driver().sleep()
        self.config.display_on = False

    def wake(self) -> None:
        self._require_driver().wake()
        self.config.display_on = True

    def read_status(self) -> int:
        return self._require_driver().read_status()

    def set_rotation(self, rotation: DisplayRotation) -> None:
        self._require_driver().set_rotation(rotation)
        self.config.rotation = rotation

    def set_display_offset(self, offset: int) -> None:
        self._require_driver().set_display_offset(offset)
        self.config.display_offset = offset

    def set_start_line(self, line: int) -> None:
        self._require_driver().set_start_line(line)
        self.config.start_line = line

    def set_multiplex_ratio(self, ratio: int) -> None:
        self._require_driver().set_multiplex_ratio(ratio)
        self.config.multiplex_ratio = ratio

    def set_clock(self, divide_ratio: int, frequency: int) -> None:
        self._require_driver().set_clock(divide_ratio, frequency)
        self.config.clock_divide_ratio = divide_ratio
        self.config.clock_frequency = frequency

    def set_precharge_period(self, phase1: int, phase2: int) -> None:
        self._require_driver().set_precharge_period(phase1, phase2)
        self.config.precharge_phase1 = phase1
        self.config.precharge_phase2 = phase2

    def set_vcomh_level(self, level: int) -> None:
        self._require_driver().set_vcomh_level(level)
        self.config.vcomh_level = level

    def set_com_pins_config(self, config: int) -> None:
        self._require_driver().set_com_pins_config(config)
        self.config.com_pins_config = config

    def set_charge_pump(self, enabled: bool) -> None:
        self._require_driver().set_charge_pump(enabled)
        self.config.charge_pump_enabled = enabled

    def software_scroll_horizontal(self, direction: ScrollDirection, offset: int) -> None:
        Display._scroll_horizontal_bits(self.framebuffer.buffer, direction, offset)
        self.framebuffer.mark_all_dirty()
        self.render()

    def software_scroll_vertical(self, direction: VerticalDirection, offset: int) -> None:
        Display._scroll_vertical_bits(self.framebuffer.buffer, direction, offset)
        self.framebuffer.mark_all_dirty()
        self.render()

    def hardware_scroll_horizontal(
        self,
        direction: ScrollDirection,
        start_page: int,
        end_page: int,
        speed: int,
    ) -> None:
        self._require_driver().hardware_scroll_horizontal(direction, start_page, end_page, speed)

    def deactivate_scroll(self) -> None:
        self._require_driver().deactivate_scroll()

    def show_page(self, page: Framebuffer) -> None:
        self.framebuffer.buffer[:] = page.buffer
        self.framebuffer.mark_all_dirty()
        self.render()

    def scroll_to_page(self, page: Framebuffer, steps: int, step_ms: int) -> None:
        steps = max(steps, 1)
        for k in range(steps + 1):
            self.scroll_to_page_frame(page, steps, k)
            if k < steps:
                time.sleep(step_ms / 1000.0)

    def scroll_to_page_frame(self, page: Framebuffer, steps: int, k: int) -> None:
        steps = max(steps, 1)
        if self._scroll_source is None or self._scroll_source[1] != steps:
            self._scroll_source = (bytes(self.framebuffer.buffer), steps)
        src, _ = self._scroll_source
        k = min(k, steps)
        offset = k * WIDTH // steps
        Display._blend_scroll_frame(self.framebuffer.buffer, src, bytes(page.buffer), offset)
        self.framebuffer.mark_all_dirty()
        self.render()
        if k >= steps:
            self._scroll_source = None

    def recover(self) -> None:
        self._log("正在重置 I2C 总线...")
        if self._driver is not None:
            self._driver.bus.close()
        self._driver = None
        try:
            bus = I2cBus(self.config.bus_id, self.config.addr).open()
            driver = Ssd1309.init_with_config(bus, self.config)
            if self._entire_display_on:
                driver.set_entire_display_on(True)
            self._driver = driver
            self._last_recover_failed = None
            self._log("I2C 总线重置成功")
        except Exception as e:
            self._last_recover_failed = time.monotonic()
            raise DriverError(str(e)) from e

    @staticmethod
    def status_busy(status: int) -> bool:
        """解析状态字节：忙标志（bit7）。"""
        return bool(status & 0x80)

    @staticmethod
    def status_booster(status: int) -> bool:
        """解析状态字节：电荷泵使能标志（bit0）。"""
        return bool(status & 0x01)

    def _require_driver(self) -> Ssd1309:
        if self._driver is None:
            raise NotInitializedError("驱动未初始化")
        return self._driver

    @staticmethod
    def _scroll_horizontal_bits(buffer: bytearray, direction: ScrollDirection, offset: int) -> None:
        offset %= WIDTH
        if offset == 0:
            return
        for page in range(HEIGHT // 8):
            start = page * WIDTH
            row = buffer[start:start + WIDTH]
            if direction == ScrollDirection.RIGHT:
                buffer[start:start + WIDTH] = row[-offset:] + row[:-offset]
            else:
                buffer[start:start + WIDTH] = row[offset:] + row[:offset]

    @staticmethod
    def _scroll_vertical_bits(buffer: bytearray, direction: VerticalDirection, offset: int) -> None:
        offset %= HEIGHT
        if offset == 0:
            return
        for col in range(WIDTH):
            v = 0
            for page in range(HEIGHT // 8):
                v |= buffer[page * WIDTH + col] << (page * 8)
            if direction == VerticalDirection.UP:
                v = ((v >> offset) | (v << (HEIGHT - offset))) & ((1 << HEIGHT) - 1)
            else:
                v = ((v << offset) | (v >> (HEIGHT - offset))) & ((1 << HEIGHT) - 1)
            for page in range(HEIGHT // 8):
                buffer[page * WIDTH + col] = (v >> (page * 8)) & 0xFF

    @staticmethod
    def _blend_scroll_frame(dst: bytearray, src: bytes, new: bytes, offset: int) -> None:
        offset = min(offset, WIDTH)
        for page in range(HEIGHT // 8):
            base = page * WIDTH
            for col in range(WIDTH):
                src_col = col + offset
                if src_col < WIDTH:
                    dst[base + col] = src[base + src_col]
                else:
                    dst[base + col] = new[base + src_col - WIDTH]
