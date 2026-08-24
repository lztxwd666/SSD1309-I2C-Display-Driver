//! SSD1309 OLED 控制器驱动。
//!
//! 页寻址模式逐页推送帧数据，避免 Pi 5 RP1 I2C 控制器的大块传输限制。
//!
//! 控制器泛型于 [`I2cDevice`]，真机使用 [`I2cBus`]，测试可注入 Mock 总线
//! 验证命令序列。

use std::io;
use std::thread;
use std::time::Duration;

use super::framebuffer::{Framebuffer, HEIGHT, WIDTH};
use super::i2c_bus::I2cDevice;
use super::{DisplayConfig, DisplayRotation, ScrollDirection};

/// SSD1309 控制器。泛型参数 `B` 为底层 I2C 设备。
pub struct Ssd1309<B: I2cDevice> {
    bus: B,
}

impl<B: I2cDevice> Ssd1309<B> {
    /// 初始化控制器。`contrast` 初始对比度、`inverted` 初始反色、
    /// `display_on` 初始化完成后是否立即开启显示。
    ///
    /// 使用默认旋转和默认高级寄存器参数，等价于
    /// `init_with_config` 配合 `DisplayConfig::new(0, 0)` 的默认值。
    pub fn init(bus: B, contrast: u8, inverted: bool, display_on: bool) -> io::Result<Self> {
        let mut config = DisplayConfig::new(0, 0);
        config.contrast = contrast;
        config.inverted = inverted;
        config.display_on = display_on;
        Self::init_with_config(bus, &config)
    }

    /// 按完整配置初始化控制器。
    pub(crate) fn init_with_config(mut bus: B, config: &DisplayConfig) -> io::Result<Self> {
        bus.write_command(&[0xAE])?; // 关闭显示（休眠模式）
        let clock = ((config.clock_frequency & 0x0F) << 4) | (config.clock_divide_ratio & 0x0F);
        bus.write_command(&[0xD5, clock])?; // 时钟分频/振荡器频率
        bus.write_command(&[0xA8, config.multiplex_ratio])?; // 多路复用比
        bus.write_command(&[0xD3, config.display_offset])?; // 显示偏移
        bus.write_command(&[0x40 | (config.start_line & 0x3F)])?; // 起始行地址
        bus.write_command(&[
            0x8D,
            if config.charge_pump_enabled {
                0x14
            } else {
                0x10
            },
        ])?; // 电荷泵
        bus.write_command(&[0xAD, 0x8A])?; // SSD1309 DC-DC 转换器（缺少会导致花屏）
        thread::sleep(Duration::from_millis(100));
        bus.write_command(&[0x20, 0x02])?; // 页寻址模式（避免 Pi 5 RP1 大块传输限制）
        match config.rotation {
            DisplayRotation::Rotate0 => {
                bus.write_command(&[0xA1])?; // 段重映射（水平翻转）
                bus.write_command(&[0xC8])?; // COM 扫描方向（垂直翻转）
            }
            DisplayRotation::Rotate180 => {
                bus.write_command(&[0xA0])?; // 段重映射关闭
                bus.write_command(&[0xC0])?; // COM 扫描方向正常
            }
        }
        bus.write_command(&[0xDA, config.com_pins_config])?; // COM 引脚硬件配置
        bus.write_command(&[0x81, config.contrast])?; // 对比度
        let precharge = ((config.precharge_phase2 & 0x0F) << 4) | (config.precharge_phase1 & 0x0F);
        bus.write_command(&[0xD9, precharge])?; // 预充电周期
        bus.write_command(&[0xDB, config.vcomh_level])?; // VCOMH 取消选择级别
        bus.write_command(&[0xA4])?; // 正常显示模式（非全亮）
        bus.write_command(&[if config.inverted { 0xA7 } else { 0xA6 }])?; // 反色/正常
        if config.display_on {
            bus.write_command(&[0xAF])?; // 开启显示
        }
        Ok(Self { bus })
    }

    /// 逐页推送 1024 字节帧数据（8 页 × 128 字节）。
    pub fn push_frame(&mut self, fb: &Framebuffer) -> io::Result<()> {
        let data = fb.as_bytes();
        for page in 0..(HEIGHT / 8) as u8 {
            self.bus.write_command(&[0xB0 | page])?;
            self.bus.write_command(&[0x00, 0x10])?;
            self.bus
                .write_data(&data[page as usize * WIDTH..][..WIDTH])?;
        }
        Ok(())
    }

