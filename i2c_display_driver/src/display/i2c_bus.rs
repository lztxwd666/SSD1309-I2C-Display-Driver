//! Linux I2C 总线底层封装。
//!
//! 直接操作 `/dev/i2c-N` 设备节点，通过 ioctl 绑定从机地址后
//! 使用 write 系统调用发送数据。
//!
//! 写帧格式（协议 v2）：
//! - 命令/数据帧：控制字节 + 1 字节载荷长度 + 载荷 + Modbus CRC16
//! - 状态读请求：`0x00 0x00` + CRC
//! - 状态读响应：状态字节 + CRC（3 字节）
//!
//! CRC 覆盖控制字节、长度字节与载荷，低字节在前。接收端根据长度字节确定
//! 帧尾，在最后一个字节（CRC_H）的 ACK 周期校验长度与 CRC，失败时必须 NACK；
//! Linux i2c-dev 通常将 NACK 映射为 `EREMOTEIO`，本层据此重试并上报。
//! 帧格式见项目 `docs/i2c-crc-protocol.md`。
//!
//! 错误约定：帧校验失败（接收端 NACK 或状态帧 CRC/长度不符）统一使用
//! `io::ErrorKind::InvalidData`；`Display` 据此映射为 `DriverError::FrameRejected`。
//! 自定义 [`I2cDevice`] 实现需遵循同一约定，其他 I/O 错误使用其他 `ErrorKind`。
//!
//! [`I2cDevice`] trait 抽象了驱动对 I2C 的全部需求（命令/数据两通道写入），
//! 便于测试注入 Mock 总线或接入其他后端。

use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

use super::crc;

const I2C_SLAVE: u64 = 0x0703;

/// 单帧命令/数据载荷上限。
const MAX_PAYLOAD: usize = 255;
/// 帧缓冲：控制字节 1 + 长度字节 1 + 最大载荷 255 + CRC 2。
const FRAME_MAX: usize = 2 + MAX_PAYLOAD + 2;
/// 状态读响应帧长度：状态字节 + CRC 2。
const STATUS_FRAME_LEN: usize = 3;
/// 接收端 NACK 后的重试次数（不含首次发送）。
const FRAME_RETRIES: usize = 2;

/// I2C 设备抽象：驱动所需的写入与读取接口。
///
/// 实现者需提供命令（控制字节 0x00）与数据（控制字节 0x40）两通道写入，
/// 以及一个字节的状态读取。CRC 帧封装属于传输实现职责：
/// [`I2cBus`] 已按 `docs/i2c-crc-protocol.md` 实现，自定义实现需遵循同一协议。
///
/// 错误约定：帧校验失败（接收端 NACK 或响应帧 CRC/长度不符）返回
/// [`io::ErrorKind::InvalidData`]，[`Display`](crate::display::Display) 会映射为
/// [`DriverError::FrameRejected`](crate::DriverError::FrameRejected)；
/// 其他 I/O 错误使用其他 `ErrorKind`。
pub trait I2cDevice {
    /// 发送命令（0x00 控制字节前缀）。
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// 发送 GDDRAM 数据（0x40 控制字节前缀）。
    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// 读取一个字节（SSD1309 状态寄存器：bit7=忙，bit0=电荷泵使能）。
    fn read(&mut self) -> io::Result<u8>;
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
}

/// 判断 I/O 错误是否按“接收端拒绝本帧”处理。
///
/// 当前策略把 `EREMOTEIO` 一律视为帧拒绝。注意 Linux DesignWare（RP1）
/// 对地址 NACK 与 SDA stuck 也可能返回该 errno，此时同样按帧拒绝跳过、
/// 不触发 `recover()`；这是本库的既定行为。
fn is_data_nack(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::EREMOTEIO)
}

/// 组装写帧：控制字节 + 载荷长度 + 载荷 + Modbus CRC16（低字节在前）。
///
/// 返回帧总长度，帧内容写入 `out[..len]`。
/// 调用方需保证 `payload.len() <= MAX_PAYLOAD`。
fn build_frame(control: u8, payload: &[u8], out: &mut [u8; FRAME_MAX]) -> usize {
    let payload_len = payload.len();
    let body_len = 2 + payload_len;
    out[0] = control;
    out[1] = payload_len as u8;
    out[2..body_len].copy_from_slice(payload);
    crc::append_crc(out, body_len)
}

