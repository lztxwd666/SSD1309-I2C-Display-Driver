"""基础单元测试，使用 MockBus，无需硬件"""

import io
import runpy
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import call, patch

from ssd1309_driver import Display, DisplayConfig, Framebuffer, I2cError
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


class UsageExampleTest(unittest.TestCase):
    path = Path(__file__).resolve().parents[1] / "examples" / "usage.py"

    def test_usage_opens_once_before_recover(self):
        initial_bus = MockBus()
        recovered_bus = MockBus()
        with patch("ssd1309_driver.display.I2cBus") as factory, patch(
            "time.sleep"
        ), patch.object(initial_bus, "close") as close, redirect_stdout(
            io.StringIO()
        ) as output:
            factory.return_value.open.side_effect = [initial_bus, recovered_bus]
            runpy.run_path(str(self.path), run_name="__main__")

        self.assertEqual(factory.call_args_list, [call(1, 0x3C)] * 2)
        close.assert_called_once_with()
        self.assertIn("done", output.getvalue())

    def test_usage_reports_i2c_failure_as_unsuccessful_exit(self):
        with patch("ssd1309_driver.display.I2cBus") as factory:
            factory.return_value.open.side_effect = I2cError("同步失败")
            with self.assertRaisesRegex(SystemExit, "驱动错误: 同步失败"):
                runpy.run_path(str(self.path), run_name="__main__")


if __name__ == "__main__":
    unittest.main()
