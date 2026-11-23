//! Linux I2C 发送端，使用独立写事务和读事务实现 v3 帧协议。
//!
//! 写帧含长度、序号与 Modbus CRC16。主机验证接收端的固定长度回执，
//! 非查询帧的匹配回执还要求 `value` 为 0；确认成功后推进序号。
//! 帧格式见 `docs/i2c-crc-protocol.md`。

use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};

use super::crc;

const I2C_SLAVE: libc::Ioctl = 0x0703;
const I2C_TIMEOUT: libc::Ioctl = 0x0702;
const T_IO_10MS: libc::c_ulong = 10;

const COMMAND: u8 = 0x00;
const DATA: u8 = 0x40;
const PANEL_STATUS: u8 = 0x01;
const SYNC: u8 = 0x7F;
const MAX_PAYLOAD: usize = 255;
const MAX_FRAME: usize = MAX_PAYLOAD + 6;
const RECEIPT_LEN: usize = 6;
const LAST_OK_VALID: u8 = 0x80;
const BUSY: u8 = 0x40;

const T_PUBLISH: Duration = Duration::from_millis(5);
const T_RESULT: Duration = Duration::from_millis(100);
const T_POLL: Duration = Duration::from_millis(2);
const T_FRAME: Duration = Duration::from_millis(500);

/// I2C 设备抽象，提供命令写入、显示数据写入和面板状态读取。
///
/// 帧封装由传输实现负责。写/读调用返回 `Ok` 表示接收端已通过链路回执
/// 确认该帧；`InvalidData` 表示可安全跳过的帧拒绝。会话失效或结果未知时，
/// 应返回其他错误类型，使上层能够恢复连接。
pub trait I2cDevice {
    /// 发送 SSD1309 命令。
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// 发送 GDDRAM 数据。
    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// 读取 SSD1309 的一个状态字节。
    fn read(&mut self) -> io::Result<u8>;
}

/// I2C 设备工厂，供显示器初始化和恢复使用。
///
/// 返回前须完成 v3 会话同步（发送 `SYNC` 并确认）；显示器随后直接发送初始化命令。
pub trait I2cDeviceFactory: Sized {
    /// 打开指定总线上的设备并建立 v3 会话。
    fn open(bus_id: u8, addr: u8) -> io::Result<Self>;
}

/// Linux I2C 总线。每个设备地址应由一个 v3 会话独占。
pub struct I2cBus {
    file: fs::File,
    next_seq: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    NeedSync,
    Synced,
    Pending,
    Ok,
    CrcError,
    FormatError,
    BusyDrop,
    RxOverflow,
    RxTimeout,
    OutOfSequence,
    DownstreamUnknown,
}

impl Status {
    fn decode(code: u8) -> io::Result<Self> {
        match code {
            0x00 => Ok(Self::NeedSync),
            0x01 => Ok(Self::Synced),
            0x02 => Ok(Self::Pending),
            0x03 => Ok(Self::Ok),
            0x04 => Ok(Self::CrcError),
            0x05 => Ok(Self::FormatError),
            0x06 => Ok(Self::BusyDrop),
            0x07 => Ok(Self::RxOverflow),
            0x08 => Ok(Self::RxTimeout),
            0x09 => Ok(Self::OutOfSequence),
            0x0A => Ok(Self::DownstreamUnknown),
            _ => Err(io::Error::other(format!(
                "I2C v3 回执状态码未知: 0x{code:02X}，结果未知"
            ))),
        }
    }

    fn requires_sync(self) -> bool {
        matches!(
            self,
            Self::NeedSync | Self::OutOfSequence | Self::DownstreamUnknown
        )
    }

    fn permits_early_retry(self, syncing: bool) -> bool {
        matches!(
            self,
            Self::CrcError | Self::FormatError | Self::BusyDrop | Self::RxTimeout
        ) || (syncing && self.requires_sync())
    }
}

fn build_frame(kind: u8, seq: u16, payload: &[u8], out: &mut [u8; MAX_FRAME]) -> usize {
    let body_len = 4 + payload.len();
    out[0] = kind;
    out[1] = payload.len() as u8;
    out[2..4].copy_from_slice(&seq.to_le_bytes());
    out[4..body_len].copy_from_slice(payload);
    crc::append_crc(out, body_len)
}