    /// 局部推帧：仅推送 (x, y, w, h) 覆盖的页面与列。
    ///
    /// 页寻址模式下设置页地址 + 列地址后写入区域宽度字节。
    /// 越界区域自动裁剪；区域为空时直接返回。
    pub fn render_region(
        &mut self,
        fb: &Framebuffer,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> io::Result<()> {
        let x = x.min(WIDTH);
        let w = w.min(WIDTH - x);
        if w == 0 {
            return Ok(());
        }
        let y0 = y.min(HEIGHT);
        let y1 = y.saturating_add(h).min(HEIGHT);
        if y0 >= y1 {
            return Ok(());
        }
        let page0 = y0 / 8;
        let page1 = (y1 - 1) / 8;
        let data = fb.as_bytes();
        for page in page0..=page1 {
            self.bus.write_command(&[0xB0 | page as u8])?;
            let col_low = x as u8 & 0x0F;
            let col_high = 0x10 | ((x as u8 >> 4) & 0x07);
            self.bus.write_command(&[col_low, col_high])?;
            self.bus.write_data(&data[page * WIDTH + x..][..w])?;
        }
        Ok(())
    }

    /// 设置对比度（0-255）。SSD1309 默认 0xCF。
    pub fn set_contrast(&mut self, val: u8) -> io::Result<()> {
        self.bus.write_command(&[0x81, val])
    }

    /// 设置反色显示（0xA7）或正常显示（0xA6）。
    pub fn set_inverted(&mut self, inverted: bool) -> io::Result<()> {
        self.bus
            .write_command(&[if inverted { 0xA7 } else { 0xA6 }])
    }

    /// 全屏点亮（0xA5）或恢复正常显示（0xA4，恢复显示 GDDRAM 内容）。
    ///
    /// 用于面板测试：全屏点亮可快速判断像素/驱动是否正常。
    pub fn set_entire_display_on(&mut self, on: bool) -> io::Result<()> {
        self.bus.write_command(&[if on { 0xA5 } else { 0xA4 }])
    }

    /// 进入休眠模式（关闭显示），GDDRAM 内容不受影响。
    pub fn sleep(&mut self) -> io::Result<()> {
        self.bus.write_command(&[0xAE])
    }

    /// 开启显示（0xAF）。与 `sleep()` 相对。
    pub fn wake(&mut self) -> io::Result<()> {
        self.bus.write_command(&[0xAF])
    }

    /// 设置旋转方向。
    ///
    /// 仅发送段重映射与 COM 扫描方向命令，不清空帧缓冲。
    pub fn set_rotation(&mut self, rotation: DisplayRotation) -> io::Result<()> {
        match rotation {
            DisplayRotation::Rotate0 => {
                self.bus.write_command(&[0xA1])?;
                self.bus.write_command(&[0xC8])?;
            }
            DisplayRotation::Rotate180 => {
                self.bus.write_command(&[0xA0])?;
                self.bus.write_command(&[0xC0])?;
            }
        }
        Ok(())
    }

    /// 设置显示偏移（0-63）。
    pub fn set_display_offset(&mut self, offset: u8) -> io::Result<()> {
        self.bus.write_command(&[0xD3, offset & 0x3F])
    }

    /// 设置起始行地址（0-63）。
    pub fn set_start_line(&mut self, line: u8) -> io::Result<()> {
        self.bus.write_command(&[0x40 | (line & 0x3F)])
    }

    /// 设置多路复用比（行数-1，128×64 面板为 0x3F）。
    pub fn set_multiplex_ratio(&mut self, ratio: u8) -> io::Result<()> {
        self.bus.write_command(&[0xA8, ratio])
    }

    /// 设置时钟分频比与振荡器频率。
    pub fn set_clock(&mut self, divide_ratio: u8, frequency: u8) -> io::Result<()> {
        let value = ((frequency & 0x0F) << 4) | (divide_ratio & 0x0F);
        self.bus.write_command(&[0xD5, value])
    }

    /// 设置预充电周期。
    pub fn set_precharge_period(&mut self, phase1: u8, phase2: u8) -> io::Result<()> {
        let value = ((phase2 & 0x0F) << 4) | (phase1 & 0x0F);
        self.bus.write_command(&[0xD9, value])
    }

    /// 设置 VCOMH 取消选择级别。
    pub fn set_vcomh_level(&mut self, level: u8) -> io::Result<()> {
        self.bus.write_command(&[0xDB, level])
    }

    /// 设置 COM 引脚硬件配置。
    pub fn set_com_pins_config(&mut self, config: u8) -> io::Result<()> {
        self.bus.write_command(&[0xDA, config])
    }

    /// 设置电荷泵使能状态。
    pub fn set_charge_pump(&mut self, enabled: bool) -> io::Result<()> {
        self.bus
            .write_command(&[0x8D, if enabled { 0x14 } else { 0x10 }])
    }

    /// 配置并激活水平硬件滚动。
    ///
    /// 注意：当前项目实测屏幕不响应硬件滚动命令；此方法仅为需要验证
    /// 其他 SSD1309 面板或进行硬件兼容性测试时保留。
    pub fn hardware_scroll_horizontal(
        &mut self,
        dir: ScrollDirection,
        start_page: u8,
        end_page: u8,
        speed: u8,
    ) -> io::Result<()> {
        let cmd = match dir {
            ScrollDirection::Right => 0x26,
            ScrollDirection::Left => 0x27,
        };
        self.bus.write_command(&[
            cmd,
            0x00,
            start_page & 0x07,
            speed,
            end_page & 0x07,
            0x00,
            0xFF,
        ])?;
        self.bus.write_command(&[0x2F])
    }

    /// 停止硬件滚动。
    pub fn deactivate_scroll(&mut self) -> io::Result<()> {
        self.bus.write_command(&[0x2E])
    }

    /// 读取状态寄存器（bit7=忙，bit0=电荷泵使能）。
    ///
    /// 用于故障诊断：区分总线故障与屏幕未响应（busy 长时间置位）。
    pub fn read_status(&mut self) -> io::Result<u8> {
        self.bus.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::mock::{MockBus, Write};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    /// 构造带独立日志的 MockBus。
    fn mock_bus() -> (Arc<Mutex<Vec<Write>>>, MockBus) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let bus = MockBus::new(
            Arc::clone(&log),
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0x01)),
        );
        (log, bus)
    }

    /// 取第 n 次写入（跳过初始化产生的写入）。
    fn writes_after_init(log: &Arc<Mutex<Vec<Write>>>, init_count: usize) -> Vec<Write> {
        log.lock().unwrap()[init_count..].to_vec()
    }

    #[test]
    fn init_writes_full_sequence() {
        let (log, bus) = mock_bus();
        Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let w = log.lock().unwrap();
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0xAE]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0xD5, 0x80]
            }
        );
        assert_eq!(
            w[2],
            Write {
                control: 0x00,
                bytes: vec![0xA8, 0x3F]
            }
        );
        assert_eq!(
            w[3],
            Write {
                control: 0x00,
                bytes: vec![0xD3, 0x00]
            }
        );
        assert_eq!(
            w[4],
            Write {
                control: 0x00,
                bytes: vec![0x40]
            }
        );
        assert_eq!(
            w[5],
            Write {
                control: 0x00,
                bytes: vec![0x8D, 0x14]
            }
        );
        assert_eq!(
            w[6],
            Write {
                control: 0x00,
                bytes: vec![0xAD, 0x8A]
            }
        ); // SSD1309 DC-DC
        assert_eq!(
            w[7],
            Write {
                control: 0x00,
                bytes: vec![0x20, 0x02]
            }
        ); // 页寻址
        assert_eq!(
            w[8],
            Write {
                control: 0x00,
                bytes: vec![0xA1]
            }
        );
        assert_eq!(
            w[9],
            Write {
                control: 0x00,
                bytes: vec![0xC8]
            }
        );
        assert_eq!(
            w[10],
            Write {
                control: 0x00,
                bytes: vec![0xDA, 0x12]
            }
        );
        assert_eq!(
            w[11],
            Write {
                control: 0x00,
                bytes: vec![0x81, 0xCF]
            }
        ); // 对比度
        assert_eq!(
            w[12],
            Write {
                control: 0x00,
                bytes: vec![0xD9, 0xF1]
            }
        );
        assert_eq!(
            w[13],
            Write {
                control: 0x00,
                bytes: vec![0xDB, 0x40]
            }
        );
        assert_eq!(
            w[14],
            Write {
                control: 0x00,
                bytes: vec![0xA4]
            }
        );
        assert_eq!(
            w[15],
            Write {
                control: 0x00,
                bytes: vec![0xA6]
            }
        ); // 非反色
        assert_eq!(
            w[16],
            Write {
                control: 0x00,
                bytes: vec![0xAF]
            }
        ); // 开启显示
        assert_eq!(w.len(), 17);
    }

    #[test]
    fn init_honors_custom_config() {
        // 自定义对比度 + 反色 + 不开启显示
        let (log, bus) = mock_bus();
        Ssd1309::init(bus, 0x40, true, false).unwrap();
        let w = log.lock().unwrap();
        assert!(w.contains(&Write {
            control: 0x00,
            bytes: vec![0x81, 0x40]
        }));
        assert!(w.contains(&Write {
            control: 0x00,
            bytes: vec![0xA7]
        }));
        // 最后一条不应是 0xAF（display_on=false）
        assert_ne!(
            w.last().unwrap(),
            &Write {
                control: 0x00,
                bytes: vec![0xAF]
            }
        );
    }

    #[test]
    fn push_frame_writes_8_pages() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        let mut fb = Framebuffer::new();
        fb.set_pixel(0, 0, true); // page0 第一字节 bit0
        fb.set_pixel(127, 63, true); // page7 最后一字节 bit7
        ssd.push_frame(&fb).unwrap();

        let w = writes_after_init(&log, init_count);
        // 8 页 × (页命令 + 列命令 + 数据)
        assert_eq!(w.len(), 24);
        for page in 0..8u8 {
            let p = page as usize * 3;
            assert_eq!(
                w[p],
                Write {
                    control: 0x00,
                    bytes: vec![0xB0 | page]
                }
            );
            assert_eq!(
                w[p + 1],
                Write {
                    control: 0x00,
                    bytes: vec![0x00, 0x10]
                }
            );
            assert_eq!(w[p + 2].control, 0x40);
            assert_eq!(w[p + 2].bytes.len(), WIDTH);
        }
        // 校验数据内容（w[2] 为 page0 的数据写入：页命令 + 列命令 + 数据）
        assert_eq!(w[2].bytes[0], 0x01); // (0,0) bit0
        assert_eq!(w[7 * 3 + 2].bytes[127], 0x80); // (127,63) bit7
    }

    #[test]
    fn render_region_writes_only_covered_pages() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        let mut fb = Framebuffer::new();
        // 区域 (10, 5, 20, 10) 覆盖 page0(行5-7) 与 page1(行8-14)
        fb.set_pixel(15, 10, true);
        ssd.render_region(&fb, 10, 5, 20, 10).unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(w.len(), 6); // 2 页 × 3 写入
        // page0：列地址 10（低 0x0A，高 0x10）
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0xB0]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0x0A, 0x10]
            }
        );
        assert_eq!(w[2].control, 0x40);
        assert_eq!(w[2].bytes.len(), 20);
        // page1
        assert_eq!(
            w[3],
            Write {
                control: 0x00,
                bytes: vec![0xB1]
            }
        );
        assert_eq!(
            w[4],
            Write {
                control: 0x00,
                bytes: vec![0x0A, 0x10]
            }
        );
        assert_eq!(w[5].control, 0x40);
        assert_eq!(w[5].bytes.len(), 20);
    }

    #[test]
    fn render_region_clips_and_noops() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        let fb = Framebuffer::new();
        // 完全越界 → 空操作
        ssd.render_region(&fb, 200, 200, 10, 10).unwrap();
        // 零宽 → 空操作
        ssd.render_region(&fb, 0, 0, 0, 10).unwrap();
        assert_eq!(log.lock().unwrap().len(), init_count);
    }

    #[test]
    fn render_region_extreme_height_no_panic() {
        // 回归测试：极端 h 不应导致加法溢出（saturating 裁剪后正常推送）
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();
        let fb = Framebuffer::new();

        // h=usize::MAX 裁剪为 (0,0,10,64)：覆盖全部 8 页
        ssd.render_region(&fb, 0, 0, 10, usize::MAX).unwrap();
        // 裁剪为 (0,60,10,4)：只覆盖 page7
        ssd.render_region(&fb, 0, 60, 10, usize::MAX).unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(w.len(), (8 + 1) * 3, "8 页 + 1 页，每页 3 次写入");
        assert_eq!(w[2].bytes.len(), 10);
        assert_eq!(w[8 * 3].bytes[0], 0xB7, "第二区域页地址应为 page7");
    }

    #[test]
    fn read_status_returns_bus_value() {
        let (log, bus) = mock_bus(); // 默认状态 0x01（电荷泵使能）
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        assert_eq!(ssd.read_status().unwrap(), 0x01);
        let _ = log;
    }

    #[test]
    fn control_commands_write_expected_bytes() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        ssd.set_contrast(0x40).unwrap();
        ssd.set_inverted(true).unwrap();
        ssd.set_inverted(false).unwrap();
        ssd.set_entire_display_on(true).unwrap();
        ssd.set_entire_display_on(false).unwrap();
        ssd.sleep().unwrap();
        ssd.wake().unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0x81, 0x40]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0xA7]
            }
        );
        assert_eq!(
            w[2],
            Write {
                control: 0x00,
                bytes: vec![0xA6]
            }
        );
        assert_eq!(
            w[3],
            Write {
                control: 0x00,
                bytes: vec![0xA5]
            }
        );
        assert_eq!(
            w[4],
            Write {
                control: 0x00,
                bytes: vec![0xA4]
            }
        );
        assert_eq!(
            w[5],
            Write {
                control: 0x00,
                bytes: vec![0xAE]
            }
        );
        assert_eq!(
            w[6],
            Write {
                control: 0x00,
                bytes: vec![0xAF]
            }
        );
    }

    #[test]
    fn init_with_config_rotation_180_writes_flip_commands() {
        let (log, bus) = mock_bus();
        let mut config = DisplayConfig::new(0, 0);
        config.rotation = DisplayRotation::Rotate180;
        Ssd1309::init_with_config(bus, &config).unwrap();
        let w = log.lock().unwrap();
        assert!(w.contains(&Write {
            control: 0x00,
            bytes: vec![0xA0]
        }));
        assert!(w.contains(&Write {
            control: 0x00,
            bytes: vec![0xC0]
        }));
        assert!(!w.contains(&Write {
            control: 0x00,
            bytes: vec![0xA1]
        }));
    }

    #[test]
    fn set_rotation_writes_expected_commands() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        ssd.set_rotation(DisplayRotation::Rotate180).unwrap();
        ssd.set_rotation(DisplayRotation::Rotate0).unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0xA0]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0xC0]
            }
        );
        assert_eq!(
            w[2],
            Write {
                control: 0x00,
                bytes: vec![0xA1]
            }
        );
        assert_eq!(
            w[3],
            Write {
                control: 0x00,
                bytes: vec![0xC8]
            }
        );
    }

    #[test]
    fn hardware_scroll_writes_expected_commands() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        ssd.hardware_scroll_horizontal(ScrollDirection::Right, 0, 7, 0x00)
            .unwrap();
        ssd.deactivate_scroll().unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0x26, 0x00, 0x00, 0x00, 0x07, 0x00, 0xFF]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0x2F]
            }
        );
        assert_eq!(
            w[2],
            Write {
                control: 0x00,
                bytes: vec![0x2E]
            }
        );
    }

    #[test]
    fn advanced_setters_write_expected_commands() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        ssd.set_display_offset(0x10).unwrap();
        ssd.set_start_line(0x20).unwrap();
        ssd.set_multiplex_ratio(0x2F).unwrap();
        ssd.set_clock(0x01, 0x07).unwrap();
        ssd.set_precharge_period(0x02, 0x0E).unwrap();
        ssd.set_vcomh_level(0x30).unwrap();
        ssd.set_com_pins_config(0x02).unwrap();
        ssd.set_charge_pump(false).unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0xD3, 0x10]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0x40 | 0x20]
            }
        );
        assert_eq!(
            w[2],
            Write {
                control: 0x00,
                bytes: vec![0xA8, 0x2F]
            }
        );
        assert_eq!(
            w[3],
            Write {
                control: 0x00,
                bytes: vec![0xD5, 0x71]
            }
        );
        assert_eq!(
            w[4],
            Write {
                control: 0x00,
                bytes: vec![0xD9, 0xE2]
            }
        );
        assert_eq!(
            w[5],
            Write {
                control: 0x00,
                bytes: vec![0xDB, 0x30]
            }
        );
        assert_eq!(
            w[6],
            Write {
                control: 0x00,
                bytes: vec![0xDA, 0x02]
            }
        );
        assert_eq!(
            w[7],
            Write {
                control: 0x00,
                bytes: vec![0x8D, 0x10]
            }
        );
    }
}
