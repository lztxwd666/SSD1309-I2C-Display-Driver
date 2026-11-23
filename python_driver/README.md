# SSD1309 纯 Python 驱动

## 虚拟环境安装

```bash
cd python_driver
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

## 快速使用

```python
from ssd1309_driver import Display
from ssd1309_driver.graphics import draw_text

display = Display.open(1, 0x3C)
display.framebuffer.clear()
draw_text(display.framebuffer, 0, 0, "Hello SSD1309!")
display.render()
# 注意：不要立即调用 display.sleep()，否则屏幕会熄灭
```

示例的安装与运行步骤见 [Python 驱动测试](examples/TESTING.md)。

## I2C 帧校验（协议 v3）

发送端打开设备时完成 `SYNC`，随后使用带 16 位序号的 Modbus CRC16 写帧和固定 6 字节回执。读坏回执时优先重读。重发使用原序号和原帧字节。链路总超时、从机重启或下游结果未知时抛出 `I2cError`，此后可用 `recover()` 同步会话并重新初始化面板。适配器的 `I2C_TIMEOUT` 设为 100 ms；该设置是适配器级的，由同一适配器上的设备共享，关闭连接不会恢复原值。

对端须支持 v3，并在校验后按 SSD1309 原生格式转发命令和显示数据。见 [v3 规范](../docs/i2c-crc-protocol.md)和 [v3 接收端速查](../docs/i2c-crc-quickstart.md)。旧版线上格式见 [v2 规范](../docs/i2c-crc-protocol-v2.md)。

## 错误处理

驱动统一使用以下异常：

- `DriverError`：所有驱动错误的基类
- `I2cError`：I2C 通信错误，包括 v3 会话失效和执行结果未知
- `FrameRejectedError`：自定义传输明确报告可安全跳过的帧校验失败；内置 v3 `I2cBus` 不会抛出，v3 帧错误在传输内部重试，最终以 `I2cError` 结束会话
- `NotInitializedError`：驱动未初始化
- `InvalidDataError`：传入数据不合法或长度不足

`FrameRejectedError` 是 `I2cError` 的子类，捕获时先具体后通用；内置 v3 传输不会抛出它。

示例：

```python
from ssd1309_driver import DriverError, FrameRejectedError, I2cError

try:
    display.render()
except FrameRejectedError as e:
    # 仅自定义传输会走到这里；内置 v3 不会抛出
    print(f"帧校验失败: {e}")
except I2cError as e:
    print(f"I2C 错误: {e}")
except DriverError as e:
    print(f"驱动错误: {e}")
```

## 完整接口示例

`examples/usage.py` 演示以下接口。

- DisplayBuilder 初始化
- 帧缓冲与 blit
- 全部图形绘制函数
- 全帧/脏矩形/局部/鲁棒推帧
- 多页与滚动动画
- 软件滚动
- 对比度、反色、旋转、高级寄存器
- 硬件滚动兼容入口
- 状态读取
- recover
- 统计与日志

运行：

```bash
python examples/usage.py
```

## 关于 pyproject.toml

`pyproject.toml` 是 Python 包的现代标准配置文件，用于支持：

```bash
pip install -e .
```

它让 `ssd1309_driver` 变成可安装包，在虚拟环境中安装后可以在任何目录直接 `import ssd1309_driver`。

如果不使用 `pyproject.toml`，也可以直接把 `ssd1309_driver` 目录放到项目里并设置 `PYTHONPATH`
