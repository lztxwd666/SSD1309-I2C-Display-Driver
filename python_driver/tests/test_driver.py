"""基础单元测试，使用 MockBus，无需硬件"""

import unittest

from ssd1309_driver import Display, DisplayConfig, Framebuffer
from ssd1309_driver.graphics import draw_text
from ssd1309_driver.ssd1309 import Ssd1309


class MockBus:
    def __init__(self):
        self.writes = []

    def write_command(self, data):
        self.writes.append(("cmd", bytes(data)))

    def write_data(self, data):
        self.writes.append(("data", bytes(data)))

    def read_byte(self):
        return 0x01

    def close(self):
        pass


class FramebufferTest(unittest.TestCase):
    def test_set_get_pixel_and_dirty(self):
        fb = Framebuffer()
        self.assertIsNone(fb.dirty_rect())
        fb.set_pixel(5, 5, True)
        self.assertTrue(fb.get_pixel(5, 5))
        self.assertEqual(fb.dirty_rect(), (5, 5, 1, 1))

    def test_clear_marks_full_dirty(self):
        fb = Framebuffer()
        fb.clear()
        self.assertEqual(fb.dirty_rect(), (0, 0, 128, 64))


class Ssd1309Test(unittest.TestCase):
    def test_init_writes_display_on(self):
        bus = MockBus()
        Ssd1309.init(bus)
        self.assertTrue(any(cmd == ("cmd", b"\xaf") for cmd in bus.writes))


class DisplayTest(unittest.TestCase):
    def test_render_via_mock(self):
        bus = MockBus()
        config = DisplayConfig(bus_id=1, addr=0x3C)
        display = Display.from_device(bus, config)
        draw_text(display.framebuffer, 0, 0, "OK")
        display.render()
        self.assertGreater(display.stats.frames_pushed, 0)


if __name__ == "__main__":
    unittest.main()
