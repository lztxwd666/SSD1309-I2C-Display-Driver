//! 显示硬件驱动层 —— SSD1309 OLED 通过 I2C 通信。
//!
//! Display 结构体将 I2C 总线、SSD1309 控制器和帧缓冲区组合为统一接口。

mod framebuffer;
mod i2c_bus;
mod ssd1309;

#[cfg(test)]
mod mock;

pub use framebuffer::{Framebuffer, HEIGHT, WIDTH};
pub use i2c_bus::{I2cBus, I2cDevice, I2cDeviceFactory};
pub use ssd1309::{ScrollDirection, ScrollFrameInterval};

use crate::DriverError;
use ssd1309::Ssd1309;

/// 显示初始化配置。
#[derive(Debug, Clone, Copy)]
pub struct DisplayConfig {
    /// I2C 总线编号。
    pub bus_id: u8,
    /// I2C 从机地址。
    pub addr: u8,
    /// 初始对比度（0-255），默认 0xCF。
    pub contrast: u8,
    /// 初始反色状态，默认 false。
    pub inverted: bool,
    /// 初始化完成后是否立即开启显示，默认 true。
    pub display_on: bool,
}

impl DisplayConfig {
    /// 使用默认参数创建配置。
    pub fn new(bus_id: u8, addr: u8) -> Self {
        Self {
            bus_id,
            addr,
            contrast: 0xCF,
            inverted: false,
            display_on: true,
        }
    }
}

/// 推帧结果（[`render_robust`](Display::render_robust) 返回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    /// 推帧成功。
    Ok,
    /// 首帧失败，经 I2C 总线恢复后重试成功。
    Recovered,
    /// 恢复后仍失败，本帧被跳过。
    Skipped,
}

/// 驱动运行统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DriverStats {
    /// 成功推送的帧数。
    pub frames_pushed: u64,
    /// 被跳过的帧数（恢复后仍失败）。
    pub frames_skipped: u64,
    /// 成功执行的 I2C 总线恢复次数。
    pub recoveries: u64,
    /// 累计错误次数。
    pub errors: u64,
}

/// OLED 显示器顶层句柄。
///
/// 泛型参数 `B` 为底层 I2C 设备，默认为 [`I2cBus`]（Linux `/dev/i2c-N`）。
/// 可通过 [`I2cDevice`] trait 注入自定义总线（测试用 Mock 总线等）。
///
/// 线程安全：`Display` 是 `Send`（可跨线程移动），但不是 `Sync`，
/// 多线程共享同一实例需要外部互斥（如 `Mutex<Display>`）。
pub struct Display<B: I2cDevice + I2cDeviceFactory = I2cBus> {
    /// Option 包装允许 `recover()` 先关闭旧连接再打开新连接，
    /// 避免两个 fd 同时指向 /dev/i2c-N 导致 RP1 控制器状态混乱。
    driver: Option<Ssd1309<B>>,
    pub framebuffer: Framebuffer,
    bus_id: u8,
    addr: u8,
    /// 当前对比度（recover 后恢复）。
    contrast: u8,
    /// 当前反色状态（recover 后恢复）。
    inverted: bool,
    /// 运行统计。
    stats: DriverStats,
    /// 日志回调；未设置时默认输出到 stderr。
    logger: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl Display<I2cBus> {
    /// 使用默认配置打开并初始化显示器。
    ///
    /// 等价于 `Display::open_config(DisplayConfig::new(bus_id, addr))`。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use i2c_display_driver::display::Display;
    ///
    /// let mut display = Display::open(1, 0x3C)?;
    /// display.framebuffer.set_pixel(10, 10, true);
    /// display.render()?;
    /// # Ok::<(), i2c_display_driver::DriverError>(())
    /// ```
    pub fn open(bus_id: u8, addr: u8) -> Result<Self, DriverError> {
        Self::open_config(DisplayConfig::new(bus_id, addr))
    }

    /// 按配置打开并初始化显示器（打开 /dev/i2c-N 并初始化控制器）。
    pub fn open_config(config: DisplayConfig) -> Result<Self, DriverError> {
        let bus = I2cBus::open(config.bus_id, config.addr)?;
        let driver = Ssd1309::init(bus, config.contrast, config.inverted, config.display_on)?;
        Ok(Self {
            driver: Some(driver),
            framebuffer: Framebuffer::new(),
            bus_id: config.bus_id,
            addr: config.addr,
            contrast: config.contrast,
            inverted: config.inverted,
            stats: DriverStats::default(),
            logger: None,
        })
    }
}

impl<B: I2cDevice + I2cDeviceFactory> Display<B> {
    /// 使用已打开的 I2C 设备构造显示器（自定义总线 / 测试用）。
    pub fn from_device(device: B, config: DisplayConfig) -> Result<Self, DriverError> {
        let driver = Ssd1309::init(device, config.contrast, config.inverted, config.display_on)?;
        Ok(Self {
            driver: Some(driver),
            framebuffer: Framebuffer::new(),
            bus_id: config.bus_id,
            addr: config.addr,
            contrast: config.contrast,
            inverted: config.inverted,
            stats: DriverStats::default(),
            logger: None,
        })
    }

