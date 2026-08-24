# i2c_display_driver

面向 Linux I2C 子系统的 **SSD1309 OLED 显示屏驱动库**（Rust，128×64 单色），为树莓派等嵌入式设备提供完整的显示能力。

同时提供纯 Python 版驱动，见 [`python_driver/`](python_driver/README.md)。

## 特性

- **帧缓冲**：1024 字节 1-bit 帧缓冲（8 页 × 128 列），实现 `embedded-graphics` `DrawTarget`，可接收矩形/圆/文字/图像等原语
- **脏矩形跟踪**：自动记录修改区域，`render_dirty()` 只推送变化部分，节省 I2C 带宽
- **文字与图形**：内嵌 5×7 位图字体（含 `°`/`↑`/`↓`），矩形/直线/圆/三角形等基础图形
- **位图 blit**：`Framebuffer::blit` 绘制任意线性 1-bit 位图（含覆盖/点亮两种模式），便于自绘字形/图标
- **软件滚动**：水平/垂直循环滚动（帧缓冲平移），跑马灯效果
- **旋转支持**：0°/180° 硬件方向切换，适配屏幕安装方向
- **高级寄存器调节**：显示偏移、起始行、多路复用比、时钟、预充电、VCOMH、COM 引脚、电荷泵
- **硬件滚动命令封装**：保留 SSD1309 硬件滚动命令入口，默认仍使用软件滚动
- **多页显示**：`PageBuffer` 多页管理，瞬时/滚动动画两种翻页形式
- **可靠显示**：`render_robust()` 自动恢复 I2C 总线（失败后冷却退避），恢复后沿用对比度/反色/显示开关等设置；逐页推帧规避 Pi 5 RP1 大块传输限制
- **状态读取**：`read_status()` 读取 SSD1309 状态寄存器（忙/电荷泵），辅助故障诊断
- **可观测性**：运行统计（推帧数/错误数/恢复次数）、日志回调
- **无硬件测试**：记录型 `MockBus` 验证命令序列，测试全部离线运行（含长稳故障注入测试）

## 快速开始

```rust
use i2c_display_driver::display::Display;
use i2c_display_driver::graphics::text;

fn main() -> Result<(), i2c_display_driver::DriverError> {
    // 打开总线 1 上的 OLED（地址 0x3C）
    let mut display = Display::open(1, 0x3C)?;

    // 绘制并推帧
    display.framebuffer.clear();
    text::draw_text(&mut display.framebuffer, 0, 0, "Hello SSD1309!");
    display.render()?;

    // 程序退出前休眠（可选）
    display.sleep()?;
    Ok(())
}
```

## 功能

### 帧缓冲与局部推帧

帧缓冲布局与 GDDRAM 完全一致（`buffer[page * 128 + col]`，每字节为一列 8 个垂直像素）。所有写入自动记录脏矩形，`render_dirty()` 只推送变化区域：

```rust
display.framebuffer.set_pixel(20, 20, true);
display.render_dirty()?; // 只推送 (20,20) 所在页与列
```

### 文字与图形

```rust
use i2c_display_driver::graphics::{canvas, text};

display.framebuffer.clear();
text::draw_text(&mut display.framebuffer, 0, 0, "ALERT: high temp"); // 支持 \n 换行
canvas::draw_rect(&mut display.framebuffer, 0, 30, 40, 20);
canvas::fill_circle(&mut display.framebuffer, 100, 40, 10);
display.render()?;
```

### 软件滚动

实测本项目屏幕不响应硬件滚动命令（`0x26`/`0x2F`），因此滚动在帧缓冲层实现：定时调用即可获得跑马灯效果。

```rust
use i2c_display_driver::display::{ScrollDirection, VerticalDirection};

display.software_scroll_horizontal(ScrollDirection::Left, 1)?; // 循环左移 1 像素
display.software_scroll_vertical(VerticalDirection::Up, 1)?;    // 内容循环上移 1 行
```

滚动为循环语义（移出屏幕的内容从另一侧进入）。注意：I2C 总线 100kHz 下每帧全推约 100ms，滚动帧率上限约 8-10 帧/秒，属预期表现。

### 旋转与初始化 Builder

```rust
use i2c_display_driver::display::{DisplayBuilder, DisplayRotation};

let mut display = DisplayBuilder::new(1, 0x3C)
    .with_contrast(0xCF)
    .with_inverted(false)
    .with_display_on(true)
    .with_rotation(DisplayRotation::Rotate180)
    .build()?;
```

运行时也可以通过 `set_rotation()` 切换 0°/180°：

```rust
display.set_rotation(DisplayRotation::Rotate180)?;
```

### 高级显示控制

以下方法可直接调节 SSD1309 寄存器，并会在 `recover()` 后保持：

```rust
display.set_display_offset(0)?;
display.set_start_line(0)?;
display.set_multiplex_ratio(0x3F)?;
display.set_clock(0, 8)?;
display.set_precharge_period(0x01, 0x0F)?;
display.set_vcomh_level(0x40)?;
display.set_com_pins_config(0x12)?;
display.set_charge_pump(true)?;
```