/// 发送一帧；接收端 NACK 时重试，其他 I/O 错误直接返回。
///
/// 重试仅针对 `EREMOTEIO`（按当前策略视为接收端拒绝本帧）；EIO、ENXIO 等
/// 其他 I/O 错误不重试，由上层 `recover()` 处理。RP1/DesignWare 的地址 NACK、
/// SDA stuck 也可能返回 `EREMOTEIO`，此时同样按帧拒绝处理。
fn write_frame_with_retries<W: Write>(writer: &mut W, frame: &[u8]) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match writer.write_all(frame) {
            Ok(()) => return Ok(()),
            Err(e) if is_data_nack(&e) => {
                if attempt >= FRAME_RETRIES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "I2C 帧被接收端拒绝（长度或 CRC 校验失败，已重试 {FRAME_RETRIES} 次）"
                        ),
                    ));
                }
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// 校验状态读响应帧并返回状态字节。
///
/// 响应格式为 `状态字节 + CRC_L + CRC_H`，CRC 仅覆盖状态字节。
fn decode_status_frame(frame: &[u8; STATUS_FRAME_LEN]) -> io::Result<u8> {
    if !crc::verify_crc(frame) {
        let received = u16::from_le_bytes([frame[1], frame[2]]);
        let expected = crc::modbus_crc16(&frame[..1]);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("状态帧 CRC 校验失败: 接收 0x{received:04X}，计算 0x{expected:04X}"),
        ));
    }
    Ok(frame[0])
}

/// 重试整次状态读取：响应帧校验失败（`InvalidData`）时重新发请求并读取，
/// 最多重试 [`FRAME_RETRIES`] 次；其他 I/O 错误直接返回。
fn retry_status_read<F>(mut read_once: F) -> io::Result<u8>
where
    F: FnMut() -> io::Result<u8>,
{
    let mut attempt = 0;
    loop {
        match read_once() {
            Ok(status) => return Ok(status),
            Err(e) if e.kind() == io::ErrorKind::InvalidData && attempt < FRAME_RETRIES => {
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
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
        Ok(Self { file })
    }

    /// 发送一帧：控制字节 + 载荷长度 + 载荷 + CRC16。
    fn write_frame(&mut self, control: u8, payload: &[u8]) -> io::Result<()> {
        let mut buf = [0u8; FRAME_MAX];
        let len = build_frame(control, payload, &mut buf);
        let mut writer = &self.file;
        write_frame_with_retries(&mut writer, &buf[..len])
    }

    /// 发送状态读请求帧（`00 00 01 B0`），带 NACK 重试。
    fn send_status_request(&mut self) -> io::Result<()> {
        let mut request = [0u8; FRAME_MAX];
        let len = build_frame(0x00, &[], &mut request);
        let mut writer = &self.file;
        write_frame_with_retries(&mut writer, &request[..len])
    }

    /// 读取并校验 3 字节状态响应帧。
    ///
    /// 长度或 CRC 不符返回 `InvalidData`，供 [`retry_status_read`] 判定是否重试。
    fn read_status_response(&mut self) -> io::Result<u8> {
        let mut frame = [0u8; STATUS_FRAME_LEN];
        let n = self.file.read(&mut frame)?;
        if n != STATUS_FRAME_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("I2C 状态帧长度错误: 期望 {STATUS_FRAME_LEN} 字节，实际 {n} 字节"),
            ));
        }
        decode_status_frame(&frame)
    }
}

