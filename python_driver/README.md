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

详细示例见 `examples/`。

## 错误处理

驱动统一使用以下异常：

- `DriverError`：所有驱动错误的基类
- `I2cError`：I2C 打开/读取/写入错误
- `NotInitializedError`：驱动未初始化
- `InvalidDataError`：传入数据不合法或长度不足

示例：

```python
from ssd1309_driver import DriverError, I2cError

try:
    display.render()
except I2cError as e:
    print(f"I2C 错误: {e}")
except DriverError as e:
    print(f"驱动错误: {e}")
```

## 完整接口示例

`examples/usage.py` 覆盖了全部公共接口，包括：

- Display / DisplayBuilder 初始化
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
