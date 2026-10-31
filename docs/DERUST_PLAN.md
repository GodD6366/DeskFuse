# DeskFuse 移除 Rust 开发方案

> 状态（2026-10-31）：早期备选方案，未采用。当前架构为 MyGo UI + Go 中间层 + 平台层，Rust 保留为底层能力（输入共享、音频共享、剪贴板同步、协议与身份等）。本文件仅作历史参考，不作为执行计划。

> 目标：开源友好、无 Rust 构建、各端原生实现（macOS Swift / Windows C#）
> 制定于：2026-10 | 预计总周期：**24–33 周**

---

## 现状速览

```
macOS   Swift UI (20K行) ──UniFFI in-process──▶ Rust dylib
                                                  deskfuse-ffi
                                                    └─ deskfuse-engine (27K行)
                                                    └─ deskfuse-runtime (18K行)
                                                    └─ deskfuse-transport (8K行)
                                                    └─ deskfuse-transfer (7K行)
                                                    └─ deskfuse-store (2K行)
                                                    └─ deskfuse-protocol / identity / discovery

Windows C# WPF (6K行) ──named pipe JSON RPC──▶ deskfuse-service.exe (Rust)
                          \\.\pipe\com.deskfuse.app.desktop.v2
```

**关键差异：**
- Windows 端边界已存在（named pipe JSON），C# 和 Rust 已解耦，替换成本最低
- macOS 端是进程内 FFI（UniFFI），需先改成 IPC 才能替换引擎

---

## 各端替换技术栈

| 层 | macOS → Swift | Windows → C# |
|----|--------------|--------------|
| QUIC 传输 | `Network.framework` `NWConnection` | `System.Net.Quic`（.NET 9+） |
| Ed25519 | `CryptoKit` `Curve25519.Signing` | NSec 或 BouncyCastle |
| Protobuf | `apple/swift-protobuf` | `Google.Protobuf` |
| mDNS | `Network.framework` `NWBrowser/NWListener` | `Makaretu.Dns.Multicast` |
| SQLite | `GRDB.swift` | `Microsoft.Data.Sqlite` |
| 并发模型 | Swift Concurrency（actors） | C# async/await + `System.Threading.Channels` |
| 输入注入 | `CGEvent`（已有） | `Windows.Win32.SendInput`（已有） |

---

## Session Proof 风险说明（所有阶段均影响）

当前实现在 QUIC TLS 握手完成后调用 `quinn::Connection::export_keying_material`，
把 TLS exporter 材料混入 Ed25519 签名，防止中间人攻击：

```
payload = LABEL ‖ network_only_flag
        ‖ canonical_hello_payload(client)
        ‖ canonical_hello_payload(server)
        ‖ export_keying_material(LABEL, transcript)  // QUIC 绑定
        ‖ server_proof_flag
signature = Ed25519_sign(payload, local_key)
```

**Network.framework 和 System.Net.Quic 是否支持 TLS exporter 需要在 Phase 0 验证。**
如果不支持，替代方案：握手期间交换 fresh nonce，双方分别签名 `LABEL ‖ hello_transcript ‖ nonce_client ‖ nonce_server`，
放弃 TLS 绑定但保留双向身份验证。这是协议变更，必须在 Phase 0 决定并写入规范。

---

## 并行开发总览

```
周  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33
    ├──── Phase 0 规范提取 ────┤
                  ├── Phase 1 互操作测试套件 ──┤
                              ├────────────────── Phase 2 Windows C# 服务 ──────────────────┤
                              ├─── Phase 3 macOS IPC 解耦 ────┤
                                                               ├───────────────────────── Phase 4 macOS Swift 引擎 ────────────────────────────────┤
                                                                                                                                                   ├─ Phase 5 ─┤

并行关系：
  Phase 0 + Phase 1      顺序（1 依赖 0 的向量文件）
  Phase 0/1 + Phase 2    可并行（Phase 2 第 1–2 周可先做项目脚手架）
  Phase 2 + Phase 3      完全并行（不同平台，互不依赖）
  Phase 4                依赖 Phase 1（需要 interop 向量）+ Phase 3（需要 IPC 边界）
  Phase 5                依赖 Phase 4 完成
```

