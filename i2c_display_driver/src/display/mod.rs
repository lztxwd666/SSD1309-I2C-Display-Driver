//! 显示硬件驱动层 —— SSD1309 OLED 通过 I2C 通信。
//!
//! Display 结构体将 I2C 总线、SSD1309 控制器和帧缓冲区组合为统一接口。

mod framebuffer;
mod i2c_bus;
mod ssd1309;

#[cfg(test)]
mod mock;

pub use framebuffer::{BlitMode, Framebuffer, HEIGHT, PageBuffer, WIDTH};
pub use i2c_bus::{I2cBus, I2cDevice, I2cDeviceFactory};
pub use ssd1309::Ssd1309;

use std::thread;
use std::time::{Duration, Instant};

use crate::DriverError;

/// 恢复冷却期：一次 recover 失败后，此期间内不再尝试恢复。
///
/// 防止总线永久故障时每帧触发一次 recover（关/开 fd + 全量初始化），
/// 造成系统调用与日志风暴。冷却期内的失败帧直接跳过，到期后重试。
const RECOVER_COOLDOWN: Duration = Duration::from_secs(2);

/// 水平滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    /// 向右滚动。
    Right,
    /// 向左滚动。
    Left,
}

/// 垂直滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalDirection {
    /// 内容向上滚动（新内容从底部进入）。
    Up,
    /// 内容向下滚动（新内容从顶部进入）。
    Down,
}

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

