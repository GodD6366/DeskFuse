# uniDesk 路线图

本表区分当前源码能力与后续交付，不把计划标成可用功能。功能契约优先于历史里程碑描述。

## 当前开源预发行

- macOS arm64 主应用：设备发现和配对、剪贴板历史／同步、文件按需传输、缓存管理、文件暂存抽屉、独立键鼠共享工作区。
- 工程：workspace 分层、固定依赖图、macOS／Windows CI、ad-hoc DMG 与 GitHub prerelease。
- Windows：原生适配器、SCM 服务及受限只读客户端检查点，不提供完整安装包。

## 后续交付

1. 真实设备验收：Finder 文件／文件夹粘贴、抽屉拖出、权限变化、断线恢复、Wi-Fi 与有线两机输入。测试数据和结论必须对应具体构建。
2. Mac 正式分发：Developer ID、hardened runtime、公证及实际 Gatekeeper 验证。
3. Windows 完整产品：GUI/helper、会话和桌面边界、安装器、升级回滚及真实 SCM 验收。
4. 局域网更新：固定发行信任根、签名载荷与安装事务验证；当前 GitHub 下载不能替代该机制。
5. 维护性：按业务边界逐步拆分大型 UI、model 和 runtime bridge；不为开源首发做无关重写。

## 非首发范围

Linux、Intel Mac 发行包、跨公网输入、自动发布到应用商店、未验证的跨平台能力承诺。

设计以 [design/current](../design/current/README.md) 为准；协议与平台边界见 [ARCHITECTURE.md](ARCHITECTURE.md)。
