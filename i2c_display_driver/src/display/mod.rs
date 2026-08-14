//! 显示硬件驱动层 —— SSD1309 OLED 通过 I2C 通信。
//!
//! Display 结构体将 I2C 总线、SSD1309 控制器和帧缓冲区组合为统一接口。

mod framebuffer;
mod i2c_bus;
mod ssd1309;

pub use framebuffer::Framebuffer;

use crate::utils::AppError;
use i2c_bus::I2cBus;
use ssd1309::Ssd1309;

/// OLED 显示器顶层句柄。
pub struct Display {
    /// Option 包装允许 `recover()` 先关闭旧连接再打开新连接，
    /// 避免两个 fd 同时指向 /dev/i2c-N 导致 RP1 控制器状态混乱。
    driver: Option<Ssd1309>,
    pub framebuffer: Framebuffer,
}

impl Display {
    /// 打开并初始化显示器。
    pub fn open(bus_id: u8, addr: u8) -> Result<Self, AppError> {
        let bus = I2cBus::open(bus_id, addr)?;
        let driver = Ssd1309::init(bus)?;
        Ok(Self {
            driver: Some(driver),
            framebuffer: Framebuffer::new(),
        })
    }

    /// 将帧缓冲区内容推送到 OLED。
    /// 当驱动未初始化（recover 失败后）时返回错误而非 panic。
    pub fn render(&mut self) -> Result<(), AppError> {
        let driver = self
            .driver
            .as_mut()
            .ok_or_else(|| AppError::Config("显示驱动未初始化".into()))?;
        Ok(driver.push_frame(&self.framebuffer)?)
    }

    /// 设置对比度（0-255）。
    /// 当驱动未初始化（recover 失败后）时返回错误而非 panic。
    pub fn set_contrast(&mut self, val: u8) -> Result<(), AppError> {
        let driver = self
            .driver
            .as_mut()
            .ok_or_else(|| AppError::Config("显示驱动未初始化".into()))?;
        Ok(driver.set_contrast(val)?)
    }

    /// 关闭 OLED 显示（进入休眠模式，0xAE）。
    /// 当驱动未初始化时返回错误而非 panic。
    pub fn sleep(&mut self) -> Result<(), AppError> {
        let driver = self
            .driver
            .as_mut()
            .ok_or_else(|| AppError::Config("显示驱动未初始化".into()))?;
        Ok(driver.sleep()?)
    }

    /// 重置 I2C 总线并重新初始化 SSD1309。
    ///
    /// 先关闭旧 I2C 连接（确保旧 fd 释放），再打开新连接并重新初始化
    /// OLED 控制器。用于 I2C 总线卡死（SDA stuck / lost arbitration）后的恢复。
    pub fn recover(&mut self, bus_id: u8, addr: u8) -> Result<(), AppError> {
        eprintln!("[显示] 正在重置 I2C 总线...");
        // 先关闭旧连接（drop 触发旧 fd close），再打开新的
        self.driver = None;
        let bus = I2cBus::open(bus_id, addr)?;
        self.driver = Some(Ssd1309::init(bus)?);
        eprintln!("[显示] I2C 总线重置成功");
        Ok(())
    }
}
