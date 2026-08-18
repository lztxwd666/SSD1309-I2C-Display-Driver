//! Linux I2C 总线底层封装。
//!
//! 直接操作 `/dev/i2c-N` 设备节点，通过 ioctl 绑定从机地址后
//! 使用 write 系统调用发送数据。
//!
//! [`I2cDevice`] trait 抽象了驱动对 I2C 的全部需求（命令/数据两通道写入），
//! 便于测试注入 Mock 总线或接入其他后端。

use std::fs;
use std::io::{self, Write};
use std::os::fd::AsRawFd;

const I2C_SLAVE: u64 = 0x0703;

/// I2C 设备抽象：驱动所需的写入接口。
///
/// 实现者需提供命令（控制字节 0x00）与数据（控制字节 0x40）两通道写入。
pub trait I2cDevice {
    /// 发送命令（0x00 控制字节前缀）。
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// 发送 GDDRAM 数据（0x40 控制字节前缀）。
    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()>;
}

/// I2C 设备工厂：按总线编号与从机地址打开设备。
///
/// 供 [`Display`](crate::display::Display) 在初始化与总线恢复时重建底层设备。
pub trait I2cDeviceFactory: Sized {
    /// 打开指定总线上的设备。
    fn open(bus_id: u8, addr: u8) -> io::Result<Self>;
}

/// Linux I2C 总线。
pub struct I2cBus {
    file: fs::File,
    #[allow(dead_code)]
    addr: u8,
}

/// I2C 写入，错误原样返回（不做重试）。
///
/// 经验证，Pi 5 RP1 控制器的偶发 EIO/EREMOTEIO 在无重试干扰时极少发生
/// 错误由上层 `recover()`（关闭/重开 I2C 总线）
/// 处理。不在本层重试的原因：sleep 导致的 I2C 空闲期会触发 RP1 控制器
/// 重新同步，反复 sleep/write 循环反而制造本不存在的故障链。
fn write_i2c(mut file: &fs::File, buf: &[u8]) -> io::Result<()> {
    file.write_all(buf)
}

impl I2cBus {
    /// 打开 /dev/i2c-{bus} 并绑定从机地址。
    pub fn open(bus: u8, addr: u8) -> io::Result<Self> {
        let path = format!("/dev/i2c-{}", bus);
        let file = fs::OpenOptions::new().read(true).write(true).open(&path)?;
        // SAFETY: file 是刚打开的合法 fd，I2C_SLAVE ioctl 仅设置从机地址，无副作用。
        let ret = unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE, addr as u32) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file, addr })
    }
}

impl I2cDevice for I2cBus {
    /// 发送 I2C 命令（控制字节 0x00 前缀）。使用栈缓冲区避免堆分配。
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()> {
        const MAX: usize = 255;
        if bytes.len() > MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("I2C 命令过长: {} > {}", bytes.len(), MAX),
            ));
        }
        let mut buf = [0u8; 256];
        buf[0] = 0x00;
        buf[1..1 + bytes.len()].copy_from_slice(bytes);
        write_i2c(&self.file, &buf[..1 + bytes.len()])
    }

    /// 发送 GDDRAM 数据（控制字节 0x40 前缀）。使用栈缓冲区避免堆分配。
    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()> {
        const MAX: usize = 255;
        if bytes.len() > MAX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("I2C 数据过长: {} > {}", bytes.len(), MAX),
            ));
        }
        let mut buf = [0u8; 256];
        buf[0] = 0x40;
        buf[1..1 + bytes.len()].copy_from_slice(bytes);
        write_i2c(&self.file, &buf[..1 + bytes.len()])
    }
}

impl I2cDeviceFactory for I2cBus {
    fn open(bus_id: u8, addr: u8) -> io::Result<Self> {
        Self::open(bus_id, addr)
    }
}
