//! 长文本跑马灯演示：模拟真实新闻/状态滚动条。
//!
//! 运行：cargo run --example scroll_demo
//!
//! 布局模拟真实应用：标题 + 滚动条（长文本连续左移）+ 静止数据行。
//! 滚动文本用模式重复拼接填满整行，保证循环滚动时无空白跳变（无缝跑马灯）。
//!
//! 帧率说明：软件滚动每帧必须全推 1024 字节（内容整体移动，无法局部推送）。
//! 本项目按 I2C 总线 100kHz 使用（总线上还挂载其他设备，默认不建议提速），
//! 全帧传输约 100ms，帧率上限约 10 帧/秒。
//! 因此 sleep 仅设 20ms 作轻微节流，实际帧率由 I2C 传输决定。

use std::thread;
use std::time::{Duration, Instant};

use i2c_display_driver::DriverError;
use i2c_display_driver::display::{Display, ScrollDirection, VerticalDirection};
use i2c_display_driver::graphics::{canvas, text};

/// 滚动步进间隔（毫秒/像素）——轻微节流，实际帧率由 I2C 传输决定。
const STEP_MS: u64 = 20;
/// 演示滚动帧数。
const SCROLL_FRAMES: usize = 100;

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;

    // 绘制静态布局：标题 + 滚动条区域 + 两行静止数据
    display.framebuffer.clear();

    // 标题（反色条 + 反色文字）
    canvas::fill_rect(&mut display.framebuffer, 0, 0, 128, 9);
    text::draw_text_inverted(&mut display.framebuffer, 2, 1, "SYS MONITOR");

    // 滚动条：模式文本重复拼接填满整行（约 400px，远超 128px 屏宽），
    // 循环滚动时任何位置都有连续文本，无空白跳变
    let ticker = "CPU 45C  MEM 62%  DISK 80%  NET 1.2MB/s  |  ";
    let ticker_full: String = ticker.repeat(4);
    text::draw_text(&mut display.framebuffer, 0, 12, &ticker_full);

    // 静止数据行
    text::draw_text(&mut display.framebuffer, 0, 28, "CPU: 45C");
    text::draw_text(&mut display.framebuffer, 0, 40, "MEM: 62%");

    display.render()?;
    println!("[跑马灯] 布局已绘制，滚动条开始连续左移");

    // 滚动循环：每帧平移 1 像素并推帧
    let start = Instant::now();
    for _ in 0..SCROLL_FRAMES {
        display.software_scroll_horizontal(ScrollDirection::Left, 1)?;
        thread::sleep(Duration::from_millis(STEP_MS));
    }

    // 统计输出：评估滚动策略（帧率 / 稳定性 / 错误）
    let elapsed = start.elapsed();
    let s = display.stats();
    println!(
        "\n[跑马灯] {} 帧滚动完成，耗时 {:?}",
        SCROLL_FRAMES, elapsed
    );
    println!(
        "         推帧数: {}，错误: {}，跳过帧: {}，恢复次数: {}",
        s.frames_pushed, s.errors, s.frames_skipped, s.recoveries
    );
    println!(
        "         平均帧率: {:.1} 帧/秒（= 滚动速度 {:.1} px/s）",
        SCROLL_FRAMES as f64 / elapsed.as_secs_f64(),
        SCROLL_FRAMES as f64 / elapsed.as_secs_f64()
    );
    println!(
        "         说明：帧率受 I2C 总线限制（全帧 1024 字节），I2C 提速到 400kHz 后可提升约 3-4 倍"
    );

    // 垂直滚动演示：整体内容循环上移（模拟整屏翻页滚动）
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 12, "VERTICAL UP");
    text::draw_text(&mut display.framebuffer, 0, 28, "content line 2");
    text::draw_text(&mut display.framebuffer, 0, 44, "content line 3");
    display.render()?;
    println!("\n[垂直滚动] 内容循环上移 4 秒");
    for _ in 0..50 {
        display.software_scroll_vertical(VerticalDirection::Up, 1)?;
        thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}
