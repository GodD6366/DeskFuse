# Security

## Reporting / 漏洞反馈

不要在公开 Issue 中提交可复现的敏感用户数据或尚未修复的漏洞利用细节。优先使用本仓库 GitHub **Security → Report a vulnerability**（维护者需启用 private vulnerability reporting）。如果入口不可用，先通过维护者 GitHub 个人资料中公开的联系方式请求私密沟通；若未提供联系方式，仅提交不含漏洞细节的“请求私密联系”Issue。

Please use GitHub private vulnerability reporting when enabled. If unavailable, request a private channel through the maintainer's public contact details, or file a contact-only issue with no exploit details. Do not attach clipboard bodies, credentials or identity stores.

当前仅跟进开发分支及最新预发行版的修复，不承诺响应 SLA 或旧实验版本长期支持。

## 信任边界

- mDNS 是发现提示，不是授权；配对和设备身份以 Ed25519 签名及用户确认建立。
- `resources/deskfuse-transport-cert.der` 和 `deskfuse-transport-key.der` 是**有意公开、全安装共享的 TLS 引导材料**，不是设备身份，不提供单机身份认证。不要给这组材料赋予信任或把它替换成私有生产凭据。
- 应用设备网络及输入握手必须验证绑定当前 QUIC TLS exporter、双端 Hello 和方向的 SessionProof；旧会话签名不得重放。公开引导私钥不能替代此验证。
- 设备私钥保存在 `~/Library/Application Support/DeskFuse/identity.key`，按用户隔离并使用 `0600` 权限；不提交、不删除或重建它来修复配对问题。发行签名私钥和发行 ledger 留在协调端仓库之外。GitHub 公开包的 Developer ID／公证、贡献者 ad-hoc 签名和 LAN 更新签名是不同机制。Apple `.p12`／`.p8` 只放在 Actions secrets 或本机 `.env`，见 [公证凭据](docs/NOTARIZE.md)。
- 断线、权限变化或输入异常应释放控制租约和按键；保留紧急收回。缓存删除仅覆盖应用自有数据。

## 当前分发限制

公开 tag 预发行在配置凭据后由 CI 做 Developer ID 签名与公证；未配置时该 job 失败而不是改发 ad-hoc。CI 不伪造辅助功能授权，不安装 Windows SCM 服务，也不替代真实设备验收。准备公开仓库时应启用 private vulnerability reporting，并检查 Issues、Actions 和发布分支／tag 管理设置；未配置的 GitHub 设置不能被描述为已生效。
