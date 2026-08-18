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

/// 水平滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    /// 向右滚动。
    Right,
    /// 向左滚动。
    Left,
}

/// 滚动帧间隔（每移动 1 像素的帧数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollFrameInterval {
    Frames2 = 0x07,
    Frames3 = 0x04,
    Frames4 = 0x05,
    Frames5 = 0x00,
    Frames25 = 0x06,
    Frames64 = 0x01,
    Frames128 = 0x02,
    Frames256 = 0x03,
}

/// SSD1309 控制器。泛型参数 `B` 为底层 I2C 设备。
pub struct Ssd1309<B: I2cDevice> {
    bus: B,
}

impl<B: I2cDevice> Ssd1309<B> {
    /// 初始化控制器。`contrast` 初始对比度、`inverted` 初始反色、
    /// `display_on` 初始化完成后是否立即开启显示。
    pub fn init(mut bus: B, contrast: u8, inverted: bool, display_on: bool) -> io::Result<Self> {
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
        bus.write_command(&[0x81, contrast])?; // 对比度
        bus.write_command(&[0xD9, 0xF1])?; // 预充电周期 1/F1
        bus.write_command(&[0xDB, 0x40])?; // VCOMH 取消选择级别
        bus.write_command(&[0xA4])?; // 正常显示模式（非全亮）
        bus.write_command(&[if inverted { 0xA7 } else { 0xA6 }])?; // 反色/正常
        if display_on {
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
        let y1 = (y + h).min(HEIGHT);
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

    /// 设置连续水平滚动（0x26/0x27）。
    ///
    /// `start_page` 与 `end_page` 为参与滚动的页范围（0-7）。
    /// 设置后需调用 [`activate_scroll`](Self::activate_scroll) 生效。
    pub fn scroll_horizontal(
        &mut self,
        dir: ScrollDirection,
        start_page: u8,
        end_page: u8,
        interval: ScrollFrameInterval,
    ) -> io::Result<()> {
        validate_pages(start_page, end_page)?;
        let cmd = match dir {
            ScrollDirection::Right => 0x26,
            ScrollDirection::Left => 0x27,
        };
        self.bus
            .write_command(&[cmd, 0x00, start_page, interval as u8, end_page, 0x00, 0xFF])
    }

    /// 设置垂直 + 水平连续滚动（0x29/0x2A）。
    ///
    /// `vertical_offset`（0-63）为垂直滚动步长。
    /// 需先通过 [`set_vertical_scroll_area`](Self::set_vertical_scroll_area) 设置滚动区域，
    /// 再调用 [`activate_scroll`](Self::activate_scroll) 生效。
    pub fn scroll_vertical_horizontal(
        &mut self,
        dir: ScrollDirection,
        start_page: u8,
        end_page: u8,
        interval: ScrollFrameInterval,
        vertical_offset: u8,
    ) -> io::Result<()> {
        validate_pages(start_page, end_page)?;
        if vertical_offset > 63 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("垂直滚动偏移超出范围: {vertical_offset} > 63"),
            ));
        }
        let cmd = match dir {
            ScrollDirection::Right => 0x29,
            ScrollDirection::Left => 0x2A,
        };
        self.bus.write_command(&[
            cmd,
            0x00,
            start_page,
            interval as u8,
            end_page,
            vertical_offset,
            0x00,
            0xFF,
        ])
    }

    /// 设置垂直滚动区域（0xA3）：顶部固定行数与底部固定行数。
    pub fn set_vertical_scroll_area(&mut self, top_fixed: u8, bottom_fixed: u8) -> io::Result<()> {
        self.bus.write_command(&[0xA3, top_fixed, bottom_fixed])
    }

    /// 激活滚动（0x2F）。
    pub fn activate_scroll(&mut self) -> io::Result<()> {
        self.bus.write_command(&[0x2F])
    }

    /// 取消滚动（0x2E）。
    pub fn deactivate_scroll(&mut self) -> io::Result<()> {
        self.bus.write_command(&[0x2E])
    }
}

/// 校验滚动页范围（0-7 且 start <= end）。
fn validate_pages(start_page: u8, end_page: u8) -> io::Result<()> {
    if start_page > 7 || end_page > 7 || start_page > end_page {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("滚动页范围无效: {start_page}..{end_page}（应为 0-7）"),
        ));
    }
    Ok(())
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
        let bus = MockBus::new(Arc::clone(&log), Rc::new(Cell::new(0)));
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
    fn scroll_commands_write_expected_bytes() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        let init_count = log.lock().unwrap().len();

        ssd.scroll_horizontal(ScrollDirection::Right, 0, 3, ScrollFrameInterval::Frames5)
            .unwrap();
        ssd.scroll_vertical_horizontal(
            ScrollDirection::Left,
            1,
            5,
            ScrollFrameInterval::Frames64,
            10,
        )
        .unwrap();
        ssd.set_vertical_scroll_area(0, 64).unwrap();
        ssd.activate_scroll().unwrap();
        ssd.deactivate_scroll().unwrap();

        let w = writes_after_init(&log, init_count);
        assert_eq!(
            w[0],
            Write {
                control: 0x00,
                bytes: vec![0x26, 0x00, 0x00, 0x00, 0x03, 0x00, 0xFF]
            }
        );
        assert_eq!(
            w[1],
            Write {
                control: 0x00,
                bytes: vec![0x2A, 0x00, 0x01, 0x01, 0x05, 0x0A, 0x00, 0xFF]
            }
        );
        assert_eq!(
            w[2],
            Write {
                control: 0x00,
                bytes: vec![0xA3, 0x00, 0x40]
            }
        );
        assert_eq!(
            w[3],
            Write {
                control: 0x00,
                bytes: vec![0x2F]
            }
        );
        assert_eq!(
            w[4],
            Write {
                control: 0x00,
                bytes: vec![0x2E]
            }
        );
    }

    #[test]
    fn scroll_rejects_invalid_pages() {
        let (log, bus) = mock_bus();
        let mut ssd = Ssd1309::init(bus, 0xCF, false, true).unwrap();
        assert!(
            ssd.scroll_horizontal(ScrollDirection::Right, 8, 3, ScrollFrameInterval::Frames5)
                .is_err()
        );
        assert!(
            ssd.scroll_horizontal(ScrollDirection::Right, 0, 9, ScrollFrameInterval::Frames5)
                .is_err()
        );
        assert!(
            ssd.scroll_horizontal(ScrollDirection::Right, 3, 2, ScrollFrameInterval::Frames5)
                .is_err()
        );
        assert!(
            ssd.scroll_vertical_horizontal(
                ScrollDirection::Right,
                0,
                1,
                ScrollFrameInterval::Frames5,
                64
            )
            .is_err()
        );
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
}
