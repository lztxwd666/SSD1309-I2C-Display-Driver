# AGENTS.md

本文件为在此仓库工作的 AI 编码助手（Claude Code / DeepSeek Harness / Cursor / Copilot 等）提供指导。

## 项目

SSD1309 I2C OLED 显示屏驱动库（Rust），面向 Linux I2C 子系统，针对项目控制器特化（128×64 单色 OLED）。

**当前状态**：从「OLED 桌宠」项目迁移精简而来的驱动核心。已保留 I2C 底层封装、SSD1309 控制器驱动、1-bit 帧缓冲（embedded-graphics DrawTarget）与统一错误类型。桌宠、系统监控、通知、UI 布局、字体渲染、配置加载等应用层代码已全部移除。文字 / 图形 / 稳定显示能力在规划中。

## 工作区

- 根 `Cargo.toml`：workspace，`members = ["i2c_display_driver"]`，`resolver = "2"`
- `i2c_display_driver/`：驱动库 crate（edition 2024），依赖 `libc = "0.2"`、`embedded-graphics = "0.8"`
- 纯库（无二进制），`[lib] name = "i2c_display_driver"`
- `Cargo.lock` 已删除，下次 `cargo build` 自动重新生成

## 命令

```bash
cd i2c_display_driver
cargo build        # 构建库
cargo test         # 帧缓冲单元测试
cargo check        # 快速检查
cargo clippy       # 静态检查
```

平台：aarch64，Debian 12 Bookworm，Rust 1.96+。代码依赖 `/dev/i2c-n` 与 `std::os::fd`，仅限 Linux。

## 硬件（基线）

- 显示：SSD1309 128×64 单色 OLED，I2C 地址 `0x3C`
- 总线：I2C-1（`/dev/i2c-1`）
- `/boot/firmware/config.txt` 需启用 I2C（`dtparam=i2c_arm=on`）

## 架构

```
lib.rs ── pub mod display; pub mod error; pub mod graphics;
├── display/
│   ├── i2c_bus.rs      I2C 底层封装（I2cBus，模块私有）
│   ├── ssd1309.rs      SSD1309 控制器（Ssd1309，模块私有）
│   ├── framebuffer.rs  1-bit 帧缓冲（Framebuffer，公开）
│   └── mod.rs          Display 顶层句柄（公开）
├── graphics/           软件绘制层（font / text / canvas）
└── error.rs            DriverError（公开）
```

> 注：`graphics/` 与 `error.rs` 为规划中的结构，实现后生效。

## 关键数据流

```
Display::open(bus, addr)
  → I2cBus::open（/dev/i2c-N + ioctl(I2C_SLAVE)）
  → Ssd1309::init（控制器初始化序列）
  → Framebuffer::new（1024 字节全零）

绘制：Framebuffer（set_pixel / embedded-graphics DrawTarget）+ graphics::text / graphics::canvas
推帧：Display::render → Ssd1309::push_frame → 8 页 × 128 字节逐页写入
```

## 关键实现细节

### I2C 底层（display/i2c_bus.rs）

- `write(&[0x00, cmd..])` = 命令，`write(&[0x40, data..])` = GDDRAM 数据
- 栈分配 `[u8; 256]` 缓冲，零堆分配
- 低层不重试：错误上抛由上层 `recover()` 处理。经验证 Pi 5 RP1 偶发 EIO/EREMOTEIO 在无重试干扰时极少发生，反复 sleep/write 循环反而制造故障链

### SSD1309 驱动（display/ssd1309.rs）

- `0xAD 0x8A`：SSD1309 DC-DC 转换器，缺失会导致花屏
- `0x20 0x02`：页寻址模式（非水平 0x00）
- 逐页推帧：8 页 × 128 字节（避开 Pi 5 RP1 大块传输限制）
- 电荷泵 + DC-DC 使能后 100ms 延时
- `sleep()` → `0xAE`；`set_contrast()` → `0x81, val`

### 总线恢复（display/mod.rs）

- `driver: Option<Ssd1309>`：`recover()` 先置 None 关闭旧 fd，再开新 fd 重新 init，避免两个 fd 同时指向 /dev/i2c-N
- `render()/set_contrast()/sleep()` 在驱动未初始化（recover 失败后）返回错误而非 panic

### 帧缓冲（display/framebuffer.rs）

- 1024 字节，`buffer[page*128 + col]` 寻址，bit 0 = 页内顶部像素
- 实现 `OriginDimensions`（128×64）+ `DrawTarget<BinaryColor>`（Infallible）
- 越界像素静默忽略

## 标注规范

- 所有代码注释中文
- 技术术语（I2C、SSD1309、RP1、framebuffer）保持英文
- 注释分隔不用 `=` 或 `-` 连线
- `unsafe` 块用 `// SAFETY:` 注释说明

## 子代理

- `embedded-reviewer`：审查嵌入式 Rust 代码，重点是安全性与性能