---

## Phase 0 — 规范提取

**周期：** 2–3 周 | **风险：** 低 | **可并行：** Phase 2 第 1–2 周的脚手架

### 目标

把所有散落在 Rust 代码里的隐性约定变成正式文档，放入 `spec/` 目录。
这是后续所有实现的合同，任何新实现只看 spec 目录，不需要读 Rust 代码。

### 交付物

```
spec/
  peer.proto                      # 来源：crates/deskfuse-protocol/src/wire.rs
  clipboard-sync.proto            # 来源：protocol/src/clipboard_sync.rs
  motion.proto                    # 来源：protocol/src/motion.rs
  IDENTITY.md                     # 来源：protocol/src/identity.rs
  DISCOVERY.md                    # 来源：crates/deskfuse-discovery/src/lib.rs
  SESSION_PROOF.md                # 来源：crates/deskfuse-transport/src/session_auth.rs
  input-share-message.schema.json # 来源：crates/deskfuse-core/src/sharing.rs ← 最难
  desktop-ipc.schema.json         # 来源：apps/deskfuse-service/src/desktop_protocol.rs
```

### 各文件要点

**peer.proto**
- Envelope：major/minor/request_id(16B)/message(oneof)
- 所有 26 个消息变体（tag 10–48）
- 每个消息的字段约束写成注释（nonce=32B、device_id=32B、public_key=32B 等）

**IDENTITY.md**
- `device_id = SHA-256(public_key)`，32 字节
- `canonical_hello_payload` 编码格式：
  ```
  domain    = b"deskfuse/2 hello canonical\0"
  major     = uint32 big-endian
  minor     = uint32 big-endian
  device_id = uint32_be(len) ‖ bytes
  name      = uint32_be(len) ‖ utf8
  pubkey    = uint32_be(len) ‖ bytes
  nonce     = uint32_be(len) ‖ bytes
  caps      = uint64 big-endian
  ```
- Ed25519 `verify_strict`（拒绝弱签名）
- nonce 必须 32 字节，display_name 1–256 个 Unicode 字符

**SESSION_PROOF.md**
- payload 构造完整字节序
- TLS exporter label：`b"DeskFuse session identity v1"`
- 双向握手顺序（client 先发，server 后发）
- **标注：Network.framework / System.Net.Quic TLS exporter 支持待验证**

**input-share-message.schema.json** ← 最复杂，单独说明

`InputShareMessage` 是 JSON 序列化后装入 protobuf `InputSharing.payload` 的两层结构。
需要覆盖所有 20 个变体：

| 变体 | 关键约束 |
|------|---------|
| `ForgetControllerEnvironment` | - |
| `ControllerPresence` | display_id 非空 |
| `ControllerSelection` | display_id 非空 |
| `Activate` | sequence ≥ 1 |
| `Activated` | sequence ≥ 1，reason ≤ 256 字符 |
| `Deactivate` | sequence ≥ 1 |
| `RequestHybrid` | target_display 非空 |
| `GrantHybrid` | ttl_ms > 0 |
| `Hybrid` | update 见下 |
| `Request`（legacy） | - |
| `Grant`（legacy） | ttl_ms > 0 |
| `Denied` | - |
| `Enter` | sequence ≥ 1，topology_generation ≥ 1 |
| `Frame` | sequence ≥ 1，events 非空，**events 不得包含 Motion 变体** |
| `Renew` | ttl_ms > 0 |
| `Leave` | - |
| `Release` | - |
| `DisplayTopology` | 1–32 个显示器，id 非空 |
| `DisplayWallpaper` | payload ≤ 96 KB |
| 其他 | - |

`HybridInputUpdate::Transition.events` **不得包含 `Motion` 变体**（核心安全约束）

### 完成标准

- `spec/` 所有文件 Review 通过，提交入 `main`
- Session proof TLS exporter 可行性结论写入 `SESSION_PROOF.md`