### 硬件滚动（兼容性入口）

当前项目实测屏幕不响应硬件滚动命令，日常滚动请使用软件滚动。这里保留硬件滚动命令封装，用于其他 SSD1309 面板验证或调试：

```rust
use i2c_display_driver::display::ScrollDirection;

display.hardware_scroll_horizontal(ScrollDirection::Left, 0, 7, 0x00)?;
display.deactivate_scroll()?;
```

### 多页显示

`PageBuffer` 管理多个独立页面，页面内容在切换间保留：

```rust
use i2c_display_driver::display::PageBuffer;

let mut pages = PageBuffer::<3>::new();      // 3 页
pages.page_at_mut(0).unwrap().set_pixel(5, 5, true); // 绘制指定页
pages.next_page();                           // 翻页（循环）

// 两种翻页形式，按应用场景自行选择：
display.show_page(pages.page_at(0).unwrap())?;                 // 瞬时切换，无动画
display.scroll_to_page(pages.page_at(1).unwrap(), 32, 10)?;    // 滚动动画（新页从右侧滚入）
```

### 位图 blit 与状态读取

```rust
use i2c_display_driver::display::BlitMode;

// 绘制线性 1-bit 位图（逐行打包，MSB 优先）到指定位置
let data = [0b1111_0000, 0b1100_0000]; // 10×1 位图
display.framebuffer.blit(0, 0, 10, 1, &data, BlitMode::Set).unwrap(); // Set=只点亮，Overwrite=覆盖

// 读取状态寄存器（bit7=忙，bit0=电荷泵使能），辅助故障诊断
let st = display.read_status()?;
println!("busy={} booster={}", st & 0x80 != 0, st & 0x01 != 0);
```

### 自动恢复与统计

```rust
// 最佳努力推帧：失败自动重置 I2C 总线并重试一次
let status = display.render_robust();
// status: Ok | Recovered | Skipped

// 运行统计
let s = display.stats();
println!("推帧 {} 次，跳过 {} 帧，恢复 {} 次，错误 {} 次",
    s.frames_pushed, s.frames_skipped, s.recoveries, s.errors);
```

## 示例

| 示例 | 用途 |
|------|------|
| `smoke` | 冒烟测试：文字 + 基础图形 + 推帧 |
| `showcase` | 全功能循环演示（文字/图形/日志/反色/对比度） |
| `feature_check` | 新功能验证：clear/fill/全屏点亮/局部推帧/滚动/recover/统计 |
| `panel_tuning` | 面板方向与高级参数演示：Rotate0/180、寄存器调节、硬件滚动兼容入口 |
| `diag` | 硬件诊断 + 软件滚动演示 |
| `scroll_demo` | 长文本跑马灯 + 垂直滚动演示 |
| `page_demo` | 多页仪表：瞬时与滚动动画两种翻页对比 |
| `selftest` | 硬件自检：一键验证全链路（状态/全亮/字符表/滚动/翻页/统计） |
| `stress` | 长稳压力测试：长时间循环覆盖全部功能路径（可配置轮数） |

```bash
cargo run --example smoke
```

## 架构

```
src/
├── lib.rs           模块组织与文档
├── error.rs         统一错误类型 DriverError
└── display/
    ├── i2c_bus.rs   I2C 设备抽象（I2cDevice 读写 / I2cDeviceFactory）+ Linux 实现
    ├── ssd1309.rs   SSD1309 控制器：初始化 / 逐页推帧 / 局部推帧 / 显示控制 / 状态读取
    ├── framebuffer.rs 1-bit 帧缓冲 + 脏矩形 + blit 位图 + PageBuffer
    ├── mock.rs      记录型 MockBus（仅测试编译）
    └── mod.rs       Display 顶层句柄：推帧 / 恢复 / 统计 / 软件滚动 / 多页
└── graphics/
    ├── font.rs      5×7 位图字体（含 °/↑/↓ 特殊字形）
    ├── text.rs      文字渲染（标准 / 紧凑 / 反色）
    └── canvas.rs    基础图形（矩形 / 直线 / 圆 / 三角形）
```

分层设计：`I2cDevice` trait 抽象底层总线（真机 `I2cBus`，测试注入 `MockBus`），`Display<B>` 泛型于设备类型——测试无需硬件即可验证完整命令序列。

## 平台要求

- Linux（依赖 `/dev/i2c-N` 与 `std::os::fd`，不支持 Windows / macOS）
- 树莓派启用 I2C：`/boot/firmware/config.txt` 中 `dtparam=i2c_arm=on`
- OLED 地址默认 `0x3C`（部分模块为 `0x3D`）

## 依赖

- `libc` — `ioctl(I2C_SLAVE)` 系统调用
- `embedded-graphics` — 帧缓冲 `DrawTarget` 集成

## License

MIT
