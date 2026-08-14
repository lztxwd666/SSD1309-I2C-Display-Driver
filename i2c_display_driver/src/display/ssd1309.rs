//! SSD1309 OLED 控制器驱动。
//!
//! 页寻址模式逐页推送帧数据，避免 Pi 5 RP1 I2C 控制器的大块传输限制。

use std::io;
use std::thread;
use std::time::Duration;

use super::framebuffer::Framebuffer;
use super::i2c_bus::I2cBus;

pub struct Ssd1309 {
    bus: I2cBus,
}

impl Ssd1309 {
    pub fn init(mut bus: I2cBus) -> io::Result<Self> {
        bus.write_command(&[0xAE])?; // 关闭显示（休眠模式）
        bus.write_command(&[0xD5, 0x80])?; // 时钟分频/振荡器频率
        bus.write_command(&[0xA8, 0x3F])?; // 多路复用比 64（128×64 面板）
        bus.write_command(&[0xD3, 0x00])?; // 显示偏移 0
        bus.write_command(&[0x40])?; // 起始行地址 0
        bus.write_command(&[0x8D, 0x14])?; // 电荷泵使能（内部 DC-DC）
        bus.write_command(&[0xAD, 0x8A])?; // SSD1309 DC-DC 转换器（缺少会导致花屏）
        thread::sleep(Duration::from_millis(100));
        bus.write_command(&[0x20, 0x02])?; // 页寻址模式（避免 Pi 5 RP1 大块传输限制）
        bus.write_command(&[0xA1])?; // 段重映射（水平翻转）
        bus.write_command(&[0xC8])?; // COM 扫描方向（垂直翻转）
        bus.write_command(&[0xDA, 0x12])?; // COM 引脚硬件配置（备选布局）
        bus.write_command(&[0x81, 0xCF])?; // 对比度 0xCF（最大值）
        bus.write_command(&[0xD9, 0xF1])?; // 预充电周期 1/F1
        bus.write_command(&[0xDB, 0x40])?; // VCOMH 取消选择级别
        bus.write_command(&[0xA4])?; // 正常显示模式（非全亮）
        bus.write_command(&[0xA6])?; // 正常显示（非反色）
        bus.write_command(&[0xAF])?; // 开启显示
        Ok(Self { bus })
    }

    /// 逐页推送 1024 字节帧数据（8 页 × 128 字节）。
    pub fn push_frame(&mut self, fb: &Framebuffer) -> io::Result<()> {
        let data = fb.as_bytes();
        for page in 0..8u8 {
            self.bus.write_command(&[0xB0 | page])?;
            self.bus.write_command(&[0x00, 0x10])?;
            self.bus.write_data(&data[page as usize * 128..][..128])?;
        }
        Ok(())
    }

    /// 设置对比度（0-255）。SSD1309 默认 0xCF。
    pub fn set_contrast(&mut self, val: u8) -> io::Result<()> {
        self.bus.write_command(&[0x81, val])
    }

    /// 进入休眠模式（关闭显示），GDDRAM 内容不受影响。
    pub fn sleep(&mut self) -> io::Result<()> {
        self.bus.write_command(&[0xAE])
    }
}
