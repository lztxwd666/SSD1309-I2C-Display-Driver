# I2C Display Driver

面向 Linux I2C 子系统的 SSD1309 OLED 显示屏驱动库（Rust），针对项目控制器特化（128×64 单色 OLED）。

## 当前能力

- **I2C 底层封装**：`/dev/i2c-N` + `ioctl(I2C_SLAVE)` 绑定从机地址，命令（0x00）/ 数据（0x40）分通道写入
- **SSD1309 控制器驱动**：可配置初始化（`DisplayConfig`）、逐页/局部推帧、对比度、反色、全屏点亮测试、休眠 / 唤醒、硬件滚动、总线恢复
- **1-bit 帧缓冲**：1024 字节（8 页 × 128 列），实现 embedded-graphics `DrawTarget`，自动脏矩形跟踪（`render_dirty()` 只推变化区域）
- **文字渲染**：5×7 字体，支持反色 / 紧凑模式
- **基础图形**：矩形、直线、圆、三角形（含填充与点线）
- **稳定显示**：`render_robust()` 自动恢复、恢复后沿用对比度/反色设置、逐页推帧避开 Pi 5 RP1 大块传输限制、栈缓冲区零堆分配
- **可观测性**：运行统计（帧数 / 错误数 / 恢复次数）、日志回调（默认 stderr）

## 硬件

- SSD1309 128×64 单色 OLED，I2C 地址 `0x3C`（或 `0x3D`）
- Linux I2C 设备节点 `/dev/i2c-N`（树莓派上通常为 `/dev/i2c-1`）

## 快速开始

```rust
use i2c_display_driver::display::Display;

// bus=1, addr=0x3C
let mut display = Display::open(1, 0x3C)?;

// 在帧缓冲区上绘制
display.framebuffer.clear();
display.framebuffer.set_pixel(10, 10, true);

// 推帧到屏幕
display.render()?;

// 长期不用时休眠（关闭显示）
display.sleep()?;
# Ok::<(), i2c_display_driver::DriverError>(())
```

文字与基础图形：

```rust
use i2c_display_driver::graphics::{canvas, text};

display.framebuffer.clear();
text::draw_text(&mut display.framebuffer, 0, 0, "ALERT: high temp");
text::draw_text(&mut display.framebuffer, 0, 16, "details");
canvas::draw_rect(&mut display.framebuffer, 0, 30, 40, 20);
canvas::fill_circle(&mut display.framebuffer, 100, 40, 10);
display.render()?;
```

局部推帧（只推送变化区域）与硬件滚动：

```rust
// 只推送脏矩形区域（帧缓冲自动跟踪修改）
display.framebuffer.set_pixel(20, 20, true);
display.render_dirty()?;

// 水平滚动（设置后需 activate_scroll 生效）
use i2c_display_driver::display::{ScrollDirection, ScrollFrameInterval};
display.scroll_horizontal(ScrollDirection::Right, 0, 3, ScrollFrameInterval::Frames5)?;
display.activate_scroll()?;
```

帧缓冲实现了 `embedded_graphics::DrawTarget`，也可直接绘制原语：

```rust
use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};

Rectangle::new(Point::new(0, 0), Size::new(20, 20))
    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
    .draw(&mut display.framebuffer)?;
```

## 架构

```
lib.rs ── pub mod display; pub mod error; pub mod graphics;
├── display/
│   ├── i2c_bus.rs      I2C 底层封装（ioctl 从机地址 + write）
│   ├── ssd1309.rs      SSD1309 控制器驱动（init / push_frame / render_region / 滚动 / 全屏点亮）
│   ├── framebuffer.rs  1-bit 帧缓冲（embedded-graphics DrawTarget + 脏矩形）
│   └── mod.rs          Display 顶层句柄（open_config / render* / recover / scroll / stats）
├── graphics/
│   ├── font.rs         5×7 位图字体
│   ├── text.rs         文字渲染
│   └── canvas.rs       基础图形
└── error.rs            DriverError 统一错误类型
```

公开 API：`display::Display`、`display::DisplayConfig`、`display::Framebuffer`、`display::{RenderStatus, DriverStats, ScrollDirection, ScrollFrameInterval}`、`graphics::{text, canvas, font}`、`DriverError`。

## 依赖

- `libc` — ioctl 系统调用
- `embedded-graphics` — 帧缓冲 DrawTarget 集成

## 平台

Linux（开发/测试目标为 aarch64，如树莓派）。代码依赖 `/dev/i2c-N` 与 `std::os::fd`，不支持 Windows / macOS。

## 命令

```bash
cargo build        # 构建库
cargo test         # 运行测试（帧缓冲 / 文字 / 图形单元测试）
cargo check        # 快速检查
cargo clippy       # 静态检查
```

## 示例

```bash
cargo run --example smoke           # 冒烟测试：文字 + 图形 + 推帧
cargo run --example showcase        # 全功能演示：循环展示文字 / 图形 / 日志 / 反色 / 对比度
cargo run --example feature_check   # 新功能验证：clear/fill / 全屏点亮 / 局部推帧 / 滚动 / recover / 统计
```

- `examples/smoke.rs` 验证基本链路（I2C → 帧缓冲 → 文字 / 图形 → 推帧）
- `examples/showcase.rs` 循环展示全部功能，含 `render_robust()` 自动恢复演示
- `examples/feature_check.rs` 逐项验证新增功能，并打印脏矩形与运行统计
