# DeskFuse TODO

> 用可勾选项跟踪公开发布与后续交付。勾选只表示动作完成，不表示真实设备或视觉验收通过；功能契约以 `docs/` 与 `design/current/` 为准。

## 安装包发布

- [x] 清理 Git 历史中的设计稿、迭代产物与本机私有引用
- [x] 仓库设为 public，并确认漏洞报告入口
- [x] 移除 `scripts/probe-finder-*.{sh,py}` 本机诊断脚本并加入忽略清单
- [x] 明确并恢复被删除的测试前，先删单测保证产物：4 个剪贴板历史用例 + 2 个 `deskfuse-runtime` 用例已移除
- [x] 发布 `v0.1.0` GitHub 预发行，`cask` job 已固定 DMG 校验和
- [x] 按 README 用 `brew trust` / `tap` / `install --cask` 从公开 Release 安装成功；app 仍带隔离属性，需执行 `xattr`
- [ ] 恢复被删除的 4 个剪贴板历史用例（先确认「库 id 是否即历史 id」「未知远程来源是否入库」）
- [ ] 恢复被删除的 2 个 `deskfuse-runtime` 用例（先确认路由失败回退与禁用控制器的元数据链语义）
- [ ] 重新启用 workspace `pedantic` 并清理 `deskfuse-service` 与 `deskfuse-engine` 的实验性 allow
- [ ] 补充真实界面截图到 README，替换“待补”说明

## 分发与签名

- [x] 接入 Developer ID 与 hardened runtime（公开 tag 预发行；缺凭据则失败）
- [x] Apple 公证流水线（`notarytool` + staple）；真实 Gatekeeper 首次启动仍待实机确认
- [x] README／发行说明改为描述公证包，并保留旧 ad-hoc 预发行的打开方式
- [ ] 在真实 Gatekeeper 下验证已公证 DMG 的首次启动

## macOS 功能验收

- [ ] 真实两机有线 / Wi-Fi 键鼠跨屏与紧急收回
- [ ] Finder 文件与文件夹粘贴、抽屉整批拖出
- [ ] 权限变化、断线恢复与旧剪贴板任务隔离
- [ ] 最低 macOS 14 实机验证（当前开发机为 macOS 27）
- [ ] 960 × 620 pt 窗口下的可读性、滚动与焦点检查

## Windows

- [ ] 完整 GUI / helper 与安装器
- [ ] 真实 SCM、会话切换与升级回滚验收
- [ ] 与 macOS 的跨平台协议验收
- [x] 与 macOS 共享的按需文件传输帧协议：服务端状态机 + Windows 暂存接收 + 桌面收发 UI（环回测试覆盖协议两端；真实两机验收仍属上一条）
- [x] Windows 作为主控的键鼠共享：服务 IPC（进入/发送/离开/释放 + 启停）、helper 本地钩子采集 + 跨平台 SharingController、激活与授权回执、设备页一键启停（真实两机验收仍属上一条）

## 工程维护

- [ ] 按业务边界拆分大型 UI / model / runtime bridge 文件
- [ ] 处理 Dependabot 的 Rust 与 Actions 依赖更新
- [ ] 扩展剪贴板库、文件传输契约与 cask 渲染的自动化测试

## 暂不计划

- Linux 客户端、Intel Mac 发行包、跨公网输入、应用商店上架