    /// 设置日志回调。设置后驱动内部日志（恢复/错误信息）改由回调输出，
    /// 未设置时默认使用 `eprintln` 输出到 stderr。
    pub fn set_logger(&mut self, logger: impl Fn(&str) + Send + Sync + 'static) {
        self.logger = Some(Box::new(logger));
    }

    /// 获取驱动运行统计（帧数 / 错误数 / 恢复次数）。
    pub fn stats(&self) -> DriverStats {
        self.stats
    }

    /// 将帧缓冲区内容推送到 OLED。
    ///
    /// 当驱动未初始化（recover 失败后）时返回错误而非 panic。
    pub fn render(&mut self) -> Result<(), DriverError> {
        let result = match self.driver.as_mut() {
            Some(driver) => driver.push_frame(&self.framebuffer),
            None => return Err(DriverError::NotInitialized),
        };
        match result {
            Ok(()) => {
                self.stats.frames_pushed += 1;
                self.framebuffer.clear_dirty();
                Ok(())
            }
            Err(e) => {
                self.stats.errors += 1;
                Err(e.into())
            }
        }
    }

    /// 局部推帧：仅推送 (x, y, w, h) 区域（页寻址模式下只写覆盖的页面与列）。
    ///
    /// 配合 [`Framebuffer::dirty_rect`] 使用可只推送变化区域，节省 I2C 带宽。
    pub fn render_region(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> Result<(), DriverError> {
        let result = match self.driver.as_mut() {
            Some(driver) => driver.render_region(&self.framebuffer, x, y, w, h),
            None => return Err(DriverError::NotInitialized),
        };
        match result {
            Ok(()) => {
                self.stats.frames_pushed += 1;
                self.framebuffer.clear_dirty();
                Ok(())
            }
            Err(e) => {
                self.stats.errors += 1;
                Err(e.into())
            }
        }
    }

    /// 只推送帧缓冲区中自上次推帧以来被修改的区域。
    ///
    /// 无修改时为空操作。依赖 [`Framebuffer`] 的脏矩形跟踪。
    pub fn render_dirty(&mut self) -> Result<(), DriverError> {
        match self.framebuffer.dirty_rect() {
            Some((x, y, w, h)) => self.render_region(x, y, w, h),
            None => Ok(()),
        }
    }

    /// 推帧（带自动恢复，最佳努力语义）。
    ///
    /// 首次推帧失败时复位 I2C 总线并重试一次；仍失败则跳过本帧
    /// （记录日志、不返回错误），等待后续帧周期自行恢复。
    /// 该策略经实测可靠：比进程崩溃或反复重试更稳。
    ///
    /// 返回 [`RenderStatus`] 供调用方区分成功 / 恢复后成功 / 跳过本帧。
    pub fn render_robust(&mut self) -> RenderStatus {
        match self.render() {
            Ok(()) => RenderStatus::Ok,
            Err(e) => {
                self.log(&format!("帧推送失败: {e}"));
                match self.recover() {
                    Ok(()) => {
                        self.stats.recoveries += 1;
                        match self.render() {
                            Ok(()) => RenderStatus::Recovered,
                            Err(e2) => {
                                self.log(&format!("恢复后首帧仍失败: {e2}，跳过本帧"));
                                self.stats.frames_skipped += 1;
                                RenderStatus::Skipped
                            }
                        }
                    }
                    Err(e3) => {
                        self.log(&format!("I2C 总线重置失败: {e3}，跳过本帧"));
                        self.stats.frames_skipped += 1;
                        RenderStatus::Skipped
                    }
                }
            }
        }
    }

    /// 清空屏幕（清帧缓冲并推帧）。
    pub fn clear(&mut self) -> Result<(), DriverError> {
        self.framebuffer.clear();
        self.render()
    }

    /// 全屏点亮（填充帧缓冲并推帧）。
    pub fn fill(&mut self) -> Result<(), DriverError> {
        self.framebuffer.fill_all();
        self.render()
    }

    /// 设置对比度（0-255）。
    pub fn set_contrast(&mut self, val: u8) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        driver.set_contrast(val)?;
        self.contrast = val;
        Ok(())
    }

