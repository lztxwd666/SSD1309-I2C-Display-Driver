//! 多页仪表演示：模拟真实设备的翻页显示。
//!
//! 运行：cargo run --example page_demo
//!
//! 场景：3 页仪表（系统状态 / 传感器 / 网络），每页含标题栏、数据行
//! 与页码指示；每 5 秒自动翻页循环（真实设备常配合按键手动翻页，
//! 此处用定时器模拟）。页面内容在切换间保留，翻页只推帧切换。

use std::thread;
use std::time::Duration;

use i2c_display_driver::DriverError;
use i2c_display_driver::display::{Display, PageBuffer};
use i2c_display_driver::graphics::{canvas, text};

/// 页面总数。
const PAGES: usize = 3;
/// 每页停留时长（毫秒）。
const PAGE_MS: u64 = 5000;

fn main() -> Result<(), DriverError> {
    let mut display = Display::open(1, 0x3C)?;
    let mut pages = PageBuffer::<PAGES>::new();

    // 逐页绘制（用 page_at_mut 显式指定页，避免误用当前页）

    // 第 1 页：系统状态
    {
        let p = pages.page_at_mut(0).unwrap();
        p.clear();
        canvas::fill_rect(p, 0, 0, 128, 9); // 反色标题条
        text::draw_text_inverted(p, 2, 1, "SYSTEM");
        text::draw_text(p, 0, 16, "CPU: 45C");
        text::draw_text(p, 0, 28, "MEM: 62%");
        text::draw_text(p, 0, 40, "DISK: 80%");
        text::draw_text(p, 100, 55, "1/3");
    }

    // 第 2 页：传感器
    {
        let p = pages.page_at_mut(1).unwrap();
        p.clear();
        canvas::fill_rect(p, 0, 0, 128, 9);
        text::draw_text_inverted(p, 2, 1, "SENSORS");
        text::draw_text(p, 0, 16, "TEMP: 25.6C");
        text::draw_text(p, 0, 28, "HUMI: 58%");
        text::draw_text(p, 0, 40, "PRES: 101.3kPa");
        text::draw_text(p, 100, 55, "2/3");
    }

    // 第 3 页：网络
    {
        let p = pages.page_at_mut(2).unwrap();
        p.clear();
        canvas::fill_rect(p, 0, 0, 128, 9);
        text::draw_text_inverted(p, 2, 1, "NETWORK");
        text::draw_text(p, 0, 16, "IP: 192.168.1.1");
        text::draw_text(p, 0, 28, "RX: 1.2MB/s");
        text::draw_text(p, 0, 40, "TX: 0.3MB/s");
        text::draw_text(p, 100, 55, "3/3");
    }

    // 自动翻页：演示两种翻页形式（驱动提供两种 API，开发者任选其一）
    display.show_page(pages.page_at(0).unwrap())?;
    println!(
        "[翻页演示] {} 页仪表，每 {} 秒翻页，共 2 轮：第 1 轮直接切换，第 2 轮滚动动画",
        PAGES,
        PAGE_MS / 1000
    );

    // 第 1 轮：直接切换（show_page，瞬时无动画）
    for _ in 0..PAGES {
        thread::sleep(Duration::from_millis(PAGE_MS));
        let idx = pages.next_page();
        display.show_page(pages.page_at(idx).unwrap())?;
        println!("  [直接切换] 第 {}/{} 页", idx + 1, PAGES);
    }

    // 第 2 轮：滚动动画（scroll_to_page，新页从右侧滚入，约 3.5 秒）
    for _ in 0..PAGES {
        thread::sleep(Duration::from_millis(PAGE_MS));
        let idx = pages.next_page();
        display.scroll_to_page(pages.page_at(idx).unwrap(), 32, 10)?;
        println!("  [滚动翻页] 第 {}/{} 页", idx + 1, PAGES);
    }

    println!("[翻页演示] 结束，屏幕停留在最后一页");
    Ok(())
}