fn exchange(
    bus: &mut (impl Read + Write),
    frame: &[u8],
    seq: Option<u16>,
    elapsed: impl Fn() -> Duration,
    mut sleep: impl FnMut(Duration),
) -> io::Result<u8> {
    let syncing = seq.is_none();
    let mut detail = String::from("未读取到回执");
    while elapsed() < T_FRAME {
        // 每次 write 对应一笔独立事务，短写后不得发送剩余字节。
        match bus.write(frame) {
            Ok(count) if count == frame.len() => {}
            Ok(count) => detail = format!("短写 {count}/{}", frame.len()),
            Err(error) => detail = format!("写事务失败: {error}"),
        }
        let written_at = elapsed();
        if syncing && written_at < T_FRAME {
            sleep(T_PUBLISH.min(T_FRAME - written_at));
        }

        let mut busy = false;
        let mut failed_reads = 0;
        while elapsed() < T_FRAME {
            let mut reply = [0; RECEIPT_LEN];
            let result = bus.read(&mut reply);
            let bus_readable = matches!(&result, Ok(count) if *count > 0);
            let now = elapsed();
            if now >= T_FRAME {
                break;
            }
            let mut retry_after = T_RESULT;
            match result {
                Ok(RECEIPT_LEN) if crc::verify_crc(&reply) => {
                    failed_reads = 0;
                    let flags = reply[0];
                    let status = Status::decode(flags & 0x3F)?;
                    let last_ok_valid = flags & LAST_OK_VALID != 0;
                    busy = flags & BUSY != 0;
                    if (!last_ok_valid && (status == Status::Ok || reply[1..4] != [0; 3]))
                        || (status == Status::Synced && last_ok_valid)
                    {
                        return Err(io::Error::other("I2C v3 回执字段不一致，结果未知"));
                    }
                    if !syncing && status.requires_sync() {
                        return Err(io::Error::other(format!(
                            "I2C v3 会话失效（{status:?}），结果未知，须重新打开并初始化面板"
                        )));
                    }
                    let last_ok_seq = u16::from_le_bytes([reply[1], reply[2]]);
                    if last_ok_valid && seq == Some(last_ok_seq) {
                        if frame[0] != PANEL_STATUS && reply[3] != 0 {
                            return Err(io::Error::other(
                                "I2C v3 非查询帧回执的 value 非零，结果未知",
                            ));
                        }
                        return Ok(reply[3]);
                    }
                    if syncing && status == Status::Synced && !busy {
                        return Ok(0);
                    }
                    if status.permits_early_retry(syncing) {
                        retry_after = T_PUBLISH;
                    }
                    detail = format!("回执 {status:?}，未确认当前操作");
                }
                other => {
                    failed_reads += 1;
                    detail = match other {
                        Ok(RECEIPT_LEN) => "回执 CRC 错误".into(),
                        Ok(count) => format!("回执短读 {count}/{RECEIPT_LEN}"),
                        Err(error) => format!("读事务失败: {error}"),
                    };
                }
            }

            // 坏回执保留已知的 BUSY 状态。读事务恢复返回字节后才可重发。
            sleep(T_POLL.min(T_FRAME - now));
            if bus_readable && !busy && failed_reads != 1 && now - written_at >= retry_after {
                break;
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("I2C v3 确认超时，结果未知: {detail}"),
    ))
}

fn exchange_frame(
    bus: &mut (impl Read + Write),
    kind: u8,
    seq: Option<u16>,
    payload: &[u8],
) -> io::Result<u8> {
    let mut frame = [0; MAX_FRAME];
    let len = build_frame(kind, seq.unwrap_or(0), payload, &mut frame);
    let start = Instant::now();
    exchange(bus, &frame[..len], seq, || start.elapsed(), thread::sleep)
}

impl I2cBus {
    /// 打开 Linux I2C 设备，设置适配器超时并同步 v3 会话。
    ///
    /// `I2C_TIMEOUT` 为适配器级设置，关闭连接不会恢复原值；调用方应独占该设备地址。
    ///
    /// ```no_run
    /// use i2c_display_driver::display::{I2cBus, I2cDevice};
    ///
    /// let mut bus = I2cBus::open(1, 0x3C)?;
    /// let status = bus.read()?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn open(bus: u8, addr: u8) -> io::Result<Self> {
        let path = format!("/dev/i2c-{bus}");
        let mut file = fs::OpenOptions::new().read(true).write(true).open(path)?;
        // SAFETY: fd 在调用期间有效，I2C_SLAVE 和 I2C_TIMEOUT 均接收整数参数。
        unsafe {
            if libc::ioctl(file.as_raw_fd(), I2C_SLAVE, libc::c_ulong::from(addr)) < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(file.as_raw_fd(), I2C_TIMEOUT, T_IO_10MS) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        exchange_frame(&mut file, SYNC, None, &[])?;
        Ok(Self {
            file,
            next_seq: Some(1),
        })
    }

    fn send(&mut self, kind: u8, payload: &[u8]) -> io::Result<u8> {
        if kind != PANEL_STATUS && !(1..=MAX_PAYLOAD).contains(&payload.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "I2C 载荷长度必须为 1–{MAX_PAYLOAD} 字节，实际 {}",
                    payload.len()
                ),
            ));
        }
        // 未确认的操作结束当前会话，后续调用必须通过重新打开来同步。
        let seq = self
            .next_seq
            .take()
            .ok_or_else(|| io::Error::other("I2C v3 会话失效，须重新打开并初始化面板"))?;
        let value = exchange_frame(&mut self.file, kind, Some(seq), payload)?;
        self.next_seq = Some(seq.wrapping_add(1));
        Ok(value)
    }
}

