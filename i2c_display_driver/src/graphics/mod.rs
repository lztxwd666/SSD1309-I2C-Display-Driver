//! 软件绘制层 —— 在 1-bit 帧缓冲区上绘制文字和基础图形。
//!
//! * `font` — 5×7 + 4×6 位图字体
//! * `text` — 文字渲染（标准 / 紧凑 / 反色 / 小号）
//! * `canvas` — 基础图形（矩形 / 直线 / 圆 / 三角形 / 点线）

pub mod canvas;
pub mod font;
pub mod text;
