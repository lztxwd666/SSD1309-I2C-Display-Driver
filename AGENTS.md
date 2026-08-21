# AGENTS.md — 仓库开发指南

Rust 库驱动 SSD1309 I2C OLED（128×64 单色），仅面向 Linux。crate 位于 `i2c_display_driver/`（edition 2024），只发布库，无二进制目标。

## 项目结构与模块组织

- `src/lib.rs` — 公开 `display`、`error`、`graphics` 三个模块
- `src/display/` — `i2c_bus`、`ssd1309`、`framebuffer`、`mock`（仅测试编译），顶层句柄在 `mod.rs`
- `src/graphics/` — 5×7 `font`、`text`、`canvas`（基础图形）
- `src/error.rs` — 统一 `DriverError`
- `examples/` — 硬件演示：`smoke`、`showcase`、`feature_check`、`diag`、`scroll_demo`、`page_demo`、`selftest`、`stress`
- 测试位于 `#[cfg(test)]` 模块（与被测代码相邻）；字体数据内嵌于 `font.rs`（无资源文件）

分层结构（自底向上）：

1. **`i2c_bus.rs`** — `I2cDevice` trait（命令 0x00 / 数据 0x40 双通道写入）+ `I2cDeviceFactory` trait（按总线/地址重建设备，供 recover 使用）。`I2cBus` 为 Linux `/dev/i2c-N` 实现。
2. **`ssd1309.rs`** — `Ssd1309<B>` 控制器驱动：初始化序列、`push_frame`（逐页推 1024 字节）、`render_region`（局部推帧）、对比度/反色/休眠/唤醒。初始化含 SSD1309 特有 DC-DC 命令（`0xAD 0x8A`）。
3. **`framebuffer.rs`** — 1-bit 帧缓冲，布局 `buffer[page * 128 + col]`：**每字节是一列 8 个垂直像素**（bit0=顶部）。实现 embedded-graphics `DrawTarget`，自动脏矩形跟踪。
4. **`display/mod.rs`** — `Display<B>` 顶层句柄（泛型参数为底层 I2C 设备，默认 `I2cBus`，测试注入 `MockBus`）：推帧/局部推帧/自动恢复（`render_robust`）/统计/日志回调/软件滚动。`recover()` 关闭并重开 I2C 连接再重初始化。
5. **`graphics/`** — 软件绘制层：`font`（5×7 位图字体，含 °/↑/↓ 特殊字形）、`text`（支持 `\n` 换行）、`canvas`（矩形/直线/圆/三角形/点线）。

测试：`mock.rs` 提供记录型 `MockBus`（记录全部 I2C 写入并可注入失败），测试通过断言写入序列验证命令正确性，全部无硬件运行。

## 常用命令

在 `i2c_display_driver/` 目录下运行：

```bash
cargo build                  # 编译库
cargo check                  # 快速编译检查
cargo test                   # 全部单元测试（MockBus 模拟 I2C，无需硬件）
cargo test <名称子串>         # 筛选单个测试，如 cargo test software_scroll
cargo clippy --all-targets   # lint（要求 0 告警）
cargo fmt --check            # 格式检查（要求通过）
cargo run --example smoke          # 冒烟：文字 + 基础图形 + 推帧
cargo run --example showcase       # 全功能循环演示（文字/图形/日志/反色/对比度）
cargo run --example feature_check  # 新功能验证（clear/fill/全屏点亮/局部推帧/软件滚动/recover/统计）
cargo run --example diag           # 硬件诊断 + 软件滚动演示
cargo run --example scroll_demo    # 长文本跑马灯 + 垂直滚动演示
cargo run --example page_demo      # 多页仪表：瞬时与滚动动画两种翻页对比
cargo run --example selftest       # 硬件自检：一键验证全链路
cargo run --example stress [轮数]  # 长稳压力测试（默认 3000 轮）
cargo test -- --ignored            # 长稳自动化测试（MockBus 故障注入验证恢复链路）
```

仅限 Linux：依赖 `/dev/i2c-N` 与 `std::os::fd`（aarch64 Debian 12）。树莓派需在 `/boot/firmware/config.txt` 启用 `dtparam=i2c_arm=on`。

## 关键硬件约束（实测结论，勿违背）

- **屏幕不响应硬件滚动命令**（0x26/0x27/0x2F）——滚动必须用软件实现：
  - 水平：`software_scroll_horizontal`，每页 128 字节 `rotate_left/right`（字节=列，无需位操作）
  - 垂直：`software_scroll_vertical`，每列 64 行组装 `u64` 循环移位（**上移=`rotate_right`、下移=`rotate_left`**——位号方向易搞反，曾有测试抓出）
- **I2C 总线 100kHz**（总线上挂载其他设备，勿改为 400kHz）：全帧推 1024 字节 ≈ 100ms，软件滚动帧率上限约 8-10 帧/秒——这是预期值，不是 bug。
- **页寻址模式**（`0x20 0x02`）是刻意选择，规避 Pi 5 RP1 I2C 控制器的大块传输限制。
- `render_region` 只清除被本次推送完全覆盖的脏矩形，未覆盖部分保留（避免局部推帧丢失更新）。

## 关键约定

- 所有代码注释用**中文**书写
- **注释只含实质内容**：陈述事实或理由，无口语化或填充词，无法实质表述时省略该注释
- 避免用 `=` 或 `-` 作为注释分隔线
- 技术术语（CPU、RAM、SSH、I2C、SSD1309、RP1、framebuffer 等）保留英文
- `unsafe` 允许但最小化：部分操作（原始 FFI、自定义布局）无 `unsafe` 不可能——这是 Rust 限制而非错误。每个 `unsafe` 块必须有 `// SAFETY:` 注释说明其安全性不变式；优先用安全包装器将 `unsafe` 封装在窄接口内。新代码在存在安全替代时以零 `unsafe` 为目标
- **代码与注释中禁止 emoji**
- 修改或新增代码时，应先分析问题本质并选择最适合当前场景的实现，不能为了“能跑”而采用最简单粗暴的补丁；同时必须避免引入新问题或拆东墙补西墙

## 编码风格与命名规范

标准 rustfmt（4 空格缩进）；`cargo fmt --check` 必须通过。标识符用 snake_case，测试以观察到的行为命名（如 `out_of_bounds_ignored`）。公开 API 提供 rustdoc 示例。

## 测试规范

优先使用 `MockBus` 编写 `#[cfg(test)]` 模块内的单元测试，无硬件时也必须通过。覆盖帧缓冲像素写入、脏矩形跟踪、驱动命令序列。保持 clippy 告警清零。

## 提交与 PR 规范

使用带中文描述的 Conventional Commits：`feat:`、`fix:`、`refactor:`、`revert:`。提交保持原子与聚焦。

PR 应说明改了什么及原因、运行了哪些测试、是否有硬件实测验证，并保证示例可编译。图形或渲染相关的改动需附带视觉对比。

## 开发工作流

Windows 编写代码 → git 推送 → 本机（树莓派）拉取测试。Git 已配置 `pull.rebase = false`（merge 方式）。
