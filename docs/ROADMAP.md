# uniDesk 路线图

本表区分当前源码能力与后续交付，不把计划标成可用功能。功能契约优先于历史里程碑描述。

## 当前预发行

- macOS arm64 主应用：设备发现和配对、剪贴板历史／同步、文件按需传输、缓存管理、文件暂存抽屉、独立键鼠共享工作区。
- 工程：workspace 分层、固定依赖图、macOS／Windows CI、GitHub prerelease；公开包在配置凭据后走 Developer ID 与公证。
- Windows：发行实验性 x64 离线安装器；原生适配器、SCM 服务及跨平台行为仍分阶段验收。

## 后续交付

1. 真实设备验收：Finder 文件／文件夹粘贴、抽屉拖出、权限变化、断线恢复、Wi-Fi 与有线两机输入。测试数据和结论必须对应具体构建。
2. Mac 正式分发：在真实 Gatekeeper 下验证已公证预发行的首次启动；贡献者／main bootstrap 仍可 ad-hoc。
3. Windows 完整产品：GUI/helper、会话和桌面边界、安装器、升级回滚及真实 SCM 验收。
4. 局域网更新：固定发行信任根、签名载荷与安装事务验证；当前 GitHub 下载不能替代该机制。
5. 维护性：按业务边界逐步拆分大型 UI、model 和 runtime bridge；不为开源首发做无关重写。

## 非首发范围

Linux、Intel Mac 发行包、跨公网输入、自动发布到应用商店、未验证的跨平台能力承诺。

界面以当前实际 UI 和用户确认修改为准，行为约定见 [design/current](../design/current/README.md)；协议与平台边界见 [ARCHITECTURE.md](ARCHITECTURE.md)。
