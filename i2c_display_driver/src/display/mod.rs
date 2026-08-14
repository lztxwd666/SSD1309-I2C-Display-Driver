//! 显示硬件驱动层 —— SSD1309 OLED 通过 I2C 通信。
//!
//! Display 结构体将 I2C 总线、SSD1309 控制器和帧缓冲区组合为统一接口。

mod framebuffer;
mod i2c_bus;
mod ssd1309;

pub use framebuffer::Framebuffer;

use crate::DriverError;
use i2c_bus::I2cBus;
use ssd1309::Ssd1309;

/// OLED 显示器顶层句柄。
pub struct Display {
    /// Option 包装允许 `recover()` 先关闭旧连接再打开新连接，
    /// 避免两个 fd 同时指向 /dev/i2c-N 导致 RP1 控制器状态混乱。
    driver: Option<Ssd1309>,
    pub framebuffer: Framebuffer,
    bus_id: u8,
    addr: u8,
}

impl Display {
    /// 打开并初始化显示器。
    pub fn open(bus_id: u8, addr: u8) -> Result<Self, DriverError> {
        let bus = I2cBus::open(bus_id, addr)?;
        let driver = Ssd1309::init(bus)?;
        Ok(Self {
            driver: Some(driver),
            framebuffer: Framebuffer::new(),
            bus_id,
            addr,
        })
    }

    /// 将帧缓冲区内容推送到 OLED。
    /// 当驱动未初始化（recover 失败后）时返回错误而非 panic。
    pub fn render(&mut self) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.push_frame(&self.framebuffer)?)
    }

    /// 推帧（带自动恢复，最佳努力语义）。
    ///
    /// 首次推帧失败时复位 I2C 总线并重试一次；仍失败则跳过本帧
    /// （记录日志、不返回错误），等待后续帧周期自行恢复。
    /// 该策略经实测可靠：比进程崩溃或反复重试更稳。
    pub fn render_robust(&mut self) {
        if let Err(e) = self.render() {
            eprintln!("[显示] 帧推送失败: {e}");
            if self.recover().is_ok() {
                if let Err(e2) = self.render() {
                    eprintln!("[显示] 恢复后首帧仍失败: {e2}，跳过此帧等待总线稳定");
                }
            } else {
                eprintln!("[显示] I2C 总线重置失败，跳过此帧");
            }
        }
    }

    /// 设置对比度（0-255）。
    pub fn set_contrast(&mut self, val: u8) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.set_contrast(val)?)
    }

    /// 设置反色显示（true=反色 0xA7，false=正常 0xA6）。
    pub fn set_inverted(&mut self, inverted: bool) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.set_inverted(inverted)?)
    }

    /// 关闭 OLED 显示（进入休眠模式，0xAE）。GDDRAM 内容不受影响。
    pub fn sleep(&mut self) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.sleep()?)
    }

    /// 开启 OLED 显示（0xAF）。与 `sleep()` 相对。
    pub fn wake(&mut self) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.wake()?)
    }

    /// 重置 I2C 总线并重新初始化 SSD1309。
    ///
    /// 先关闭旧 I2C 连接（确保旧 fd 释放），再打开新连接并重新初始化
    /// OLED 控制器。用于 I2C 总线卡死（SDA stuck / lost arbitration）后的恢复。
    pub fn recover(&mut self) -> Result<(), DriverError> {
        eprintln!("[显示] 正在重置 I2C 总线...");
        self.driver = None;
        let bus = I2cBus::open(self.bus_id, self.addr)?;
        self.driver = Some(Ssd1309::init(bus)?);
        eprintln!("[显示] I2C 总线重置成功");
        Ok(())
    }
}
