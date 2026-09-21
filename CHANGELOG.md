# Changelog

版本来自根 `Cargo.toml`；源码变化不代表已发布或真实设备验收通过。

## [Unreleased]

## [0.4.0] — DeskFuse 独立产品

- 产品、源码目录、Rust crate、Swift 模块、Windows 服务及安装资产统一使用 DeskFuse 命名。
- 使用 `com.deskfuse.app` 身份、新存储目录、Bonjour、TLS、IPC 和 MIME 标识。
- 控制协议升级至主版本 2，不支持旧产品互通或自动升级。
- 用户手动安装并重新授权、配对和配置；不自动迁移或删除旧数据。
- 仓库、下载与 Homebrew 发布配置集中管理，不保留旧名回退。

发布前提及验收边界见 `docs/DESKFUSE_MIGRATION.md`。早期产品的发行记录保留在对应 Git 标签，不改写历史。
