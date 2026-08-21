//! 长稳压力测试（真机观察）：长时间运行下的稳定性验证。
//!
//! 运行：cargo run --example stress [轮数]
//!
//! 循环覆盖驱动全部核心路径：绘制→推帧→滚动→翻页→对比度变化→状态读取。
//! 每 100 轮打印一次统计（帧数/错误/跳过/恢复），观察长时间运行下
//! I2C 偶发故障的恢复表现与内存稳定性。默认 3000 轮（约 5-10 分钟），
//! Ctrl+C 可随时中断（中断前的统计已输出）。
//!
//! 自动化版本见 `cargo test -- --ignored`（MockBus 故障注入验证恢复链路）。

use std::thread;
use std::time::Duration;

use i2c_display_driver::DriverError;
use i2c_display_driver::display::{Display, I2cBus, PageBuffer, ScrollDirection};
use i2c_display_driver::graphics::{canvas, text};

/// 单轮循环中的帧间间隔（毫秒）。
const FRAME_MS: u64 = 50;

fn main() -> Result<(), DriverError> {
    // 轮数可配置：默认 3000，命令行参数覆盖
    let rounds: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);

    let mut display = Display::open(1, 0x3C)?;
    display.set_logger(|msg| println!("  [驱动日志] {msg}"));

    // 两页内容（翻页路径）
    let mut pages = PageBuffer::<2>::new();
    pages.page_at_mut(0).unwrap().set_pixel(1, 1, true);
    pages.page_at_mut(1).unwrap().set_pixel(127, 63, true);

    println!("[压力测试] 开始 {} 轮循环（每轮约 {}ms）", rounds, FRAME_MS);

    for i in 0..rounds {
        // 帧内容随迭代变化（覆盖全屏区域）
        display.framebuffer.clear();
        let px = i * 3 % 128;
        let py = i % 64;
        text::draw_text(&mut display.framebuffer, 0, 0, "STRESS");
        canvas::fill_rect(&mut display.framebuffer, px, 20, 8, 8);
        text::draw_text(&mut display.framebuffer, 0, 40, &format!("{}", i));
        display.framebuffer.set_pixel(px, py, true);
        let _ = display.render_robust();

        // 周期性覆盖各功能路径
        if i % 50 == 0 {
            let _ = display.software_scroll_horizontal(ScrollDirection::Left, 1);
        }
        if i % 100 == 0 {
            let page = pages.page_at(i / 100 % 2).unwrap();
            let _ = display.show_page(page);
        }
        if i % 200 == 0 {
            let contrast = [0x80, 0xA0, 0xCF][i / 200 % 3];
            let _ = display.set_contrast(contrast);
        }
        // 状态读取（诊断路径）
        if i % 500 == 0 {
            match display.read_status() {
                Ok(st) => {
                    let busy = Display::<I2cBus>::status_busy(st);
                    let booster = Display::<I2cBus>::status_booster(st);
                    println!("  状态: 0x{st:02X}（忙={busy}，电荷泵={booster}）");
                }
                Err(e) => println!("  状态读取失败: {e}"),
            }
        }
        // 定期统计
        if i % 100 == 0 && i > 0 {
            let s = display.stats();
            println!(
                "  第 {} 轮：推帧 {}，错误 {}，跳过 {}，恢复 {}",
                i, s.frames_pushed, s.errors, s.frames_skipped, s.recoveries
            );
        }
        thread::sleep(Duration::from_millis(FRAME_MS));
    }

    let s = display.stats();
    println!("\n[压力测试] 完成 {} 轮", rounds);
    println!(
        "  总计：推帧 {}，错误 {}，跳过 {}，恢复 {}",
        s.frames_pushed, s.errors, s.frames_skipped, s.recoveries
    );
    println!("  评估：错误>0 但恢复成功属正常（I2C 偶发故障被恢复）；错误持续增长需检查硬件");
    Ok(())
}
