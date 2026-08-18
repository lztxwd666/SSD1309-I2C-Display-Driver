# Repository Guidelines

Rust library driving SSD1309 I2C OLED displays (128×64 monochrome) on Linux. Library-only crate; no binaries.

## Project Structure & Module Organization

Workspace root `Cargo.toml`; the crate lives in `i2c_display_driver/` (edition 2024).

- `src/lib.rs` — public `display`, `error`, and `graphics` modules
- `src/display/` — `i2c_bus`, `ssd1309`, `framebuffer`, `mock`, and the top-level `Display` handle in `mod.rs`
- `src/graphics/` — 5×7 `font`, `text`, and shape drawing in `canvas`
- `src/error.rs` — unified `DriverError`
- `examples/` — hardware demos: `smoke`, `showcase`, `feature_check`
- Tests live in `#[cfg(test)]` modules next to the code they cover; font data is embedded in `font.rs` (no asset files)

## Build, Test & Development Commands

Run from `i2c_display_driver/`:

- `cargo build` — compile the library
- `cargo test` — run unit tests (hardware-free via `MockBus`)
- `cargo check` — fast compile check
- `cargo clippy` — lint
- `cargo fmt` — format
- `cargo run --example <smoke|showcase|feature_check>` — hardware demo

Linux-only: requires `/dev/i2c-N` and `std::os::fd` (aarch64 Debian 12). On Raspberry Pi enable I2C with `dtparam=i2c_arm=on` in `/boot/firmware/config.txt`.

## Coding Style & Naming Conventions

Standard rustfmt (4 spaces); `cargo fmt --check` must pass. Use snake_case identifiers and name tests for observed behavior (`out_of_bounds_ignored`). Comments are written in Chinese with technical terms (I2C, SSD1309, RP1, framebuffer) in English; avoid `=` or `-` divider lines; every `unsafe` block needs a `// SAFETY:` comment. Public APIs get rustdoc examples.

## Testing Guidelines

Prefer unit tests in `#[cfg(test)]` modules using `MockBus`; they must pass without hardware. Cover framebuffer pixel writes, dirty-rectangle tracking, and driver command sequences. Keep clippy warnings clean.

## Commit & Pull Request Guidelines

Use Conventional Commits with Chinese descriptions: `feat:`, `fix:`, `refactor:`, `revert:`. Keep commits atomic and focused.

Pull requests should explain what changed and why, state which tests were run, note any on-hardware validation, and keep the examples compiling. Include visual diffs only for graphics or rendering changes.