impl I2cDevice for I2cBus {
    /// 发送 I2C 命令（控制字节 0x00，末尾 Modbus CRC16）。
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "I2C 命令载荷不能为空：空命令帧与状态读请求（len=0）冲突",
            ));
        }
        if bytes.len() > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("I2C 命令过长: {} > {}", bytes.len(), MAX_PAYLOAD),
            ));
        }
        self.write_frame(0x00, bytes)
    }

    /// 发送 GDDRAM 数据（控制字节 0x40，末尾 Modbus CRC16）。
    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "I2C 数据载荷不能为空",
            ));
        }
        if bytes.len() > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("I2C 数据过长: {} > {}", bytes.len(), MAX_PAYLOAD),
            ));
        }
        self.write_frame(0x40, bytes)
    }

    /// 读取一个字节（SSD1309 状态寄存器）。
    ///
    /// 先发送状态读请求帧（`0x00 0x00` + CRC，线上 `00 00 01 B0`），再读取
    /// 3 字节响应 `状态字节 + CRC_L + CRC_H`。响应长度或 CRC 不符时重试整次
    /// 读取，最多重试 `FRAME_RETRIES` 次；其他 I/O 错误直接返回。
    ///
    /// SSD1306/1309 的 I2C 读序列要求先发送控制字节 0x00（命令模式）
    /// 选择状态寄存器，再进行读操作；两次独立事务在从机侧等效于
    /// 重复起始（实测可用）。
    fn read(&mut self) -> io::Result<u8> {
        retry_status_read(|| {
            self.send_status_request()?;
            self.read_status_response()
        })
    }
}

impl I2cDeviceFactory for I2cBus {
    fn open(bus_id: u8, addr: u8) -> io::Result<Self> {
        Self::open(bus_id, addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// 创建以普通临时文件充当底层设备的 I2cBus，仅用于验证线上字节。
    fn temp_bus(name: &str) -> (I2cBus, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "i2c_display_driver_{}_{}.tmp",
            std::process::id(),
            name
        ));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        (I2cBus { file }, path)
    }

