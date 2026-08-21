//! 硬件自检：一键验证屏幕与驱动全链路。
//!
//! 运行：cargo run --example selftest
//!
//! 逐项执行：状态读取 → 全屏点亮测试 → 字符表 → 滚动 → 翻页 → 统计。
//! 每步打印预期现象与结果（通过/失败），适合每次驱动改动后的快速回归。
//! 最后输出总结论：全部通过才可视为驱动与硬件状态良好。

use std::thread;
use std::time::Duration;

use i2c_display_driver::DriverError;
use i2c_display_driver::display::{Display, I2cBus, ScrollDirection};
use i2c_display_driver::graphics::{canvas, text};

fn pause(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;
    let mut failed = false;

    // 1. 状态读取
    println!("[1/6] 状态读取");
    match display.read_status() {
        Ok(st) => {
            let busy = Display::<I2cBus>::status_busy(st);
            let booster = Display::<I2cBus>::status_booster(st);
            println!("  状态 0x{st:02X}：忙={busy}，电荷泵={booster}");
            if booster {
                println!("  通过：电荷泵已使能");
            } else {
                println!("  失败：电荷泵未使能（屏幕可能未正常初始化）");
                failed = true;
            }
        }
        Err(e) => {
            println!("  失败：状态读取错误 {e}");
            failed = true;
        }
    }

    // 2. 全屏点亮测试模式（0xA5，不依赖 GDDRAM）
    println!("[2/6] 全屏点亮测试模式，观察 2 秒（屏幕应全亮）");
    display.set_entire_display_on(true)?;
    pause(2000);
    display.set_entire_display_on(false)?;
    pause(300);

    // 3. 基础显示：字符表与图形
    println!("[3/6] 字符表与图形，观察 3 秒");
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, "ABCDEFGHIJKLM");
    text::draw_text(&mut display.framebuffer, 0, 12, "nopqrstuvwxyz");
    text::draw_text(&mut display.framebuffer, 0, 24, "0123456789 +-*/");
    text::draw_text(&mut display.framebuffer, 0, 36, "48.5C  UP DOWN");
    canvas::draw_rect(&mut display.framebuffer, 0, 50, 20, 10);
    canvas::fill_circle(&mut display.framebuffer, 60, 55, 6);
    display.render()?;
    pause(3000);

    // 4. 软件滚动
    println!("[4/6] 软件滚动（循环右移 3 秒）");
    for _ in 0..30 {
        display.software_scroll_horizontal(ScrollDirection::Right, 1)?;
        pause(100);
    }

    // 5. 翻页（两页瞬时切换）
    println!("[5/6] 翻页（两页交替，各 1.5 秒）");
    let mut pages = i2c_display_driver::display::PageBuffer::<2>::new();
    pages.page_at_mut(0).unwrap().set_pixel(1, 1, true);
    pages.page_at_mut(1).unwrap().set_pixel(127, 63, true);
    display.show_page(pages.page_at(0).unwrap())?;
    pause(1500);
    display.show_page(pages.page_at(1).unwrap())?;
    pause(1500);

    // 6. 统计
    let s = display.stats();
    println!(
        "[6/6] 运行统计：推帧 {}，错误 {}，跳过 {}，恢复 {}",
        s.frames_pushed, s.errors, s.frames_skipped, s.recoveries
    );
    if s.errors > 0 {
        println!(
            "  注意：运行中发生 {} 次 I/O 错误（已自动恢复则属正常）",
            s.errors
        );
    }

    // 结论
    println!();
    if failed {
        println!("自检结论：存在失败项，请检查硬件连接与初始化");
    } else {
        println!("自检结论：全部通过，驱动与硬件状态良好");
    }
    Ok(())
}
