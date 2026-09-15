# uniDesk TODO

> 用可勾选项跟踪公开发布与后续交付。勾选只表示动作完成，不表示真实设备或视觉验收通过；功能契约以 `docs/` 与 `design/current/` 为准。

## 开源发布

- [x] 清理 Git 历史中的设计稿、迭代产物与本机私有引用
- [x] 仓库设为 public，并确认漏洞报告入口
- [x] 移除 `scripts/probe-finder-*.{sh,py}` 本机诊断脚本并加入忽略清单
- [ ] 明确剪贴板库语义（库 id 是否即历史 id、未知远程来源是否入库），恢复被删除的 4 个剪贴板历史用例
- [ ] 明确路由失败回退与禁用控制器的元数据链语义，恢复被删除的 2 个 `unidesk-runtime` 用例
- [ ] 收紧 `unidesk-service` 的实验性 clippy allow，改为真实修复
- [ ] 发布 `v0.1.0` GitHub 预发行，触发 `cask` job 固定 Homebrew 校验和
- [ ] 在一台干净 Mac 上按 README 验证 `brew trust` / `tap` / `install --cask` / `xattr` 全流程
- [ ] 补充真实界面截图到 README，替换“待补”说明

## 分发与签名

- [ ] 接入 Developer ID 与 hardened runtime
- [ ] Apple 公证，并在真实 Gatekeeper 下验证首次启动
- [ ] 从 README 与发行说明移除 ad-hoc、未公证的临时说明

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

## 工程维护

- [ ] 按业务边界拆分大型 UI / model / runtime bridge 文件
- [ ] 处理 Dependabot 的 Rust 与 Actions 依赖更新
- [ ] 扩展剪贴板库、文件传输契约与 cask 渲染的自动化测试

## 暂不计划

- Linux 客户端、Intel Mac 发行包、跨公网输入、应用商店上架
