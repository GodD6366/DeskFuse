# Windows 桌面客户端：Microsoft Store / MSIX 发布准备

本文件记录把 DeskFuse Windows 客户端发布到 Microsoft Store 的可行范围、已落地的打包链路、必须补齐的提交材料，以及当前架构与 Store 模型的硬性冲突。

## 1. 结论先行

- **可以进 Store 的部分**：桌面客户端本体（full-trust MSIX，`runFullTrust`）。剪贴板监听、资源管理器 OLE 拖放、命名管道 IPC、窗口子类化等桌面能力在 full-trust MSIX 下仍然可用。
- **不能进 Store 的部分**：
  - `DeskFuseService`：LocalSystem 服务、写入 `%ProgramFiles%\DeskFuse`、注册 SCM 恢复策略、创建防火墙规则。MSIX 无法安装 Win32 服务。
  - `deskfuse-input-helper.exe`：清单请求 `uiAccess="true"`。MSIX **不能**声明 UIAccess，Store 包也无法获得该令牌，因此"跨 UIPI 注入到管理员窗口"在 Store 版本中不可用。
  - 机器级状态：`%ProgramData%\com.deskfuse.app`（身份、信任、SQLite、暂存缓存）由服务持有。
- 因此 Store 包只覆盖客户端。完整功能仍需要服务由独立安装器提供；这一点必须在提交说明中如实描述，否则有被判为"依赖外部安装器"而拒审的风险。可选策略见第 5 节。

## 2. 已实现的打包链路

| 文件 | 作用 |
|---|---|
| `apps/deskfuse-windows/Package.appxmanifest` | MSIX 包清单：占位 Identity、`runFullTrust`/`internetClient`/`privateNetworkClientServer`、`uap10:TrustLevel="mediumIL"` + `uap10:RuntimeBehavior="packagedClassicApp"`（这两个是 **属性**，不是子元素） |
| `apps/deskfuse-windows/app.store.manifest` | 供 Store/无 UI 场景使用的 exe 清单：`asInvoker` 且 **不含** uiAccess；离线 NSIS 包继续使用带 uiAccess 的 `app.manifest` |
| `scripts/generate-store-assets.ps1` | 从既有应用图标生成 MSIX 图像资产并写 `Assets/store-assets.json`（含每个文件的 SHA-256） |
| `scripts/package-windows-store.ps1` | `dotnet publish` → 组装 layout → `MakeAppx pack`（pack 阶段即做 schema 校验）→ 可选 `signtool` 签名 → 可选 WACK → 输出 `.msix`、`.sha256`、`.store.json` |

产物示例（本机已验证，客户端仍为 WPF 版本）：`dist/windows-store/DeskFuse-0.4.64.0-x64.msix`，71 436 955 字节，SHA-256 `303691d6…704d`，未签名、占位 Identity。

### 版本映射

Store 要求 4 段版本且 `Revision` 必须为 0，同时每次提交必须严格递增。映射规则：

```
MSIX Version = <产品 Major>.<产品 Minor>.<release.json build_number>.0
例：产品 0.4.5 + build 64 → 0.4.64.0
```

`build_number` 在每次发布都会递增，因此包版本单调递增，无需再引入第二个计数器；代价是 MSIX 版本号不逐字对应 `X.Y.Z`（Partner Center 展示版本可另行填写）。可用 `-Version` 显式覆盖。

### 发布命令

```powershell
# 生成图像资产（需要 Windows；优先使用 resources/generated/DeskFuse-icon-1024.png）
.\scripts\generate-store-assets.ps1

# 仅打包（占位 Identity，用于内部验证）
.\scripts\package-windows-store.ps1

# 面向提交：真实 Identity + 测试签名 + WACK
.\scripts\package-windows-store.ps1 `
  -IdentityName "12345GodD6366.DeskFuse" `
  -Publisher "CN=XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX" `
  -PublisherDisplayName "GodD6366" `
  -SigningPfx .\secrets\store-test.pfx -SigningPassword "***" `
  -RequireStoreIdentity -RunWack
