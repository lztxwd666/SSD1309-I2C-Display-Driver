"""Display 对接收端 NACK 的处理测试，无需硬件"""

import unittest

from ssd1309_driver import Display, DisplayConfig, RenderStatus
from ssd1309_driver.errors import FrameRejectedError


class RejectingDriver:
    """模拟推帧时被接收端 NACK 的控制器。"""

    def __init__(self):
        self.calls = 0

    def push_frame(self, _framebuffer):
        self.calls += 1
        raise FrameRejectedError("mock: 接收端 CRC 拒绝")


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


if __name__ == "__main__":
    unittest.main()