    /// 设置反色显示（true=反色 0xA7，false=正常 0xA6）。
    pub fn set_inverted(&mut self, inverted: bool) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        driver.set_inverted(inverted)?;
        self.inverted = inverted;
        Ok(())
    }

    /// 全屏点亮（0xA5）或恢复正常显示（0xA4）。
    ///
    /// 用于面板测试：全屏点亮可快速判断像素/驱动是否正常。
    pub fn set_entire_display_on(&mut self, on: bool) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.set_entire_display_on(on)?)
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

    /// 设置连续水平滚动。
    ///
    /// `start_page` 与 `end_page` 为参与滚动的页范围（0-7）。
    /// 设置后需调用 [`activate_scroll`](Self::activate_scroll) 生效。
    pub fn scroll_horizontal(
        &mut self,
        dir: ScrollDirection,
        start_page: u8,
        end_page: u8,
        interval: ScrollFrameInterval,
    ) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.scroll_horizontal(dir, start_page, end_page, interval)?)
    }

    /// 设置垂直 + 水平连续滚动。
    ///
    /// 需先通过 [`set_vertical_scroll_area`](Self::set_vertical_scroll_area) 设置滚动区域，
    /// 再调用 [`activate_scroll`](Self::activate_scroll) 生效。
    pub fn scroll_vertical_horizontal(
        &mut self,
        dir: ScrollDirection,
        start_page: u8,
        end_page: u8,
        interval: ScrollFrameInterval,
        vertical_offset: u8,
    ) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.scroll_vertical_horizontal(
            dir,
            start_page,
            end_page,
            interval,
            vertical_offset,
        )?)
    }

    /// 设置垂直滚动区域（0xA3）：顶部固定行数与底部固定行数。
    pub fn set_vertical_scroll_area(
        &mut self,
        top_fixed: u8,
        bottom_fixed: u8,
    ) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.set_vertical_scroll_area(top_fixed, bottom_fixed)?)
    }

    /// 激活滚动（0x2F）。
    pub fn activate_scroll(&mut self) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.activate_scroll()?)
    }

    /// 取消滚动（0x2E）。
    pub fn deactivate_scroll(&mut self) -> Result<(), DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.deactivate_scroll()?)
    }

    /// 重置 I2C 总线并重新初始化 SSD1309。
    ///
    /// 先关闭旧 I2C 连接（确保旧 fd 释放），再打开新连接并重新初始化
    /// OLED 控制器。用于 I2C 总线卡死（SDA stuck / lost arbitration）后的恢复。
    /// 恢复时沿用当前对比度与反色设置。
    pub fn recover(&mut self) -> Result<(), DriverError> {
        self.log("正在重置 I2C 总线...");
        self.driver = None;
        let bus = B::open(self.bus_id, self.addr)?;
        let driver = Ssd1309::init(bus, self.contrast, self.inverted, true)?;
        self.driver = Some(driver);
        self.log("I2C 总线重置成功");
        Ok(())
    }

    /// 内部日志入口：有回调走回调，否则输出到 stderr。
    fn log(&self, msg: &str) {
        match &self.logger {
            Some(f) => f(msg),
            None => eprintln!("[显示] {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::mock::{MockBus, Write};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    /// 构造带共享日志与失败计数器的 MockBus。
    fn new_bus() -> (Arc<Mutex<Vec<Write>>>, Rc<Cell<usize>>, MockBus) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let failures = Rc::new(Cell::new(0));
        let bus = MockBus::new(Arc::clone(&log), Rc::clone(&failures));
        (log, failures, bus)
    }

    #[test]
    fn render_robust_ok_when_bus_healthy() {
        let (log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        display.framebuffer.set_pixel(1, 1, true);
        assert_eq!(display.render_robust(), RenderStatus::Ok);
        let s = display.stats();
        assert_eq!(s.frames_pushed, 1);
        assert_eq!(s.errors, 0);
        assert_eq!(s.recoveries, 0);
        let _ = log;
    }

    #[test]
    fn render_robust_recovers_after_bus_failure() {
        let (log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        // 首帧所有写入失败 → 触发恢复；工厂创建的总线正常工作
        failures.set(usize::MAX);
        display.framebuffer.set_pixel(1, 1, true);
        assert_eq!(display.render_robust(), RenderStatus::Recovered);
        let s = display.stats();
        assert_eq!(s.frames_pushed, 1);
        assert_eq!(s.recoveries, 1);
        assert_eq!(s.errors, 1);
        assert_eq!(s.frames_skipped, 0);
        let _ = log;
    }

    #[test]
    fn render_robust_skips_when_recover_fails() {
        let (_log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        failures.set(usize::MAX);
        // 工厂创建的总线也失败（init 即失败 → recover 失败）
        MockBus::set_factory(Arc::new(Mutex::new(Vec::new())), usize::MAX);
        display.framebuffer.set_pixel(1, 1, true);
        assert_eq!(display.render_robust(), RenderStatus::Skipped);
        let s = display.stats();
        assert_eq!(s.frames_pushed, 0);
        assert_eq!(s.recoveries, 0);
        assert_eq!(s.frames_skipped, 1);
        assert!(s.errors >= 1);
    }

    #[test]
    fn recover_restores_contrast_and_inverted() {
        let (_log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        // 修改对比度与反色后触发恢复
        display.set_contrast(0x40).unwrap();
        display.set_inverted(true).unwrap();
        let factory_log = Arc::new(Mutex::new(Vec::new()));
        MockBus::set_factory(Arc::clone(&factory_log), 0);
        failures.set(usize::MAX);
        display.framebuffer.set_pixel(1, 1, true);
        assert_eq!(display.render_robust(), RenderStatus::Recovered);
        // 恢复时重新 init 应使用记忆的对比度与反色
        let w = factory_log.lock().unwrap();
        assert!(w.contains(&Write { control: 0x00, bytes: vec![0x81, 0x40] }));
        assert!(w.contains(&Write { control: 0x00, bytes: vec![0xA7] }));
    }

    #[test]
    fn clear_and_fill_push_full_frame() {
        let (log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        let init_count = log.lock().unwrap().len();

        display.clear().unwrap();
        {
            let guard = log.lock().unwrap();
            let w = &guard[init_count..];
            assert_eq!(w.len(), 24); // 8 页 × 3 写入
            for page in 0..8u8 {
                let p = page as usize * 3;
                assert_eq!(w[p], Write { control: 0x00, bytes: vec![0xB0 | page] });
                assert_eq!(w[p + 2].control, 0x40);
                assert!(w[p + 2].bytes.iter().all(|&b| b == 0));
            }
        }

        let init_count2 = log.lock().unwrap().len();
        display.fill().unwrap();
        {
            let guard = log.lock().unwrap();
            let w2 = &guard[init_count2..];
            for page in 0..8u8 {
                assert!(w2[page as usize * 3 + 2].bytes.iter().all(|&b| b == 0xFF));
            }
        }
        assert_eq!(display.stats().frames_pushed, 2);
    }

    #[test]
    fn render_dirty_pushes_only_changed_region() {
        let (log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        let init_count = log.lock().unwrap().len();

        // 无修改 → 空操作
        display.render_dirty().unwrap();
        assert_eq!(log.lock().unwrap().len(), init_count);

        // 修改一个像素 → 只推该像素所在页与列
        display.framebuffer.set_pixel(5, 5, true);
        display.render_dirty().unwrap();
        {
            let guard = log.lock().unwrap();
            let w = &guard[init_count..];
            assert_eq!(w.len(), 3);
            assert_eq!(w[0], Write { control: 0x00, bytes: vec![0xB0] }); // page0
            assert_eq!(w[1], Write { control: 0x00, bytes: vec![0x05, 0x10] }); // 列 5
            assert_eq!(w[2].control, 0x40);
            assert_eq!(w[2].bytes, vec![0x20]); // (5,5) → bit5
        }

        // 推送后脏矩形已清空 → 再次 render_dirty 为空操作
        display.render_dirty().unwrap();
        assert_eq!(log.lock().unwrap().len(), init_count + 3);
    }

    #[test]
    fn logger_callback_receives_error_messages() {
        let (_log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        let messages = Arc::new(Mutex::new(Vec::new()));
        {
            let messages = Arc::clone(&messages);
            display.set_logger(move |msg| messages.lock().unwrap().push(msg.to_string()));
        }
        failures.set(usize::MAX);
        display.framebuffer.set_pixel(1, 1, true);
        let _ = display.render_robust();
        assert!(!messages.lock().unwrap().is_empty());
        // 恢复成功后应记录成功日志
        assert!(messages.lock().unwrap().iter().any(|m| m.contains("重置成功")));
    }
}
