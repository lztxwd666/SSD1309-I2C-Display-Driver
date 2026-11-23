# Python 驱动测试

在 Linux 上，从仓库根目录按顺序执行。

1. 进入 Python 驱动目录，创建并激活虚拟环境。

   ```bash
   cd python_driver
   python3 -m venv .venv
   source .venv/bin/activate.fish
   ```

2. 安装当前目录的驱动。

   ```bash
   python -m pip install -e .
   ```

3. 运行单元测试。无需连接屏幕，覆盖 v3 回执重读/重发/会话失效、Display 恢复等错误路径；全部通过时末尾显示 `OK`。

   ```bash
   python -m unittest discover -s tests -v
   ```

4. 连接 v3 接收端和 SSD1309 屏幕。示例使用总线 1、地址 `0x3C`，运行账号需要对 `/dev/i2c-1` 有读写权限。

   ```bash
   python examples/usage.py                  # 测试常用接口、状态读取和恢复
   python examples/demo_all.py               # 测试绘图、局部推帧和滚动
   python examples/boot_animation.py classic # 测试开机动画
   ```

5. 测试结束后退出虚拟环境。

   ```bash
   deactivate
   ```