---

## Phase 1 — 互操作测试套件

**周期：** 2–3 周 | **风险：** 低 | **依赖：** Phase 0 交付物
**可并行：** Phase 2 第 1–4 周的基础层开发（transport/identity）

### 目标

以 Rust 实现为 oracle，建立字节级测试向量和端到端互操作测试，
任何新实现必须通过全部测试才能上线。

### 目录结构

```
interop/
  vectors/
    hello-sign.json          # Ed25519 签名向量（pubkey、signing_input、expected_sig）
    canonical-hello.json     # canonical_hello_payload 输入/输出
    frame-codec.bin          # encode/decode 往返测试帧
    clipboard-sync.bin       # ClipboardSync HELLO→DATA→ACK 完整帧序列
    motion-datagram.bin      # DFM\x01 前缀 datagram
    input-share-message.json # InputShareMessage 所有合法变体 + 非法变体（期望拒绝）
  runner/
    generate-vectors.sh      # 调用 Rust 生成向量（只运行一次，结果锁定）
    run-rust.sh              # Rust baseline（CI 始终跑）
    run-csharp.sh            # Phase 2 完成后添加
    run-swift.sh             # Phase 4 完成后添加
  README.md
```

### 测试分层

| 层 | 内容 | 运行方式 |
|----|------|---------|
| Wire | Envelope encode/decode、帧边界（截断/超长/trailing bytes） | 纯字节，无网络 |
| Identity | canonical_hello_payload、sign/verify、device_id 派生 | 纯算法 |
| Validation | 每个消息变体的约束检查（合法 + 非法向量） | 纯字节 |
| Protocol flow | Hello↔HelloAck、Pairing、SessionProof、Ping↔Pong | in-memory transport |
| Features | Clipboard sync chunked 往返、InputShareMessage 完整变体 | in-memory |
| E2E | 两进程通过 localhost QUIC 握手 + clipboard + input 完整流程 | 进程间 |

### 完成标准

- 向量文件锁定提交，任何修改需要解释原因
- Rust baseline 所有层全绿（CI gate）

---

## Phase 2 — Windows C# 服务

**周期：** 6–8 周 | **风险：** 中 | **可并行：** Phase 3 完全并行

### 目标

C# WPF 应用（`apps/deskfuse-windows/`）**一行不动**，
只替换 named pipe 另一端的 Rust service。

```
deskfuse-windows (C# WPF，不变)
    └── ServiceClient.cs ──named pipe──▶ [DeskFuse.Service.exe（C#，本阶段新建）]
                                             └── QUIC ──▶ 对等网络
```

### 模块分解与依赖关系

```
Week 1–2  [可并行于 Phase 0/1]
  ┌─ 项目脚手架
  │    DeskFuse.Service/
  │      DeskFuse.Service.csproj  (.NET 9, Windows)
  │      Program.cs
  │      README.md
  └─ IdentityStore（无网络依赖，可立即开发）
       生成/持久化 Ed25519 密钥对
       canonical_hello_payload 实现
       device_id = SHA-256(pubkey)
       单元测试对齐 interop/vectors/hello-sign.json

Week 2–3  [依赖 Phase 1 Wire 层向量]
  ┌─ FrameCodec
  │    4字节大端长度前缀 + protobuf Envelope
  │    最大帧 16 MB
  │    单元测试对齐 interop/vectors/frame-codec.bin
  └─ ProtobufTransport（System.Net.Quic）
       ALPN: "deskfuse/2"
       TLS 证书从 Ed25519 密钥派生（或自签名 ECDSA，规范决定）
       验证 TLS exporter 支持（SESSION_PROOF.md 结论）

Week 3–4
  ┌─ PeerDiscovery
  │    NW 发布 _deskfuse._udp.local.
  │    TXT: v/id/name/capabilities/port/protocol
  └─ SessionManager
       Hello↔HelloAck 握手
       SessionProof（双向 Ed25519）
       配对请求/接受/拒绝状态机
       重连退避

Week 4–5
  ┌─ ClipboardSync
  │    chunked 同步：HELLO→DATA→ACK
  │    SourceMetadata（app_bundle_id、no_history、image_digest）
  │    source policy version 检查
  │    最大单块 1 MiB，总大小 257 MiB
  └─ InputController
       InputShareMessage 所有变体解析
       motion datagram（DFM\x01，最大 512B）
       SendInput 注入
       单元测试对齐 interop/vectors/input-share-message.json

Week 5–6
  ┌─ PipeServer
  │    \\.\pipe\com.deskfuse.app.desktop.v2
  │    JSON RPC v2，最大帧 24 MiB
  │    所有 ~50 个 Command 变体
  └─ Store
       Microsoft.Data.Sqlite
       PeerRecord（trust/pairing 状态）
       Settings
       clipboard 历史（与现有 schema 兼容）

Week 6–7
  ├─ FileTransfer（TransferEngine C# 实现）
  ├─ WorkflowEngine（基础实现，可后续迭代）
  └─ E2E 两机验收
       interop runner/run-csharp.sh 全绿
       C# service ↔ macOS Rust daemon E2E 测试
```