/// 驱动日志回调类型。
type LogCallback = Box<dyn Fn(&str) + Send + Sync>;

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
    logger: Option<LogCallback>,
    /// 上次 recover 失败的时刻；冷却期内 `render_robust` 不再尝试恢复。
    last_recover_failed: Option<Instant>,
    /// 滚动翻页动画状态（逐帧版使用）：动画源内容与总步数。
    scroll_source: Option<([u8; WIDTH * HEIGHT / 8], usize)>,
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
            last_recover_failed: None,
            scroll_source: None,
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
            last_recover_failed: None,
            scroll_source: None,
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
                // 脏矩形仅当本次推送完全覆盖时才清除：未覆盖部分保留，
                // 避免局部推帧后未推送区域的更新永久丢失。
                let covered = self
                    .framebuffer
                    .dirty_rect()
                    .is_some_and(|(dx, dy, dw, dh)| {
                        x <= dx
                            && y <= dy
                            && x.saturating_add(w) >= dx.saturating_add(dw)
                            && y.saturating_add(h) >= dy.saturating_add(dh)
                    });
                if covered {
                    self.framebuffer.clear_dirty();
                }
                Ok(())
            }
            Err(e) => {
                self.stats.errors += 1;
                Err(e.into())
            }
        }
    }

    /// 整页推帧：将 `page` 的内容完整推送到屏幕（瞬时切换，无动画）。
    ///
    /// 配合 [`PageBuffer`] 实现翻页显示：绘制各页 → 切换当前页 → 推帧。
    /// 需要动画过渡时改用 [`scroll_to_page`](Self::scroll_to_page)，
    /// 两种翻页形式由调用方自行选择。
    pub fn show_page(&mut self, page: &Framebuffer) -> Result<(), DriverError> {
        self.framebuffer.buffer.copy_from_slice(&page.buffer);
        self.render()
    }

    /// 滚动翻页动画：当前内容向左滚出，`page` 内容从右侧滚入。
    ///
    /// 动画共 `steps` 帧（每帧前进 `WIDTH/steps` 像素），`step_ms` 为每帧间隔；
    /// 结束时屏幕内容与 `page` 完全一致。I2C 总线 100kHz 下每帧推帧约 100ms，
    /// 建议 `steps=32`（动画约 3-4 秒）。
    ///
    /// 阻塞式：动画期间占用调用线程。需要在动画帧之间执行其他工作时，
    /// 改用逐帧驱动的 [`scroll_to_page_frame`](Self::scroll_to_page_frame)。
    /// 与瞬时切换的 [`show_page`](Self::show_page) 是两种可选的翻页形式，
    /// 按应用场景自行选择。
    pub fn scroll_to_page(
        &mut self,
        page: &Framebuffer,
        steps: usize,
        step_ms: u64,
    ) -> Result<(), DriverError> {
        let steps = steps.max(1);
        for k in 0..=steps {
            self.scroll_to_page_frame(page, steps, k)?;
            if k < steps {
                thread::sleep(Duration::from_millis(step_ms));
            }
        }
        Ok(())
    }

    /// 滚动翻页动画的逐帧驱动版：生成并推送第 `k` 帧（`k` 从 0 到 `steps`）。
    ///
    /// 首次调用（或 `steps` 变化）时捕获当前屏幕内容作为动画源，`k` 单调递增，
    /// `k == steps` 时动画完成并清除内部状态。调用方自行控制帧间节奏，
    /// 可在帧之间执行其他工作（如轮询输入）。
    ///
    /// 与阻塞式 [`scroll_to_page`](Self::scroll_to_page) 效果相同，
    /// 后者即按固定间隔循环调用本方法。
    pub fn scroll_to_page_frame(
        &mut self,
        page: &Framebuffer,
        steps: usize,
        k: usize,
    ) -> Result<(), DriverError> {
        let steps = steps.max(1);
        // 动画源在首次调用（或步数变化）时捕获；完成（k=steps）后清除
        if self
            .scroll_source
            .as_ref()
            .is_none_or(|(_, total)| *total != steps)
        {
            self.scroll_source = Some((self.framebuffer.buffer, steps));
        }
        let (current, _) = self.scroll_source.expect("滚动动画源已初始化");
        let k = k.min(steps);
        let offset = k * WIDTH / steps;
        blend_scroll_frame(&mut self.framebuffer.buffer, &current, &page.buffer, offset);
        self.render()?;
        if k >= steps {
            self.scroll_source = None;
        }
        Ok(())
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
    /// 恢复失败后进入冷却期（见 [`RECOVER_COOLDOWN`]），冷却期内
    /// 不再尝试恢复、直接跳过本帧，避免永久故障时每帧触发
    /// recover（关/开 fd + 全量初始化）造成系统调用与日志风暴。
    /// 该策略经实测可靠：比进程崩溃或反复重试更稳。
    ///
    /// 返回 [`RenderStatus`] 供调用方区分成功 / 恢复后成功 / 跳过本帧。
    pub fn render_robust(&mut self) -> RenderStatus {
        match self.render() {
            Ok(()) => RenderStatus::Ok,
            Err(e) => {
                self.log(&format!("帧推送失败: {e}"));
                if self.recover_in_cooldown() {
                    self.stats.frames_skipped += 1;
                    return RenderStatus::Skipped;
                }
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

    /// 是否处于恢复冷却期（上次 recover 失败后 [`RECOVER_COOLDOWN`] 内）。
    fn recover_in_cooldown(&self) -> bool {
        self.last_recover_failed
            .is_some_and(|t| t.elapsed() < RECOVER_COOLDOWN)
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

    /// 读取 SSD1309 状态寄存器原始值（bit7=忙，bit0=电荷泵使能）。
    ///
    /// 用于故障诊断：读失败说明总线异常；读成功但 busy 持续置位
    /// 说明屏幕内部忙（控制器无响应），二者恢复策略不同。
    /// 解析辅助见 [`status_busy`](Self::status_busy) 与
    /// [`status_booster`](Self::status_booster)。
    pub fn read_status(&mut self) -> Result<u8, DriverError> {
        let driver = self.driver.as_mut().ok_or(DriverError::NotInitialized)?;
        Ok(driver.read_status()?)
    }

    /// 解析状态字节：忙标志（bit7）。
    pub fn status_busy(status: u8) -> bool {
        status & 0x80 != 0
    }

    /// 解析状态字节：电荷泵（DC-DC 升压器）使能标志（bit0）。
    pub fn status_booster(status: u8) -> bool {
        status & 0x01 != 0
    }

    /// 水平滚动：将帧缓冲内容循环平移 `offset` 像素并推帧。
    ///
    /// 通过帧缓冲平移实现（实测本项目屏幕不响应硬件滚动命令 0x26/0x2F，
    /// 因此采用软件方案），定时调用本方法即可实现跑马灯/新闻滚动效果。
    /// 平移是循环的：移出屏幕一侧的内容从另一侧进入。
    pub fn software_scroll_horizontal(
        &mut self,
        dir: ScrollDirection,
        offset: usize,
    ) -> Result<(), DriverError> {
        scroll_horizontal_bits(&mut self.framebuffer.buffer, dir, offset);
        self.render()
    }

    /// 垂直滚动：将帧缓冲内容循环平移 `offset` 行并推帧。
    ///
    /// 与水平滚动同为软件方案（帧缓冲位平移 + 推帧），每列 64 行像素
    /// 组装为 64 位整数循环移位，跨页边界（第 8/16/…行）无缝处理。
    /// 平移是循环的：移出屏幕一侧的内容从另一侧进入。
    pub fn software_scroll_vertical(
        &mut self,
        dir: VerticalDirection,
        offset: usize,
    ) -> Result<(), DriverError> {
        scroll_vertical_bits(&mut self.framebuffer.buffer, dir, offset);
        self.render()
    }

    /// 重置 I2C 总线并重新初始化 SSD1309。
    ///
    /// 先关闭旧 I2C 连接（确保旧 fd 释放），再打开新连接并重新初始化
    /// OLED 控制器。用于 I2C 总线卡死（SDA stuck / lost arbitration）后的恢复。
    /// 恢复时沿用当前对比度与反色设置。
    ///
    /// 失败时记录恢复冷却（`render_robust` 在冷却期内不再自动重试）；
    /// 成功时清除冷却。
    pub fn recover(&mut self) -> Result<(), DriverError> {
        self.log("正在重置 I2C 总线...");
        self.driver = None;
        let result: Result<Ssd1309<B>, DriverError> = B::open(self.bus_id, self.addr)
            .map_err(Into::into)
            .and_then(|bus| {
                Ssd1309::init(bus, self.contrast, self.inverted, true).map_err(Into::into)
            });
        match result {
            Ok(driver) => {
                self.driver = Some(driver);
                self.last_recover_failed = None;
                self.log("I2C 总线重置成功");
                Ok(())
            }
            Err(e) => {
                self.last_recover_failed = Some(Instant::now());
                Err(e)
            }
        }
    }

    /// 内部日志入口：有回调走回调，否则输出到 stderr。
    fn log(&self, msg: &str) {
        match &self.logger {
            Some(f) => f(msg),
            None => eprintln!("[显示] {msg}"),
        }
    }
}

/// 生成滚动翻页动画帧：左段为 `src` 左移 `offset` 列，右段为 `new` 的进入部分。
///
/// `offset=0` 时结果与 `src` 一致，`offset=WIDTH` 时结果与 `new` 一致，
/// 中间值形成"旧内容滚出、新内容滚入"的过渡帧。
fn blend_scroll_frame(
    dst: &mut [u8; WIDTH * HEIGHT / 8],
    src: &[u8; WIDTH * HEIGHT / 8],
    new: &[u8; WIDTH * HEIGHT / 8],
    offset: usize,
) {
    let offset = offset.min(WIDTH);
    for page_idx in 0..(HEIGHT / 8) {
        for col in 0..WIDTH {
            let src_col = col + offset;
            dst[page_idx * WIDTH + col] = if src_col < WIDTH {
                src[page_idx * WIDTH + src_col]
            } else {
                new[page_idx * WIDTH + src_col - WIDTH]
            };
        }
    }
}

/// 将帧缓冲内容水平循环平移 `offset` 像素。
///
/// 帧缓冲布局为 `buffer[page * WIDTH + col]`：每字节是一列 8 个垂直像素，
/// 因此水平平移等于每页整行字节的循环旋转（无需位操作）。
fn scroll_horizontal_bits(
    buffer: &mut [u8; WIDTH * HEIGHT / 8],
    dir: ScrollDirection,
    offset: usize,
) {
    let offset = offset % WIDTH;
    if offset == 0 {
        return;
    }
    for page in 0..(HEIGHT / 8) {
        let row = &mut buffer[page * WIDTH..(page + 1) * WIDTH];
        match dir {
            ScrollDirection::Right => row.rotate_right(offset),
            ScrollDirection::Left => row.rotate_left(offset),
        }
    }
}

/// 将帧缓冲内容垂直循环平移 `offset` 行。
///
/// 每列 64 行像素跨 8 个页字节（`buffer[page * WIDTH + col]` 的 bit0..7
/// 对应行 `page*8..page*8+7`），组装为 u64 循环移位后拆回，
/// 跨页边界（第 8/16/…行）天然无缝。
fn scroll_vertical_bits(
    buffer: &mut [u8; WIDTH * HEIGHT / 8],
    dir: VerticalDirection,
    offset: usize,
) {
    let offset = offset % HEIGHT;
    if offset == 0 {
        return;
    }
    for col in 0..WIDTH {
        // 组装 64 位列：bit k = 行 k 的像素（列 col）
        let mut v: u64 = 0;
        for page in 0..(HEIGHT / 8) {
            v |= (buffer[page * WIDTH + col] as u64) << (page * 8);
        }
        // 循环平移：行 i 显示原行 i+offset（上移）→ 位号减小 = rotate_right；
        // 下移 → 位号增大 = rotate_left。
        let v = match dir {
            VerticalDirection::Up => v.rotate_right(offset as u32),
            VerticalDirection::Down => v.rotate_left(offset as u32),
        };
        // 拆回 8 页
        for page in 0..(HEIGHT / 8) {
            buffer[page * WIDTH + col] = (v >> (page * 8)) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{MockBus, Write};
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    /// 共享句柄：写入日志 + 失败计数器 + 总线。
    type SharedBus = (Arc<Mutex<Vec<Write>>>, Rc<Cell<usize>>, MockBus);

    /// 构造带共享日志与失败计数器的 MockBus。
    fn new_bus() -> SharedBus {
        let log = Arc::new(Mutex::new(Vec::new()));
        let failures = Rc::new(Cell::new(0));
        let bus = MockBus::new(
            Arc::clone(&log),
            Rc::clone(&failures),
            Rc::new(Cell::new(0x01)),
        );
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
        assert!(w.contains(&Write {
            control: 0x00,
            bytes: vec![0x81, 0x40]
        }));
        assert!(w.contains(&Write {
            control: 0x00,
            bytes: vec![0xA7]
        }));
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
                assert_eq!(
                    w[p],
                    Write {
                        control: 0x00,
                        bytes: vec![0xB0 | page]
                    }
                );
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
            assert_eq!(
                w[0],
                Write {
                    control: 0x00,
                    bytes: vec![0xB0]
                }
            ); // page0
            assert_eq!(
                w[1],
                Write {
                    control: 0x00,
                    bytes: vec![0x05, 0x10]
                }
            ); // 列 5
            assert_eq!(w[2].control, 0x40);
            assert_eq!(w[2].bytes, vec![0x20]); // (5,5) → bit5
        }

        // 推送后脏矩形已清空 → 再次 render_dirty 为空操作
        display.render_dirty().unwrap();
        assert_eq!(log.lock().unwrap().len(), init_count + 3);
    }

    /// 模拟 feature_check 步骤 3：两行文字经 render_dirty 局部推送。
    /// 验证两次推送的页面、列地址与数据内容（'P' 字形第一列 = 0x7F）。
    #[test]
    fn two_line_render_dirty_matches_feature_check() {
        let (log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        let init_count = log.lock().unwrap().len();

        // 第一行：y=0 只覆盖 page0
        crate::graphics::text::draw_text(&mut display.framebuffer, 0, 0, "PARTIAL UPDATE");
        display.render_dirty().unwrap();
        {
            let guard = log.lock().unwrap();
            let w = &guard[init_count..];
            assert_eq!(w.len(), 3, "第一行应只推 1 页（3 次写入）");
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
                    bytes: vec![0x00, 0x10]
                }
            );
            assert_eq!(w[2].control, 0x40);
            assert_eq!(w[2].bytes.len(), 83); // 14 字符 × 5px + 13 gap（末字符后无 gap）
            assert_eq!(w[2].bytes[0], 0x7F, "'P' 字形第一列应为 0x7F");
            assert_eq!(display.framebuffer.dirty_rect(), None);
        }

        // 第二行：y=12 覆盖 page1 + page2
        crate::graphics::text::draw_text(&mut display.framebuffer, 0, 12, "dirty rect OK");
        let before2 = log.lock().unwrap().len();
        display.render_dirty().unwrap();
        {
            let guard = log.lock().unwrap();
            let w = &guard[before2..];
            assert_eq!(w.len(), 6, "第二行应推 2 页（6 次写入）");
            assert_eq!(
                w[0],
                Write {
                    control: 0x00,
                    bytes: vec![0xB1]
                }
            );
            assert_eq!(
                w[1],
                Write {
                    control: 0x00,
                    bytes: vec![0x00, 0x10]
                }
            );
            assert_eq!(w[2].bytes.len(), 77); // 13 字符 × 5px + 12 gap
            assert_eq!(
                w[3],
                Write {
                    control: 0x00,
                    bytes: vec![0xB2]
                }
            );
            assert_eq!(w[5].bytes.len(), 77);
            assert_eq!(display.framebuffer.dirty_rect(), None);
        }
    }

    #[test]
    fn software_scroll_shifts_and_wraps() {
        let (_log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 右移 1 像素：列 5 → 列 6
        display.framebuffer.set_pixel(5, 5, true);
        display
            .software_scroll_horizontal(ScrollDirection::Right, 1)
            .unwrap();
        assert!(display.framebuffer.get_pixel(6, 5));
        assert!(!display.framebuffer.get_pixel(5, 5));

        // 循环：右边界像素回到左边界
        display.framebuffer.clear();
        display.framebuffer.set_pixel(WIDTH - 1, 5, true);
        display
            .software_scroll_horizontal(ScrollDirection::Right, 1)
            .unwrap();
        assert!(display.framebuffer.get_pixel(0, 5));

        // 跨字节偏移：右移 9 像素
        display.framebuffer.clear();
        display.framebuffer.set_pixel(0, 5, true);
        display
            .software_scroll_horizontal(ScrollDirection::Right, 9)
            .unwrap();
        assert!(display.framebuffer.get_pixel(9, 5));

        // 整宽平移 = 无变化
        display.framebuffer.clear();
        display.framebuffer.set_pixel(10, 10, true);
        display
            .software_scroll_horizontal(ScrollDirection::Right, WIDTH)
            .unwrap();
        assert!(display.framebuffer.get_pixel(10, 10));

        // 左移对称：列 100 → 列 93
        display.framebuffer.clear();
        display.framebuffer.set_pixel(100, 20, true);
        display
            .software_scroll_horizontal(ScrollDirection::Left, 7)
            .unwrap();
        assert!(display.framebuffer.get_pixel(93, 20));
        assert!(!display.framebuffer.get_pixel(100, 20));
    }

    #[test]
    fn scroll_to_page_animates_and_ends_on_new_page() {
        let (_log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 当前内容：像素 (5,5)；目标页：像素 (100,30)
        display.framebuffer.set_pixel(5, 5, true);
        let mut target = Framebuffer::new();
        target.set_pixel(100, 30, true);

        // 动画结束：屏幕内容与目标页完全一致（旧像素消失）
        display.scroll_to_page(&target, 32, 0).unwrap();
        assert!(display.framebuffer.get_pixel(100, 30));
        assert!(!display.framebuffer.get_pixel(5, 5));

        // 幂等：再滚一次结果相同
        display.scroll_to_page(&target, 16, 0).unwrap();
        assert!(display.framebuffer.get_pixel(100, 30));
    }

    #[test]
    fn blend_frame_midway_splits_src_and_new() {
        // 源全黑、目标全白：offset=64 时左 64 列应为黑、右 64 列应为白
        let src = [0u8; WIDTH * HEIGHT / 8];
        let new = [0xFFu8; WIDTH * HEIGHT / 8];
        let mut dst = [0u8; WIDTH * HEIGHT / 8];

        blend_scroll_frame(&mut dst, &src, &new, WIDTH / 2);
        for page_idx in 0..(HEIGHT / 8) {
            for col in 0..WIDTH / 2 {
                assert_eq!(dst[page_idx * WIDTH + col], 0x00, "左段应为源内容");
            }
            for col in WIDTH / 2..WIDTH {
                assert_eq!(dst[page_idx * WIDTH + col], 0xFF, "右段应为新页内容");
            }
        }

        // offset=0：与源一致
        blend_scroll_frame(&mut dst, &src, &new, 0);
        assert!(dst.iter().all(|&b| b == 0x00));

        // offset=WIDTH：与目标一致
        blend_scroll_frame(&mut dst, &src, &new, WIDTH);
        assert!(dst.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn scroll_to_page_frame_drives_animation_stepwise() {
        let (_log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 源：像素 (5,5)；目标：像素 (100,30)
        display.framebuffer.set_pixel(5, 5, true);
        let mut target = Framebuffer::new();
        target.set_pixel(100, 30, true);

        // k=0：帧内容 = 源（旧像素在，新像素未进入）
        display.scroll_to_page_frame(&target, 4, 0).unwrap();
        assert!(display.framebuffer.get_pixel(5, 5));
        assert!(!display.framebuffer.get_pixel(100, 30));

        // k=1：中间帧（偏移 32 列）：旧像素已滚出左侧，新像素尚未进入
        display.scroll_to_page_frame(&target, 4, 1).unwrap();
        assert!(
            !display.framebuffer.get_pixel(5, 5),
            "旧像素 (5,5) 应已滚出"
        );
        assert!(
            !display.framebuffer.get_pixel(100, 30),
            "新像素 (100,30) 尚未进入"
        );

        // k=4：动画完成，内容 = 目标
        display.scroll_to_page_frame(&target, 4, 4).unwrap();
        assert!(display.framebuffer.get_pixel(100, 30));
        assert!(!display.framebuffer.get_pixel(5, 5));

        // 动画完成后状态已清除：再次 k=0 应以新源开始（当前内容 = 目标）
        display.scroll_to_page_frame(&target, 4, 0).unwrap();
        assert!(
            display.framebuffer.get_pixel(100, 30),
            "新动画源应为当前内容"
        );
    }

    #[test]
    fn show_page_pushes_page_content() {
        let (log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 构造页面内容并推帧
        let mut pages = PageBuffer::<2>::new();
        pages.page().set_pixel(10, 10, true); // page0：像素 (10,10)
        display.show_page(pages.page_at(0).unwrap()).unwrap();
        // 推帧后 Display 帧缓冲与页面内容一致
        assert!(display.framebuffer.get_pixel(10, 10));

        // 切换页后推帧，内容随之切换
        pages.next_page();
        pages.page().set_pixel(20, 20, true); // page1：像素 (20,20)
        display.show_page(pages.page_at(1).unwrap()).unwrap();
        assert!(display.framebuffer.get_pixel(20, 20));
        assert!(!display.framebuffer.get_pixel(10, 10));

        // 再切回 page0：内容保留
        display.show_page(pages.page_at(0).unwrap()).unwrap();
        assert!(display.framebuffer.get_pixel(10, 10));
        assert!(!display.framebuffer.get_pixel(20, 20));
        let _ = log;
    }

    #[test]
    fn software_scroll_vertical_shifts_and_wraps() {
        let (_log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 上移 1 行：行 5 → 行 4
        display.framebuffer.set_pixel(5, 5, true);
        display
            .software_scroll_vertical(VerticalDirection::Up, 1)
            .unwrap();
        assert!(display.framebuffer.get_pixel(5, 4));
        assert!(!display.framebuffer.get_pixel(5, 5));

        // 循环：顶部行移到底部
        display.framebuffer.clear();
        display.framebuffer.set_pixel(5, 0, true);
        display
            .software_scroll_vertical(VerticalDirection::Up, 1)
            .unwrap();
        assert!(display.framebuffer.get_pixel(5, HEIGHT - 1));

        // 跨页边界：行 8（page1 bit0）上移 1 → 行 7（page0 bit7）
        display.framebuffer.clear();
        display.framebuffer.set_pixel(5, 8, true);
        display
            .software_scroll_vertical(VerticalDirection::Up, 1)
            .unwrap();
        assert!(display.framebuffer.get_pixel(5, 7));
        assert!(!display.framebuffer.get_pixel(5, 8));

        // 下移对称：行 5 → 行 6
        display.framebuffer.clear();
        display.framebuffer.set_pixel(5, 5, true);
        display
            .software_scroll_vertical(VerticalDirection::Down, 1)
            .unwrap();
        assert!(display.framebuffer.get_pixel(5, 6));

        // 整高平移 = 无变化
        display.framebuffer.clear();
        display.framebuffer.set_pixel(5, 10, true);
        display
            .software_scroll_vertical(VerticalDirection::Up, HEIGHT)
            .unwrap();
        assert!(display.framebuffer.get_pixel(5, 10));

        // 多行偏移：行 10 上移 3 → 行 7（跨页：page1 bit2 → page0 bit7）
        display.framebuffer.clear();
        display.framebuffer.set_pixel(5, 10, true);
        display
            .software_scroll_vertical(VerticalDirection::Up, 3)
            .unwrap();
        assert!(display.framebuffer.get_pixel(5, 7));
    }

    #[test]
    fn render_region_keeps_uncovered_dirty_area() {
        let (log, _failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 修改两个远离的区域：左上角 (5,5) 和右下角 (100,60)
        display.framebuffer.set_pixel(5, 5, true);
        display.framebuffer.set_pixel(100, 60, true);
        assert_eq!(display.framebuffer.dirty_rect(), Some((5, 5, 96, 56)));

        // 只推送左上角区域 → 右下角更新必须保留在脏矩形中
        display.render_region(0, 0, 20, 20).unwrap();
        assert_eq!(display.framebuffer.dirty_rect(), Some((5, 5, 96, 56)));

        // 推送覆盖整个脏矩形的区域 → 才清除
        display.render_region(0, 0, 128, 64).unwrap();
        assert_eq!(display.framebuffer.dirty_rect(), None);
        let _ = log;
    }

    /// 长稳压力测试（默认忽略，用 `cargo test -- --ignored` 运行）：
    /// 循环渲染/滚动/翻页 5000 帧，每 100 帧注入一次总线写入失败，
    /// 验证恢复链路（render_robust → recover）在长时间运行下的正确性。
    #[test]
    #[ignore = "长稳测试：cargo test -- --ignored 运行"]
    fn long_run_stress_with_fault_injection() {
        let (log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();
        // 工厂共享同一失败句柄：recover 后的新总线仍受外部注入控制
        MockBus::set_factory_cell(Arc::clone(&log), Rc::clone(&failures));

        let mut pages = PageBuffer::<2>::new();
        pages.page_at_mut(0).unwrap().set_pixel(1, 1, true);
        pages.page_at_mut(1).unwrap().set_pixel(127, 63, true);

        for i in 0..5000usize {
            // 每 100 帧注入 1 次写入失败，模拟 I2C 偶发故障
            if i % 100 == 0 {
                failures.set(1);
                // 注入后工厂仍指向同一句柄，保证 recover 创建健康总线
                MockBus::set_factory_cell(Arc::clone(&log), Rc::clone(&failures));
            }
            // 帧内容随迭代变化
            let px = i % 128;
            let py = i % 64;
            display.framebuffer.set_pixel(px, py, true);
            let _ = display.render_robust();
            // 滚动与翻页路径周期性覆盖
            if i % 50 == 0 {
                let _ = display.software_scroll_horizontal(ScrollDirection::Left, 1);
            }
            if i % 200 == 0 {
                display
                    .show_page(pages.page_at(i / 200 % 2).unwrap())
                    .unwrap();
            }
        }

        let s = display.stats();
        println!(
            "长稳结果：推帧 {}，错误 {}，跳过 {}，恢复 {}",
            s.frames_pushed, s.errors, s.frames_skipped, s.recoveries
        );
        // 5000 帧中注入 50 次故障，每次恢复后重试成功
        assert!(s.frames_pushed >= 4950, "推帧数 {} 过少", s.frames_pushed);
        assert!(s.errors >= 40, "错误注入应被记录，实际 {}", s.errors);
        assert!(s.recoveries >= 40, "应完成恢复，实际 {}", s.recoveries);
        assert!(s.frames_skipped < 50, "跳过帧 {} 过多", s.frames_skipped);
    }

    #[test]
    fn read_status_returns_bus_status() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let status = Rc::new(Cell::new(0x81)); // busy + booster
        let bus = MockBus::new(Arc::clone(&log), Rc::new(Cell::new(0)), Rc::clone(&status));
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        assert_eq!(display.read_status().unwrap(), 0x81);
        // 状态解析
        assert!(Display::<MockBus>::status_busy(0x81));
        assert!(Display::<MockBus>::status_booster(0x81));
        assert!(!Display::<MockBus>::status_busy(0x01));
        assert!(!Display::<MockBus>::status_booster(0x80));

        // 状态在运行中可变（模拟真实忙状态变化）
        status.set(0x00);
        assert_eq!(display.read_status().unwrap(), 0x00);
        assert!(!Display::<MockBus>::status_busy(0x00));
        let _ = log;
    }

    #[test]
    fn render_robust_respects_recover_cooldown() {
        let (log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 首帧失败且恢复也失败 → 进入冷却期
        failures.set(usize::MAX);
        MockBus::set_factory(Arc::new(Mutex::new(Vec::new())), usize::MAX);
        display.framebuffer.set_pixel(1, 1, true);
        assert_eq!(display.render_robust(), RenderStatus::Skipped);
        assert_eq!(display.stats().recoveries, 0);

        // 冷却期内再次失败：不再尝试 recover（recoveries 保持 0）
        let _ = display.render_robust();
        assert_eq!(display.stats().recoveries, 0, "冷却期内不应再次尝试恢复");
        assert_eq!(display.stats().frames_skipped, 2);
        let _ = log;
    }

    #[test]
    fn recover_success_clears_cooldown() {
        let (log, failures, bus) = new_bus();
        let mut display =
            Display::<MockBus>::from_device(bus, DisplayConfig::new(1, 0x3C)).unwrap();

        // 首次恢复失败 → 进入冷却
        failures.set(usize::MAX);
        MockBus::set_factory(Arc::new(Mutex::new(Vec::new())), usize::MAX);
        display.framebuffer.set_pixel(1, 1, true);
        assert_eq!(display.render_robust(), RenderStatus::Skipped);

        // 显式 recover 成功（工厂总线恢复正常）→ 冷却清除
        MockBus::set_factory(Arc::new(Mutex::new(Vec::new())), 0);
        display.recover().unwrap();
        assert!(!display.recover_in_cooldown(), "成功后冷却应被清除");
        let _ = log;
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
        assert!(
            messages
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains("重置成功"))
        );
    }
}