```

`-RunWack` 需要 `Windows Kits\10\App Certification Kit\appcert.exe`；本机已确认存在但**未运行**（见第 6 节验证边界）。

## 3. 提交前必须补齐

1. **Identity**：Partner Center 保留应用后，用真实 `Package/Identity/Name` 与 `Publisher`（证书 CN）替换占位值；脚本默认读取清单里的值，`-RequireStoreIdentity` 可阻止带占位值出包。
2. **图像资产**：`StoreLogo.png`(50×50)、`Square44x44Logo.png`、`Square150x150Logo.png`、`Wide310x150Logo.png`、`SplashScreen.png`(620×300) 已生成；另需 Partner Center 的 **应用商店图标 300×300**（已生成 `StoreListingLogo-300x300.png`）与**至少一张 1366×768 以上截图**。
3. **文案与合规**：应用说明（zh-CN/en-US）、隐私政策 URL、支持联系方式、年龄分级问卷、类别、版权信息。
4. **能力说明**：`runFullTrust` 需在提交时说明理由（本地剪贴板、OLE、命名管道、窗口子类化、无 UWP 替代）；`privateNetworkClientServer`/`internetClient` 用于局域网发现与 QUIC 传输。
5. **认证风险自查**：客户端的核心功能依赖机器上单独安装的服务。提交说明必须写清楚，或按第 5 节改造。
6. **签名**：Store 由微软签名；本地/CI 测试用 `-SigningPfx`。清单 `Publisher` 必须与签名证书 `Subject` 完全一致（Store 证书的 Subject 就是保留身份的 Publisher）。`package-windows-store.ps1` 已在 publish 前校验，不一致时直接给出可执行的报错，不再落到 SignTool 的 `0x8007000b`。

## 4. 与 Store 模型的硬性冲突（清单）

| 现有能力 | 实现方式 | 在 MSIX/Store 中 |
|---|---|---|
| 机器身份/信任/网络运行时 | LocalSystem 服务 + `%ProgramData%` | ❌ 不能打包；需外置安装器或重构到应用进程 |
| 跨 UIPI 注入（管理员窗口） | helper 的 `uiAccess="true"` | ❌ MSIX 不能声明 UIAccess |
| 防火墙放行（UDP 42424/42425/5353） | 安装器调用 `New-NetFirewallRule` | ⚠️ 客户端不需要；服务侧仍需外置安装器。MSIX 有 `desktop6:firewallRules` 扩展，但只适用于包内可执行文件 |
| 安装到 `%ProgramFiles%\DeskFuse` | NSIS | ❌ Store 包装到 `WindowsApps`（只读、由系统管理） |
| 资源管理器 OLE 延迟渲染、CF_HDROP | Win32/OLE | ✅ full-trust MSIX 可用 |
| 全局剪贴板监听 | `AddClipboardFormatListener` + 窗口消息 | ✅ full-trust MSIX 可用 |
| 与服务的认证命名管道 | `\\.\pipe\com.deskfuse.app.desktop.v2` | ✅ full-trust MSIX 可用，但管道 ACL 需允许打包应用身份（当前按交互会话/用户授权，未验证） |
| 登录自启 | 客户端自带策略 | ⚠️ Store 应改用 `windows.startupTask` 扩展（尚未接入） |

## 5. Store 策略选项

- **A. 客户端进 Store + 服务外置安装（改动最小）**：当前脚本即此形态。风险：核心功能依赖外置安装器，审核可能拒绝；也不适合只想装 Store 版的用户。
- **B. Store 版本裁剪**：Store 包只提供"本机历史 + 需要服务时的明确提示"，跨设备能力标注需另行安装。审核友好，但产品价值受限。
- **C. 把运行时搬进应用进程**：full-trust 桌面应用可自行做 UDP/QUIC、mDNS、DPAPI 用户级身份，去掉服务依赖；代价是失去"未登录也在运行"的能力，并需要重做身份/信任归属与 IPC 边界。工作量最大，但能真正单包发布。
- **D. Windows 11 打包服务**（仅在评估中）：Win11 支持打包应用内的服务扩展，但运行账户不是 LocalSystem，仍需重做权限模型，且不覆盖 Windows 10。

建议：短期走 A（不阻塞其它工作），中期评估 C。

## 6. 验证边界

已验证：

- `scripts/generate-store-assets.ps1` 生成 7 个 PNG 与 `store-assets.json`。
- `scripts/package-windows-store.ps1` 完整跑通：publish → MakeAppx pack 成功 → `signtool` 签名成功 → `.msix` + `.sha256` + `.store.json`。
- AppxManifest 通过 MakeAppx 的 schema 校验（修正了 `windows.appExecutionAlias` 命名空间与 `uap10:TrustLevel/RuntimeBehavior` 的写法）。
- 签名证书 Subject 与清单 Publisher 不一致时，脚本在前置校验阶段即失败并给出修正参数（负向用例实测）。
- **实测安装与运行**：测试签名后的 MSIX 经 `Add-AppxPackage` 注册为 `DeskFuse.Desktop_0.4.64.0_x64__1fq7k4d2z8axe`，从 `C:\Program Files\WindowsApps\...` 启动成功；打包版 full-trust 客户端与 LocalSystem 服务的认证命名管道 IPC 正常（界面显示"服务运行中"，剪贴板历史与预览可用）。验证后已 `Remove-AppxPackage` 清理，未改变常规安装。

未验证（不得表述为已通过）：

- WACK（`appcert.exe`）未运行；未做 Store 提交；未使用真实 Partner Center Identity/Publisher，未做 Store 签名。
- 未在 Windows 10 1809/21H2 等 `TargetPlatformMinVersion` 边界系统上安装验证。
- WinUI 3 迁移尚未完成，上述打包与安装验证使用的是 WPF 客户端；迁移完成后同一打包脚本可直接复用（需以 `-ClientProject` 指向 WinUI 工程）。
- 打包版客户端的开机自启、防火墙策略、多用户/多会话行为均未实测。
