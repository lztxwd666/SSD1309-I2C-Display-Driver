//! 测试用 Mock I2C 总线（仅 `#[cfg(test)]` 编译）。
//!
//! 记录所有写入（含 0x00/0x40 控制字节），并支持按需注入失败，
//! 用于无硬件验证驱动命令序列与恢复流程。

use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;
use std::sync::Mutex;

use super::i2c_bus::{I2cDevice, I2cDeviceFactory};

/// 一次 I2C 写入记录（含控制字节）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Write {
    /// 控制字节（0x00=命令，0x40=数据）。
    pub control: u8,
    /// 载荷。
    pub bytes: Vec<u8>,
}

/// 工厂状态：下次 open 使用的日志句柄与失败次数。
///
/// 仅用于恢复流程测试（工厂创建的总线默认使用独立新日志）。
/// 单测串行使用，避免并行冲突。
static FACTORY: Mutex<Option<(Rc<RefCell<Vec<Write>>>, usize)>> = Mutex::new(None);

/// 记录型 Mock I2C 总线。
pub(crate) struct MockBus {
    log: Rc<RefCell<Vec<Write>>>,
    /// 剩余失败写入次数（通过共享 Cell 可在持有总线后修改）。
    failures: Rc<Cell<usize>>,
}

impl MockBus {
    /// 创建 MockBus，日志与失败计数通过共享句柄由测试持有。
    pub(crate) fn new(log: Rc<RefCell<Vec<Write>>>, failures: Rc<Cell<usize>>) -> Self {
        Self { log, failures }
    }

    /// 设置工厂下一次 open 创建的 MockBus 的日志句柄与失败次数。
    pub(crate) fn set_factory(log: Rc<RefCell<Vec<Write>>>, failures: usize) {
        *FACTORY.lock().unwrap() = Some((log, failures));
    }

    fn write(&mut self, control: u8, bytes: &[u8]) -> io::Result<()> {
        let n = self.failures.get();
        if n > 0 {
            self.failures.set(n - 1);
            return Err(io::Error::new(io::ErrorKind::Other, "mock: 模拟 I2C 写入失败"));
        }
        self.log.borrow_mut().push(Write {
            control,
            bytes: bytes.to_vec(),
        });
        Ok(())
    }
}

impl I2cDevice for MockBus {
    fn write_command(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write(0x00, bytes)
    }

    fn write_data(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write(0x40, bytes)
    }
}

impl I2cDeviceFactory for MockBus {
    fn open(_bus_id: u8, _addr: u8) -> io::Result<Self> {
        let (log, failures) = FACTORY
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| (Rc::new(RefCell::new(Vec::new())), 0));
        Ok(Self {
            log,
            failures: Rc::new(Cell::new(failures)),
        })
    }
}
