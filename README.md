# I2C Display Driver

面向 Linux I2C 子系统的 SSD1309 OLED 显示屏驱动库（Rust），针对项目控制器特化（128×64 单色 OLED）。

> 本项目从「OLED 桌宠」项目迁移精简而来，当前保留的是**驱动核心**，后续将在此基础上补全文字 / 图形 / 稳定显示能力。

## 当前能力

- **I2C 底层封装**：`/dev/i2c-N` + `ioctl(I2C_SLAVE)` 绑定从机地址，命令（0x00）/ 数据（0x40）分通道写入
- **SSD1309 控制器驱动**：初始化、逐页推帧、对比度、休眠、总线恢复
- **1-bit 帧缓冲**：1024 字节（8 页 × 128 列），实现 embedded-graphics `DrawTarget`
- **稳定性**：逐页推帧避开 Pi 5 RP1 大块传输限制、栈缓冲区零堆分配、`recover()` 总线复位

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
# Ok::<(), i2c_display_driver::utils::AppError>(())
```

帧缓冲实现了 `embedded_graphics::DrawTarget`，可直接绘制原语：

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
lib.rs ── 模块声明（pub mod display; pub mod utils;）
├── display/
│   ├── i2c_bus.rs      I2C 底层封装（ioctl 从机地址 + write）
│   ├── ssd1309.rs      SSD1309 控制器驱动（init / push_frame / set_contrast / sleep）
│   ├── framebuffer.rs  1-bit 帧缓冲（embedded-graphics DrawTarget）
│   └── mod.rs          Display 顶层句柄（open / render / recover）
└── utils/
    └── error.rs        AppError 统一错误类型
```

公开 API：`display::Display`、`display::Framebuffer`、`utils::AppError`。

## 依赖

- `libc` — ioctl 系统调用
- `embedded-graphics` — 帧缓冲 DrawTarget 集成

## 平台

Linux（开发/测试目标为 aarch64，如树莓派）。代码依赖 `/dev/i2c-N` 与 `std::os::fd`，不支持 Windows / macOS。

## 命令

```bash
cargo build        # 构建库
cargo test         # 运行测试（帧缓冲单元测试）
cargo check        # 快速检查
cargo clippy       # 静态检查
```

## 路线图（规划中）

- 自动恢复策略（推帧失败 → 总线复位 → 重试，`render_robust`）
- 电源 / 亮度 API 补全（`wake` / `set_inverted`）
- 文字渲染（5×7 标准 + 4×6 小字体，含反色 / 紧凑模式）
- 基础图形（矩形 / 直线 / 圆 / 三角形 / 点线）
- 错误类型优化（`DriverError`）
