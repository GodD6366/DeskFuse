# macOS Developer ID 与公证凭据

公开 GitHub 预发行必须用 **Developer ID Application** 签名、打开 hardened runtime，再交给 Apple `notarytool` 公证并 staple。贡献者本机和 main 分支的 Homebrew artifact 仍可显式 `CODESIGN_IDENTITY=-`，不需要这些凭据。

把下面 6 个值加到私有源码仓库 `GodD6366/uniDesk` 的 **Settings → Secrets and variables → Actions**。不要提交到 Git，不要写进 issue 或 PR。本机调试可放在被忽略的 `.env`（变量名见根目录 `.env.example`）。

## 先决条件

1. 付费 [Apple Developer Program](https://developer.apple.com/programs/)（个人或组织均可）。
2. 用来操作证书的 Mac 已登录该团队，并安装完整 Xcode。
3. 团队角色能创建 **Developer ID Application** 证书，以及 App Store Connect API Key（Account Holder / Admin 可建；Developer 角色的 key 足够提交公证）。

## GitHub Actions secrets

| Secret | 内容 | 怎么拿到 |
| --- | --- | --- |
| `APPLE_DEVELOPER_ID_P12` | 导出的 Developer ID Application `.p12` 的 **标准 base64**（单行） | 见下方「证书」 |
| `APPLE_DEVELOPER_ID_P12_PASSWORD` | 导出 `.p12` 时自己设的密码 | 导出对话框里输入的那个，不是 Apple ID 密码 |
| `APPLE_TEAM_ID` | 10 位 Team ID | [developer.apple.com/account](https://developer.apple.com/account) → Membership details，或证书名末尾括号 |
| `APPLE_API_KEY` | `AuthKey_XXXXXXXXXX.p8` 的 **标准 base64**，或 PEM 原文 | 见下方「API Key」 |
| `APPLE_API_KEY_ID` | 10 位 Key ID | 创建 API Key 时页面上的 Key ID |
| `APPLE_API_ISSUER` | Issuer ID（UUID） | App Store Connect → Users and Access → Integrations → App Store Connect API 页顶 |

可选备用（没有 API Key 时才需要；CI 优先走 API Key）：

| Secret | 内容 | 怎么拿到 |
| --- | --- | --- |
| `APPLE_ID` | Apple ID 邮箱 | 登录 developer.apple.com 的那个账号 |
| `APPLE_APP_SPECIFIC_PASSWORD` | 专用密码 | [appleid.apple.com](https://appleid.apple.com) → 登录与安全 → App 专用密码 |

## 1. 记下 Team ID（`APPLE_TEAM_ID`）

1. 打开 [https://developer.apple.com/account](https://developer.apple.com/account) 并登录。
2. 进入 **Membership details**。
3. 复制 **Team ID**（10 个大写字母/数字，例如 `A1B2C3D4E5`）。
4. 仓库 → **Settings → Secrets and variables → Actions → New repository secret**，名称 `APPLE_TEAM_ID`，值粘贴 Team ID。

本机核对：

```sh
security find-identity -v -p codesigning
```

已安装的 Developer ID 会显示 `Developer ID Application: 名字 (TEAMID)`。

## 2. 创建并导出 Developer ID Application 证书

公证**不能**用 Apple Development / Apple Distribution / ad-hoc。必须是 **Developer ID Application**。

### 生成 CSR

1. 打开 **钥匙串访问 / Keychain Access**。
2. 菜单 **钥匙串访问 → 证书助理 → 从证书颁发机构请求证书**。
3. 用户电子邮件填你的 Apple ID；常用名称填开发者名；**不**选“存储到钥匙串”，选 **存储到磁盘**。
4. 得到 `CertificateSigningRequest.certSigningRequest`。

### 在开发者站点签发

1. 打开 [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/certificates/list)。
2. **Certificates → +**。
3. 选 **Developer ID Application**（Software 一栏，不是 Developer ID Installer）。
4. 上传 CSR，下载 `.cer`。
5. 双击 `.cer` 装进登录钥匙串。钥匙串里应出现 **Developer ID Application: 你的名字 (TEAMID)**，且能展开看到私钥。

若本机已有同名证书，不要重新生成一对新密钥覆盖正在用的发行身份；继续用现有证书导出即可。

### 导出 `.p12` 并做 base64（`APPLE_DEVELOPER_ID_P12`）

1. 钥匙串访问 → **登录 → 我的证书**。
2. 右键 **Developer ID Application: …** → **导出**。
3. 格式选 **个人信息交换 (.p12)**，保存到本机（不要放进仓库）。
4. 设置一个强密码，这就是 `APPLE_DEVELOPER_ID_P12_PASSWORD`。
5. 编码（macOS）：

```sh
base64 -i DeveloperID.p12 | pbcopy
```

6. 仓库新建 secret `APPLE_DEVELOPER_ID_P12`，粘贴剪贴板里的整段 base64。
7. 再新建 secret `APPLE_DEVELOPER_ID_P12_PASSWORD`，填导出时的密码。

核对解码后不是空文件：

```sh
printf '%s' "$APPLE_DEVELOPER_ID_P12" | base64 -d | wc -c
```

应明显大于 64 字节。

## 3. 创建 App Store Connect API Key（推荐）

比 Apple ID + 专用密码更适合 CI：可撤销、不绑 2FA 会话。

1. 打开 [App Store Connect API](https://appstoreconnect.apple.com/access/integrations/api)（路径：Users and Access → Integrations → App Store Connect API）。
2. 页顶复制 **Issuer ID**（UUID）→ secret `APPLE_API_ISSUER`。
3. **Team Keys → +**（或 Active 列表旁的生成按钮）。
4. Name 例如 `uniDesk GitHub Notarization`；Access 选 **Developer** 即可。
5. 生成后立刻 **下载 .p8**。只能下载一次，丢失只能作废重建。
6. 记下 **Key ID**（10 位）→ secret `APPLE_API_KEY_ID`。
7. 编码密钥：

```sh
base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy
```

8. 新建 secret `APPLE_API_KEY`，粘贴 base64。

本机也可不编码，把 PEM 原文放进 `.env` 的 `APPLE_API_KEY`（以 `-----BEGIN` 开头）。GitHub secret 请用 base64，避免换行被吃掉。

## 4. 可选：Apple ID 专用密码

没有 API Key 权限时：

1. 打开 [https://appleid.apple.com](https://appleid.apple.com) → 登录与安全 → **App 专用密码**。
2. 生成一条，标注 `uniDesk notarytool`。
3. secret `APPLE_ID` = Apple ID 邮箱；`APPLE_APP_SPECIFIC_PASSWORD` = 这条专用密码（形如 `xxxx-xxxx-xxxx-xxxx`）。
4. 仍需 `APPLE_TEAM_ID`。

## 5. 加到 GitHub 之后

1. 六个主 secret 都在 **Settings → Secrets and variables → Actions** 里（备用两项可空）。
2. 推送 `vX.Y.Z` tag 后，`Prerelease` 的 `package` job 会导入证书、用 Developer ID + hardened runtime 打包、`notarytool submit --wait`、staple，再写 `build-info.json` 的 `signing=developer-id`、`notarized=true`。
3. 缺任何必需 secret 时该 job 直接失败，**不会再发 ad-hoc 公开包**。
4. 证书轮换：重新导出 `.p12` / 新建 API Key，更新对应 secret；不要改 bundle id `com.unidesk.app`。

本机在已导入同一 Developer ID 的 Mac 上：

```sh
set -a && source .env && set +a
CODESIGN_IDENTITY="Developer ID Application: 你的名字 (TEAMID)" \
UNIDESK_REQUIRE_NOTARIZE=1 \
UNIDESK_DMG_FINDER_LAYOUT=0 \
./scripts/package-macos.sh --homebrew
```

`scripts/import-developer-id.sh` 只给 CI 或本机没有把证书装进登录钥匙串时用。

## 不要准备的东西

- **不要**把 Apple ID 主密码放进 secret。
- **不要**用 Apple Development 证书做公证。
- **不要**把 `.p12` / `.p8` / 专用密码提交到 Git 或 Actions artifact。
- LAN 自动更新的 Ed25519 发行私钥（`signing-key.hex`）与 Apple 公证无关，继续只放在协调端本机。
