"""验证 Display 对帧拒绝与链路错误的处理，无需硬件。"""

import unittest
from unittest.mock import Mock, patch

from ssd1309_driver import Display, DisplayConfig, RenderStatus
from ssd1309_driver.errors import FrameRejectedError, I2cError


class RejectingDriver:
    """模拟可安全跳过的帧拒绝。"""

    def __init__(self):
        self.calls = 0

    def push_frame(self, _framebuffer):
        self.calls += 1
        raise FrameRejectedError("mock: 接收端 CRC 拒绝")


class FailingDriver:
    """模拟推帧/状态读取抛出指定异常的控制器。"""

    def __init__(self, error):
        self.error = error
        self.read_calls = 0

    def push_frame(self, _framebuffer):
        raise self.error

    def read_status(self):
        self.read_calls += 1
        raise self.error


class DisplayFrameRejectedTest(unittest.TestCase):
    def test_render_robust_skips_without_recover(self):
        driver = RejectingDriver()
        display = Display(driver, DisplayConfig(bus_id=1, addr=0x3C))
        display.set_logger(lambda _msg: None)
        status = display.render_robust()
        self.assertEqual(status, RenderStatus.SKIPPED)
        self.assertEqual(driver.calls, 1)
        self.assertEqual(display.stats.errors, 1)
        self.assertEqual(display.stats.frames_rejected, 1)
        self.assertEqual(display.stats.frames_skipped, 1)
        self.assertEqual(display.stats.recoveries, 0)

    def test_render_preserves_i2c_error(self):
        display = Display(
            FailingDriver(I2cError("mock: 总线错误")),
            DisplayConfig(bus_id=1, addr=0x3C),
        )
        with self.assertRaises(I2cError):
            display.render()
        self.assertEqual(display.stats.errors, 1)

    def test_read_status_error_counts_in_errors_only(self):
        display = Display(
            FailingDriver(FrameRejectedError("mock: 状态帧 CRC 拒绝")),
            DisplayConfig(bus_id=1, addr=0x3C),
        )
        with self.assertRaises(FrameRejectedError):
            display.read_status()
        self.assertEqual(display.stats.errors, 1)
        self.assertEqual(display.stats.frames_rejected, 0)

    def test_unknown_result_reopens_and_redraws_after_panel_initialization(self):
        old_driver = FailingDriver(I2cError("I2C v3 结果未知"))
        old_driver.bus = Mock()
        display = Display(old_driver, DisplayConfig(bus_id=1, addr=0x3C))
        display.set_logger(lambda _msg: None)
        bus = Mock()
        driver = Mock()
        with (
            patch("ssd1309_driver.display.I2cBus") as factory,
            patch(
                "ssd1309_driver.display.Ssd1309.init_with_config", return_value=driver
            ) as init,
        ):
            factory.return_value.open.return_value = bus
            self.assertEqual(display.render_robust(), RenderStatus.RECOVERED)
        old_driver.bus.close.assert_called_once_with()
        init.assert_called_once_with(bus, display.config)
        driver.push_frame.assert_called_once_with(display.framebuffer)
        self.assertIsNone(display.framebuffer.dirty_rect())
        self.assertEqual(display.stats.recoveries, 1)
        self.assertEqual(display.stats.frames_rejected, 0)


if __name__ == "__main__":
    unittest.main()