    fn read_bytes(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    /// 预置文件内容，并把 I2cBus 持有的文件偏移复位到 0。
    fn seed(bus: &mut I2cBus, bytes: &[u8]) {
        use std::io::{Seek, SeekFrom, Write};
        bus.file.seek(SeekFrom::Start(0)).unwrap();
        bus.file.write_all(bytes).unwrap();
        bus.file.seek(SeekFrom::Start(0)).unwrap();
    }

    /// 模拟接收端 NACK 的写入器：前 n 次 write 返回 EREMOTEIO。
    struct NackWriter {
        nacks_left: usize,
        bytes: Vec<u8>,
        writes: usize,
    }

    impl NackWriter {
        fn new(nacks_left: usize) -> Self {
            Self {
                nacks_left,
                bytes: Vec::new(),
                writes: 0,
            }
        }
    }

    impl Write for NackWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.nacks_left > 0 {
                self.nacks_left -= 1;
                return Err(io::Error::from_raw_os_error(libc::EREMOTEIO));
            }
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_command_appends_length_and_modbus_crc() {
        let (mut bus, path) = temp_bus("write_command_crc");
        bus.write_command(&[0xAE]).unwrap();
        drop(bus);
        assert_eq!(read_bytes(&path), vec![0x00, 0x01, 0xAE, 0xF1, 0xEC]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn write_data_appends_length_and_modbus_crc() {
        let (mut bus, path) = temp_bus("write_data_crc");
        bus.write_data(&[0x01, 0x02]).unwrap();
        drop(bus);
        assert_eq!(read_bytes(&path), vec![0x40, 0x02, 0x01, 0x02, 0x34, 0x75]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_frames_match_protocol_vectors() {
        let (mut bus, path) = temp_bus("command_vectors");
        bus.write_command(&[0x81, 0xCF]).unwrap();
        bus.write_command(&[0x20, 0x02]).unwrap();
        bus.write_command(&[0xB0]).unwrap();
        bus.write_command(&[0x00, 0x10]).unwrap();
        drop(bus);
        assert_eq!(
            read_bytes(&path),
            vec![
                0x00, 0x02, 0x81, 0xCF, 0x81, 0xE0, // 对比度
                0x00, 0x02, 0x20, 0x02, 0x39, 0xE5, // 页寻址
                0x00, 0x01, 0xB0, 0x71, 0xE4, // 页地址
                0x00, 0x02, 0x00, 0x10, 0xA0, 0x28, // 列地址
            ]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn oversize_command_is_rejected_without_writing() {
        let (mut bus, path) = temp_bus("oversize_command");
        let payload = [0u8; MAX_PAYLOAD + 1];
        let err = bus.write_command(&payload).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        drop(bus);
        assert!(read_bytes(&path).is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn read_sends_request_and_verifies_response() {
        let (mut bus, path) = temp_bus("read_valid");
        seed(&mut bus, &[0x00, 0x00, 0x00, 0x00, 0x01, 0x7E, 0x80]);
        assert_eq!(bus.read().unwrap(), 0x01);
        drop(bus);
        assert_eq!(&read_bytes(&path)[..4], &[0x00, 0x00, 0x01, 0xB0]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn read_rejects_corrupted_status_frame() {
        let (mut bus, path) = temp_bus("read_bad_crc");
        seed(&mut bus, &[0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00]);
        let err = bus.read().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        drop(bus);
        assert_eq!(&read_bytes(&path)[..4], &[0x00, 0x00, 0x01, 0xB0]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn read_rejects_short_status_frame() {
        let (mut bus, path) = temp_bus("read_short");
        seed(&mut bus, &[0x00, 0x00, 0x00, 0x00, 0x01, 0x7E]);
        let err = bus.read().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn data_nack_detection_uses_eremoteio() {
        assert!(is_data_nack(&io::Error::from_raw_os_error(libc::EREMOTEIO)));
        assert!(!is_data_nack(&io::Error::other("bus error")));
    }

    #[test]
    fn write_frame_retries_after_nack() {
        let mut writer = NackWriter::new(FRAME_RETRIES);
        let frame = [0x00, 0x01, 0xAE, 0xF1, 0xEC];
        write_frame_with_retries(&mut writer, &frame).unwrap();
        assert_eq!(writer.writes, FRAME_RETRIES + 1);
        assert_eq!(writer.bytes, frame);
    }

    #[test]
    fn persistent_nack_reports_frame_rejected() {
        let mut writer = NackWriter::new(FRAME_RETRIES + 1);
        let err = write_frame_with_retries(&mut writer, &[0x00, 0x01, 0xAE]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(writer.writes, FRAME_RETRIES + 1);
        assert!(writer.bytes.is_empty());
    }

    #[test]
    fn write_frame_stops_on_non_nack_after_nack() {
        struct MixedWriter {
            calls: usize,
        }

        impl Write for MixedWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                if self.calls == 1 {
                    Err(io::Error::from_raw_os_error(libc::EREMOTEIO))
                } else {
                    Err(io::Error::from_raw_os_error(libc::EIO))
                }
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut writer = MixedWriter { calls: 0 };
        let err = write_frame_with_retries(&mut writer, &[0x00, 0x01, 0xAE]).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EIO));
        assert_eq!(writer.calls, 2);
    }

    #[test]
    fn empty_command_is_rejected_without_writing() {
        let (mut bus, path) = temp_bus("empty_command");
        let err = bus.write_command(&[]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        drop(bus);
        assert!(read_bytes(&path).is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn empty_data_is_rejected_without_writing() {
        let (mut bus, path) = temp_bus("empty_data");
        let err = bus.write_data(&[]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        drop(bus);
        assert!(read_bytes(&path).is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn max_payload_frame_is_259_bytes_with_valid_crc() {
        let (mut bus, path) = temp_bus("max_payload");
        bus.write_command(&[0x5A; MAX_PAYLOAD]).unwrap();
        drop(bus);
        let bytes = read_bytes(&path);
        assert_eq!(bytes.len(), FRAME_MAX);
        assert_eq!(bytes[0], 0x00);
        assert_eq!(bytes[1], MAX_PAYLOAD as u8);
        assert!(crate::display::crc::verify_crc(&bytes));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn retry_status_read_retries_invalid_data_then_succeeds() {
        let mut calls = 0;
        let status = retry_status_read(|| {
            calls += 1;
            if calls < 3 {
                Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame"))
            } else {
                Ok(0x01)
            }
        })
        .unwrap();
        assert_eq!(status, 0x01);
        assert_eq!(calls, 3);
    }

    #[test]
    fn retry_status_read_gives_up_after_retries() {
        let mut calls = 0;
        let err = retry_status_read(|| {
            calls += 1;
            Err::<u8, _>(io::Error::new(io::ErrorKind::InvalidData, "bad frame"))
        })
        .unwrap_err();
        assert_eq!(calls, FRAME_RETRIES + 1);
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn retry_status_read_does_not_retry_bus_errors() {
        let mut calls = 0;
        let err = retry_status_read(|| {
            calls += 1;
            Err::<u8, _>(io::Error::from_raw_os_error(libc::EIO))
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(err.raw_os_error(), Some(libc::EIO));
    }
}