impl I2cDevice for I2cBus {
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send(COMMAND, bytes).map(drop)
    }

    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send(DATA, bytes).map(drop)
    }

    fn read(&mut self) -> io::Result<u8> {
        self.send(PANEL_STATUS, &[])
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
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    const SYNC_FRAME: &[u8] = &[0x7F, 0x00, 0x00, 0x00, 0x19, 0xF0];
    const SYNCED: &[u8] = &[0x01, 0x00, 0x00, 0x00, 0x01, 0xD8];
    const COMMAND_FRAME: &[u8] = &[0x00, 0x01, 0x01, 0x00, 0xAE, 0xF5, 0x80];
    const OK_COMMAND: &[u8] = &[0x83, 0x01, 0x00, 0x00, 0x78, 0x60];
    const DATA_FRAME: &[u8] = &[0x40, 0x02, 0x02, 0x00, 0x01, 0x02, 0xF6, 0xF2];
    const OK_DATA: &[u8] = &[0x83, 0x02, 0x00, 0x00, 0x88, 0x60];
    const CRC_ERROR: &[u8] = &[0x84, 0x01, 0x00, 0x00, 0x79, 0x14];
    const BUSY_DROP: &[u8] = &[0xC6, 0x01, 0x00, 0x00, 0x6D, 0x6C];
    const NEED_SYNC: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00, 0x24];

    enum Step<'a> {
        Write(&'a [u8], io::Result<usize>),
        Read(&'a [u8], Duration),
        ReadError(io::ErrorKind, Duration),
    }

    struct Script<'a> {
        steps: VecDeque<Step<'a>>,
        time: &'a Cell<Duration>,
    }

    impl Read for Script<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert_eq!(buf.len(), RECEIPT_LEN);
            match self.steps.pop_front() {
                Some(Step::Read(reply, delay)) => {
                    self.time.set(self.time.get() + delay);
                    buf[..reply.len()].copy_from_slice(reply);
                    Ok(reply.len())
                }
                Some(Step::ReadError(kind, delay)) => {
                    self.time.set(self.time.get() + delay);
                    Err(kind.into())
                }
                _ => panic!("意外的读事务"),
            }
        }
    }

    impl Write for Script<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let Some(Step::Write(expected, result)) = self.steps.pop_front() else {
                panic!("意外的写事务");
            };
            assert_eq!(buf, expected);
            result
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn run(steps: Vec<Step<'_>>, frame: &[u8], seq: Option<u16>) -> (io::Result<u8>, Duration) {
        let time = Cell::new(Duration::ZERO);
        let mut script = Script {
            steps: steps.into(),
            time: &time,
        };
        let result = exchange(
            &mut script,
            frame,
            seq,
            || time.get(),
            |d| time.set(time.get() + d),
        );
        assert!(script.steps.is_empty());
        (result, time.get())
    }

    #[test]
    fn frames_match_protocol_vectors() {
        let mut buf = [0; MAX_FRAME];
        for (kind, seq, payload, expected) in [
            (SYNC, 0, &[][..], SYNC_FRAME),
            (COMMAND, 1, &[0xAE][..], COMMAND_FRAME),
            (DATA, 2, &[0x01, 0x02][..], DATA_FRAME),
            (
                PANEL_STATUS,
                3,
                &[][..],
                &[0x01, 0x00, 0x03, 0x00, 0x01, 0x28],
            ),
        ] {
            let len = build_frame(kind, seq, payload, &mut buf);
            assert_eq!(&buf[..len], expected);
        }
        let len = build_frame(DATA, 0x1234, &[0x55; MAX_PAYLOAD], &mut buf);
        assert_eq!(len, 261);
        assert_eq!(&buf[..4], &[DATA, 255, 0x34, 0x12]);
        assert!(crc::verify_crc(&buf));
    }

    #[test]
    fn sync_waits_for_publication_and_can_retry_after_need_sync() {
        let (result, elapsed) = run(
            vec![
                Step::Write(SYNC_FRAME, Ok(SYNC_FRAME.len())),
                Step::Read(NEED_SYNC, Duration::ZERO),
                Step::Write(SYNC_FRAME, Ok(SYNC_FRAME.len())),
                Step::Read(SYNCED, Duration::ZERO),
            ],
            SYNC_FRAME,
            None,
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(elapsed, T_PUBLISH * 2 + T_POLL);
    }

    #[test]
    fn sync_waits_for_idle_before_accepting_synced_snapshot() {
        let mut busy_synced = [0x41, 0, 0, 0, 0, 0];
        crc::append_crc(&mut busy_synced, 4);
        let (result, elapsed) = run(
            vec![
                Step::Write(SYNC_FRAME, Ok(SYNC_FRAME.len())),
                Step::Read(&busy_synced, Duration::ZERO),
                Step::Read(SYNCED, Duration::ZERO),
            ],
            SYNC_FRAME,
            None,
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(elapsed, T_PUBLISH + T_POLL);
    }

    #[test]
    fn timeouts_short_reads_and_bad_crc_read_again_without_resending() {
        let mut bad_crc = OK_COMMAND.to_vec();
        bad_crc[5] ^= 1;
        let (result, elapsed) = run(
            vec![
                Step::Write(COMMAND_FRAME, Err(io::ErrorKind::TimedOut.into())),
                Step::Read(&OK_COMMAND[..3], Duration::ZERO),
                Step::Read(&bad_crc, Duration::ZERO),
                Step::Read(OK_COMMAND, Duration::ZERO),
            ],
            COMMAND_FRAME,
            Some(1),
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(elapsed, T_POLL * 2);
    }

    #[test]
    fn short_write_replays_whole_frame_only_after_result_window() {
        let (result, elapsed) = run(
            vec![
                Step::Write(COMMAND_FRAME, Ok(2)),
                Step::Read(&OK_COMMAND[..3], T_RESULT),
                Step::Read(&OK_COMMAND[..3], Duration::ZERO),
                Step::Write(COMMAND_FRAME, Ok(COMMAND_FRAME.len())),
                Step::Read(OK_COMMAND, Duration::ZERO),
            ],
            COMMAND_FRAME,
            Some(1),
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(elapsed, T_RESULT + T_POLL * 2);
    }

    #[test]
    fn replay_waits_until_read_returns_bytes_after_transport_errors() {
        let (result, elapsed) = run(
            vec![
                Step::Write(COMMAND_FRAME, Err(io::ErrorKind::TimedOut.into())),
                Step::ReadError(io::ErrorKind::TimedOut, T_RESULT),
                Step::ReadError(io::ErrorKind::TimedOut, Duration::ZERO),
                Step::Read(&[], Duration::ZERO),
                Step::Read(&OK_COMMAND[..3], Duration::ZERO),
                Step::Write(COMMAND_FRAME, Ok(COMMAND_FRAME.len())),
                Step::Read(OK_COMMAND, Duration::ZERO),
            ],
            COMMAND_FRAME,
            Some(1),
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(elapsed, T_RESULT + T_POLL * 4);
    }

    #[test]
    fn persistent_read_failure_expires_without_replay() {
        let timeout = io::ErrorKind::TimedOut;
        let (result, elapsed) = run(
            vec![
                Step::Write(COMMAND_FRAME, Err(timeout.into())),
                Step::ReadError(timeout, T_RESULT),
                Step::ReadError(timeout, T_RESULT),
                Step::ReadError(timeout, T_RESULT),
                Step::ReadError(timeout, T_RESULT),
                Step::ReadError(timeout, T_RESULT - T_POLL * 4),
            ],
            COMMAND_FRAME,
            Some(1),
        );
        assert_eq!(elapsed, T_FRAME);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn crc_rejection_retries_same_frame_after_publication() {
        let (result, elapsed) = run(
            vec![
                Step::Write(DATA_FRAME, Ok(DATA_FRAME.len())),
                Step::Read(OK_COMMAND, Duration::ZERO),
                Step::Read(CRC_ERROR, Duration::ZERO),
                Step::Read(CRC_ERROR, T_PUBLISH),
                Step::Write(DATA_FRAME, Ok(DATA_FRAME.len())),
                Step::Read(OK_DATA, Duration::ZERO),
            ],
            DATA_FRAME,
            Some(2),
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(elapsed, T_PUBLISH + T_POLL * 3);
    }

    #[test]
    fn known_busy_survives_unreadable_receipts_until_deadline() {
        let (result, elapsed) = run(
            vec![
                Step::Write(DATA_FRAME, Ok(DATA_FRAME.len())),
                Step::Read(BUSY_DROP, Duration::ZERO),
                Step::Read(&[], T_RESULT),
                Step::Read(OK_DATA, T_FRAME - T_RESULT - T_POLL * 2),
            ],
            DATA_FRAME,
            Some(2),
        );
        assert_eq!(elapsed, T_FRAME);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn protocol_failure_takes_priority_over_matching_sequence() {
        for (flags, sequence) in [
            (0x81, 2),
            (0x03, 0),
            (0x04, 2),
            (0x89, 2),
            (0x8A, 2),
            (0xBF, 2),
        ] {
            let mut reply = [flags, sequence, 0, 0, 0, 0];
            crc::append_crc(&mut reply, 4);
            let (result, elapsed) = run(
                vec![
                    Step::Write(DATA_FRAME, Ok(DATA_FRAME.len())),
                    Step::Read(&reply, Duration::ZERO),
                ],
                DATA_FRAME,
                Some(2),
            );
            assert_eq!(elapsed, Duration::ZERO);
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
        }
    }

    #[test]
    fn nonquery_receipt_with_matching_sequence_requires_zero_value() {
        let mut reply = [0x83, 2, 0, 0x81, 0, 0];
        crc::append_crc(&mut reply, 4);
        let (result, _) = run(
            vec![
                Step::Write(DATA_FRAME, Ok(DATA_FRAME.len())),
                Step::Read(&reply, Duration::ZERO),
            ],
            DATA_FRAME,
            Some(2),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
    }

    #[test]
    fn stale_panel_status_value_does_not_invalidate_current_write() {
        let mut reply = [0x83, 1, 0, 0x81, 0, 0];
        crc::append_crc(&mut reply, 4);
        let (result, _) = run(
            vec![
                Step::Write(DATA_FRAME, Ok(DATA_FRAME.len())),
                Step::Read(&reply, Duration::ZERO),
                Step::Read(OK_DATA, Duration::ZERO),
            ],
            DATA_FRAME,
            Some(2),
        );
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn public_calls_validate_payload_wrap_sequence_and_end_failed_session() {
        let (host, mut peer) = UnixStream::pair().unwrap();
        let host: OwnedFd = host.into();
        let mut bus = I2cBus {
            file: host.into(),
            next_seq: Some(u16::MAX),
        };
        let peer = thread::spawn(move || {
            let mut frame = [0; 7];
            peer.read_exact(&mut frame).unwrap();
            assert_eq!(&frame[..5], &[COMMAND, 1, 0xFF, 0xFF, 0xAE]);
            assert!(crc::verify_crc(&frame));
            let mut receipt = [0x83, 0xFF, 0xFF, 0, 0, 0];
            crc::append_crc(&mut receipt, 4);
            peer.write_all(&receipt).unwrap();

            peer.read_exact(&mut frame[..6]).unwrap();
            assert_eq!(&frame[..4], &[PANEL_STATUS, 0, 0, 0]);
            assert!(crc::verify_crc(&frame[..6]));
            receipt = [0x83, 0, 0, 0x81, 0, 0];
            crc::append_crc(&mut receipt, 4);
            peer.write_all(&receipt).unwrap();

            peer.read_exact(&mut frame).unwrap();
            assert_eq!(frame, COMMAND_FRAME);
            peer.write_all(NEED_SYNC).unwrap();
            assert_eq!(peer.read(&mut frame).unwrap(), 0);
        });

        assert_eq!(
            bus.write_command(&[]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            bus.write_data(&[0; 256]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        bus.write_command(&[0xAE]).unwrap();
        assert_eq!(bus.read().unwrap(), 0x81);
        assert_eq!(
            bus.write_command(&[0xAE]).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(
            bus.write_data(&[0x00]).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        drop(bus);
        peer.join().unwrap();
    }
}