### 风险

| 风险 | 缓解 |
|------|------|
| System.Net.Quic TLS exporter 不支持 | Phase 0 先验证；如不支持，改用 nonce 挑战方案，同时更新 spec 和 Rust 实现 |
| InputShareMessage Transition.events 约束遗漏 | interop 向量包含所有非法变体，作为 CI gate |

### 完成标准

- `interop/runner/run-csharp.sh` 全部层级通过
- 真实 Windows + macOS 两机全功能验收
- Windows 构建流水线删除所有 Rust 相关步骤

**删除：**
```
apps/deskfuse-service/         （Windows 相关部分）
apps/deskfuse-input-windows/
crates/deskfuse-platform-windows/
```

---

## Phase 3 — macOS IPC 解耦

**周期：** 3–4 周 | **风险：** 中 | **可并行：** Phase 2 完全并行

### 目标

把 macOS 进程内 UniFFI FFI 改成进程间 Unix domain socket IPC，
Swift UI 代码不变，Rust 变成独立 daemon。这是 Phase 4 的前提。

```
[现状]  Swift UI ──UniFFI (in-process)──▶ Rust dylib

[目标]  Swift UI ──Unix socket──▶ deskfuse-engine-daemon (Rust，暂时保留)
                  /tmp/com.deskfuse.engine.sock
                  JSON RPC（复用 desktop-ipc.schema.json，与 Windows pipe 协议对称）
```

选择 Unix socket 而非 XPC 的原因：与 Windows named pipe 使用同一 JSON RPC schema，
两端协议对称，减少规范维护负担。

### 模块分解

```
Week 1
  ├─ 在 deskfuse-ffi 旁新建 deskfuse-ipc-server
  │    Unix socket 服务端（复用 desktop_protocol.rs 类型）
  │    JSON 帧协议（与 Windows pipe 一致：4字节小端长度 + JSON）
  └─ Swift DeskFuseServiceClient
       替换 DeskFuseEngine.swift（UniFFI 生成）
       连接 Unix socket，发送 JSON RPC，接收快照事件

Week 2
  ├─ 逐命令迁移：Snapshot、Poll、设备操作（Pairing/Disconnect/Revoke）
  └─ 剪贴板命令（PublishClipboard、CommitClipboard 等）

Week 3
  ├─ 输入命令（RegisterInput、EnterInputControl、SendInputEvents 等）
  ├─ Workflow 命令
  └─ RemoteApp 命令

Week 3–4
  └─ macOS 完整功能验收
```

### 完成标准

- Swift UI 通过 Unix socket 与 Rust daemon 通信，功能与原 FFI 等价
- macOS 所有现有功能验收通过
- `Generated/DeskFuseEngine.swift` 和 `Sources/DeskFuseEngine/` 删除

**删除：**
```
crates/deskfuse-ffi/
apps/deskfuse-macos/Generated/DeskFuseEngine.swift
apps/deskfuse-macos/Sources/DeskFuseEngine/
```

