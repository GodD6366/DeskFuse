# Changelog

版本来自根 `Cargo.toml`；源码变化不代表已发布或真实设备验收通过。

## [Unreleased]

## [0.4.1] — 授权后可重启

- 启动授权弹窗显示期间，不再拦截应用退出。系统设置授权后的「退出并重新打开」可以结束 DeskFuse 并由系统重新打开。

发行边界：公开预发行提供 macOS arm64 与 x86_64 DMG；Windows 构建暂跳过。签名、公证与包完整性由发布流水线验证。系统设置里的重启按钮需在安装包上实机确认。

## [0.4.0] — DeskFuse 独立产品

- 产品、源码目录、Rust crate、Swift 模块、Windows 服务及安装资产统一使用 DeskFuse 命名。
- 使用 `com.deskfuse.app` 身份、新存储目录、Bonjour、TLS、IPC 和 MIME 标识。
- 控制协议升级至主版本 2，不支持旧产品互通或自动升级。
- 用户手动安装并重新授权、配对和配置；不自动迁移或删除旧数据。
- 仓库、下载与 Homebrew 发布配置集中管理，不保留旧名回退。

发布前提及验收边界见 `docs/DESKFUSE_MIGRATION.md`。早期产品的发行记录保留在对应 Git 标签，不改写历史。