---

## Phase 4 — macOS Swift 引擎

**周期：** 10–14 周 | **风险：** 高 | **依赖：** Phase 1 全绿 + Phase 3 完成

### 目标

把 Phase 3 的 Rust daemon 替换为 Swift 实现。

```
[Phase 3] Swift UI ──socket──▶ deskfuse-engine-daemon (Rust)
[Phase 4] Swift UI ──socket──▶ DeskFuseService (Swift)
```

### 模块分解与依赖关系

```
Week 1–2  [基础层，无依赖]
  ┌─ IdentityStore
  │    CryptoKit Curve25519.Signing（即 Ed25519）
  │    Keychain 持久化
  │    canonical_hello_payload 实现
  │    单元测试对齐 interop/vectors/
  └─ FrameCodec
       4字节大端长度前缀 + swift-protobuf Envelope
       单元测试对齐 interop/vectors/frame-codec.bin

Week 2–4
  ┌─ PeerTransport
  │    Network.framework NWConnection
  │    QUIC + ALPN "deskfuse/2"
  │    TLS 配置（验证 TLS exporter 或使用 nonce 方案）
  └─ Discovery
       NWBrowser（监听 _deskfuse._udp.local.）
       NWListener（发布自身）
       TXT 记录格式与 spec 一致

Week 3–5  [依赖 PeerTransport]
  └─ SessionManager
       Hello↔HelloAck 握手
       SessionProof（双向 Ed25519）
       配对状态机（Request/Accept/Reject/Revoke）
       连接生命周期管理
       重连退避（exponential backoff）

Week 5–7  [依赖 SessionManager]
  ├─ ClipboardEngine
  │    chunked sync 协议（HELLO/DATA/ACK）
  │    SourceMetadata + source policy
  │    历史持久化（GRDB.swift）
  └─ Store
       GRDB.swift
       PeerRecord、Settings、clipboard 历史
       schema 与现有 SQLite 文件兼容

Week 6–9  [依赖 SessionManager，可与 ClipboardEngine 并行]
  └─ InputEngine                            ← 最高复杂度
       InputShareMessage 所有 20 个变体
       ActivationManager（Activate/Activated/Deactivate 状态机）
       HybridSession（RequestHybrid/GrantHybrid/Hybrid 状态机）
       motion datagram 编解码（DFM\x01 前缀）
       CGEvent 注入
       单元测试对齐 interop/vectors/input-share-message.json
         - 所有合法变体 roundtrip
         - 所有非法变体（Transition.events 含 Motion）期望拒绝

Week 8–10  [依赖 ClipboardEngine]
  └─ FileTransferEngine
       chunked 传输状态机
       DeferredFileEntry / DeferredClipboardSource
       接收暂存目录管理

Week 9–11
  └─ SocketServer
       Unix socket 服务端
       响应所有 IPC 命令（与 Phase 3 deskfuse-ipc-server 等价）
       快照推送、事件订阅

Week 10–12
  ├─ WorkflowEngine（基础实现）
  ├─ UpdateEngine（版本检查、LAN 对等更新分发）
  └─ LanWeb（可选，按优先级）

Week 12–14
  └─ E2E 验收
       interop/runner/run-swift.sh 全绿
       Swift ↔ C# 两机完整验收
       Swift ↔ Rust（如仍在运行）互操作验证
```

### 关键注意事项

**InputEngine 是全程最高风险模块。**
`HybridInputUpdate::Transition.events` 中不允许出现 `Motion` 变体，
这个约束只存在于 Rust 的 `validate()` 里。
必须先有 interop 向量覆盖所有合法和非法组合，再开始编写实现。

**不要跳过 interop 向量：**
如果 Swift 实现接受了 Rust 拒绝的 payload（或反之），
就是跨平台安全漏洞，不是兼容性问题。

### 完成标准

- `interop/runner/run-swift.sh` 全部层级通过
- Swift ↔ C# E2E 两机全功能验收（键鼠/剪贴板/文件传输）
- Rust daemon 从 macOS 构建流水线移除

**删除：**
```
crates/deskfuse-engine/
crates/deskfuse-runtime/
crates/deskfuse-transport/
crates/deskfuse-transfer/
crates/deskfuse-store/
crates/deskfuse-protocol/
crates/deskfuse-identity/
crates/deskfuse-discovery/
crates/deskfuse-core/
crates/deskfuse-platform/
crates/deskfuse-platform-macos/
apps/deskfuse-input-macos/
apps/deskfuse-probe/
```

---

## Phase 5 — 移除 Rust Workspace

**周期：** 1 周 | **风险：** 低 | **依赖：** Phase 4 完成

```bash
git rm -r crates/
git rm -r apps/deskfuse-service/ apps/deskfuse-probe/
git rm -r apps/deskfuse-input-macos/ apps/deskfuse-input-windows/
git rm -r tools/deskfuse-bindgen/ tools/deskfuse-sdk/
git rm Cargo.toml Cargo.lock rust-toolchain.toml
```

### 最终仓库结构

```
DeskFuse/
  apps/
    deskfuse-macos/          # Swift（Xcode 直接打开）
    deskfuse-windows/        # C#（Visual Studio / dotnet CLI）
  spec/
    peer.proto
    clipboard-sync.proto
    motion.proto
    IDENTITY.md
    SESSION_PROOF.md
    DISCOVERY.md
    input-share-message.schema.json
    desktop-ipc.schema.json
  interop/
    vectors/
    runner/
  docs/
  CHANGELOG.md
  README.md
```

贡献者只需 Xcode 或 Visual Studio，首次构建 < 5 分钟。

---

## 阶段时间线与并行关系

| 阶段 | 周期 | 依赖 | 可并行 |
|------|------|------|--------|
| 0 规范提取 | 2–3 周 | — | Phase 2 脚手架 |
| 1 互操作套件 | 2–3 周 | Phase 0 | Phase 2 基础层 |
| 2 Windows C# 服务 | 6–8 周 | Phase 0/1 基础 | **Phase 3 完全并行** |
| 3 macOS IPC 解耦 | 3–4 周 | Phase 0 | **Phase 2 完全并行** |
| 4 macOS Swift 引擎 | 10–14 周 | Phase 1 全绿 + Phase 3 | — |
| 5 移除 Rust | 1 周 | Phase 4 完成 | — |

**总计：24–33 周（约 6–8 个月）**

最佳并行策略：同时推进 Phase 2 + Phase 3，两者完全独立；
Phase 4 在两者都完成后启动。

---

## 风险汇总

| 风险 | 阶段 | 严重度 | 缓解措施 |
|------|------|--------|---------|
| TLS exporter 平台库不支持 | 2, 4 | 高 | Phase 0 提前验证；备选：nonce 挑战方案，同步修改 spec |
| InputShareMessage Transition 约束遗漏 | 2, 4 | 高 | interop 向量覆盖所有非法变体，CI 强制检查 |
| macOS QUIC TLS 证书配置（Network.framework 对 Ed25519 TLS 支持有限） | 4 | 中 | Phase 4 Week 2–3 优先验证；可能需要 ECDSA 证书 + Ed25519 身份分离 |
| Swift 实现与 C# 实现功能进度不同步 | 4 | 中 | interop 全绿作为版本发布的硬性门控 |
| macOS Unix socket entitlement 与 hardened runtime 冲突 | 3 | 低 | Phase 3 第 1 周先做 PoC 验证 |

---

## 开始动手

最低风险的第一步（约半天）：

```bash
mkdir -p spec
# 把 wire.rs 翻译成 peer.proto（不改任何现有代码）
```

同时验证 TLS exporter 支持：

```swift
// macOS PoC：Network.framework QUIC 是否暴露 TLS exporter
let connection = NWConnection(to: endpoint, using: quicParameters)
// 验证是否有 API 等价于 quinn::Connection::export_keying_material
```

```csharp
// Windows PoC：System.Net.Quic TLS exporter
// .NET 9 QuicConnection 是否支持 exportKeyingMaterial
```
