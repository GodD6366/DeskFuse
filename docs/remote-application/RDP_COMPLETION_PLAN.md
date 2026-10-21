# Windows RDP 远程应用完善开发方案（Mac 主控 → Windows 被控）

> 状态：P0 完成；P1 代码与 Rust/C/Swift 自动测试完成，P1 实机项待验收；P2–P7 未开始。日期：2026-10-21。
> 范围：把现有“能连上、能看到画面”的 Beta 实现，补齐为可正式发布的 Windows 远程应用功能。
> 本文中的“100% 完成”指第 9 节完成定义（DoD）里的每一项都有自动测试或实机验收记录。编译通过、CI 通过都不算完成。

---

## 1. 现状基线（2026-10-21 代码核对结果）

| 模块 | 位置 | 现状 |
| --- | --- | --- |
| Windows 应用目录 | `apps/deskfuse-service/src/native/remote_app.rs` | 从 `App Paths` 和三个内置应用发现应用，首次启动时写入 `TSAppAllowList`，用 JSON v1 返回 `.rdp` |
| 连接方式 | `RemoteRdpSession.swift` | 把 `.rdp` 中的 `{host}` 替换为当前活动路由的 IP，**直连 `Windows:3389`**，流量不经过 DeskFuse |
| C 桥 | `RdpBridge/RdpBridge.c` | 支持 TLS 指纹固定、NLA 凭据、拒绝重定向、RAIL 握手、整张 GDI 画面拷贝、鼠标/键盘/Unicode 注入 |
| 画面 | `RemoteRdpViews.swift` `RemoteRdpCanvasView` | 30 Hz 定时器轮询，每帧全量 `memcpy`，再生成 `Data`、`CGImage`、`NSImage`，缩放后画进一个 1024×720 的 `NSWindow` |
| 多应用 | `RemoteApplicationManager.openWindowsApplication` | **每个应用各开一条 RDP 连接** |
| 凭据 | `RemoteRDPCredentialStore.swift` | 只保存在 macOS 钥匙串；证书指纹要用户手动核对 |
| 构建 | `scripts/build-freerdp-macos.sh` | FreeRDP 3.14.1 静态库；`CLIPRDR`、`RAIL`、`DISP`、`RDPGFX`、`RDPEI` 编进库里，但 `RDPSND`、`AUDIN`、`RDPDR` 都没编 |

## 2. 差距清单

按严重程度排序。“证据”一列都是代码里能直接看到的。

| # | 问题 | 影响 | 证据 |
| --- | --- | --- | --- |
| G1 | 每个应用各开一条 RDP 连接 | Windows 对同一个用户只保留一个会话（`fSingleSessionPerUser` 默认开启），开第二个应用会把第一个窗口的连接挤掉 | `openWindowsApplication` 每次 `RemoteRdpSession(...)` 加 `connect()` |
| G2 | 不是无缝窗口 | 远端的所有窗口挤在一个缩放后的画布里，菜单、对话框、多个窗口都错位；窗口没法独立移动、缩放、最小化 | 没有注册 `WindowCreate/Update/Delete`，只渲染 `gdi->primary_buffer` |
| G3 | 直连 3389 | 需要 Windows 开放防火墙；IP 在建立选择器时就固定了，雷电和 Wi-Fi 之间切换后连接断掉也不重连；只支持 IPv4 | `windowsHost(endpoint)`，`full address:s:{host}:3389` |
| G4 | 剪贴板不通 | 复制粘贴是最基本的需求。DeskFuse 自己的剪贴板同步在 Windows 进入远程会话时会主动停用（`SessionBlocker::RemoteSession`），所以 RDP 期间两条路都不通 | 桥里没有处理 `cliprdr`；`deskfuse-platform-windows/src/session.rs` 判定为远程会话就停用 |
| G5 | 键盘映射 | ⌘ 映射成 Win 键，⌘C/⌘V 不起作用；没用 Mac 输入法；没有锁定键同步；功能键、小键盘、ISO/JIS 键不全 | `scancodes` 表；`keyDown` 不走 `interpretKeyEvents` |
| G6 | 鼠标 | 没有水平滚动；滚轮量 `& 0x1ff` 在大数值时会回绕，方向会反；没有侧键；不显示远端光标形状 | `scrollWheel`；没有 `Pointer_New/Set` |
| G7 | 渲染性能 | 30 Hz 全量拷贝，每帧都新建对象；在 Retina 屏上发糊；分辨率不跟 Mac 屏幕走 | `refreshFrame`；没有使用 `DesktopScaleFactor`、`disp` |
| G8 | 断线与生命周期 | 不自动重连，睡眠唤醒后要手动重开；最后一个窗口关闭后的行为没有定义 | 断开后只提示“关闭此窗口后重新打开” |
| G9 | 错误不分类 | 密码错误、账户锁定、远程桌面未开启、权限不足都显示成“RDP 连接失败” | `bridge_thread` 只调用 `notify(..., "RDP 连接失败")` |
| G10 | 证书要手动核对 | 第一次使用必须比对 95 位指纹，大部分用户不会核对 | `certificateNotice` |
| G11 | Windows 端没有预检 | 远程桌面没开、端口改过、用户不在“远程桌面用户”组里，都要等到连接失败才知道 | `availability()` 只检查版本 |
| G12 | 应用发现不全，没有图标 | 开始菜单、用户级安装、Store 应用都发现不了；卡片上显示的都是同一个占位图标 | `launchable_apps`；`appButton` 固定用 `macwindow` |
| G13 | 抢占本机会话没有提示 | 连接时 Windows 本机会被锁屏或被挤下线，用户事先不知道 | 无 |
| G14 | 没有声音 | Windows 应用的声音放不出来 | `audiomode:i:2`，`CHANNEL_RDPSND=OFF` |
| G15 | 注册表残留 | 自动写进 `TSAppAllowList` 的条目不会清理 | `register_app` 不做标记 |
| G16 | 测试基础 | Swift 测试目标本身编译不过，新代码的测试跑不起来 | `HostingEnvironmentTests.swift` 用了 `quietWorkspace`；`WifiPermissionTests.swift` 访问了 fileprivate 的 `pane` |

## 3. 关键架构决策

每项都给出推荐方案和理由。按项目规范，修改时不保留旧路径和兼容代码，两端必须一起升级。

### D1：RDP 流量改走 DeskFuse 的 QUIC 隧道（推荐，解决 G3、G10、G11）

```
Mac FreeRDP ──(socketpair fd)──▶ Rust 隧道泵 ──QUIC 流 RemoteRdpTunnel──▶ Windows 服务 ──TCP──▶ 127.0.0.1:<RDP 端口>
            TLS/NLA 端到端（DeskFuse 只转发密文）
```

- FreeRDP 3.14.1 的 `freerdp_set_io_callbacks` 支持替换 `TCPConnect`，它只需要返回一个 socket fd（已在 `include/freerdp/transport_io.h` 核对）。桥接层用 `socketpair(AF_UNIX, SOCK_STREAM)` 生成一对 fd，一端交给 FreeRDP，另一端交给 Rust 泵。这样不用在 Mac 上监听任何端口，本机其他进程也就无法冒用这个隧道。
- Windows 服务只接受这样的隧道流：对方是已配对、已信任、未阻止的设备，并且带着目录通道刚签发的一次性启动票据（30 秒有效，用一次即作废）。服务连接的是本机回环地址，端口从注册表 `WinStations\RDP-Tcp\PortNumber` 读，不写死 3389。
- 好处：
  - 不需要为局域网开放 3389 防火墙。
  - 线路切换由 DeskFuse 现有的路由迁移处理，RDP 连接本身感觉不到。
  - IPv6 也能用。
  - 证书可以自动固定：Windows 服务读取 RDP 监听证书（`Win32_TSGeneralSetting.SSLCertificateSHA1Hash` 所指的 `Remote Desktop` 存储中的证书），算出 SHA-256，按 FreeRDP 的格式通过已认证通道发给 Mac。Mac 只接受这个指纹，不再弹出人工核对对话框。证书不一致就直接拒绝，不提供手动放行。
- 代价：多一跳转发；RDP 的 UDP 传输不可用，改用 TCP 语义。局域网内这点影响可以接受，P4 阶段会测量确认。
- 删除的旧代码：`{host}` 替换、`windowsHost()`、`peerHost`、人工指纹核对界面，以及 `RemoteRDPCredentialStore` 中保存指纹的部分（改为每次由服务端下发）。

### D2：每台 Windows 只保留一条 RDP 连接，多个应用共用（解决 G1）

- 新增 `RemoteRdpHostConnection`，每台 Windows 一个，由 `RemoteApplicationManager` 持有。它负责桥接层的生命周期、RAIL 状态和窗口表。
- 第一个应用通过 `.rdp` 里的 `remoteapplicationprogram` 启动；之后的应用由 RAIL 的 `ClientExecute("||<alias>")` 在同一个会话里启动。启动前，Mac 先通过目录通道发出 `prepare`，服务端按需完成注册后回复 `ready`。
- 远端最后一个窗口关闭后，连接再空闲 30 秒就断开。远端应用全部退出后，Windows 会自己结束 RemoteApp 会话。

### D3：远端每个窗口对应一个 Mac 原生窗口（解决 G2）

- 接管 `rdpWindowUpdate` 的 `WindowCreate/Update/Delete/Icon/CachedIcon` 和 `MonitoredDesktop` 回调。在 C 层维护窗口表：ID、所属窗口、样式、标题、位置大小、可见区域、图标。表的变化通过事件队列交给 Swift。
- 远端桌面尺寸等于 Mac 所有屏幕合起来的外接矩形（像素）。每个 Mac 窗口从共享画面里裁出自己的那块区域，并按可见区域设置窗口形状。
- 窗口外观：
  - 普通窗口用无边框的 `NSWindow`，标题栏由 Windows 自己绘制。
  - 弹出菜单、提示框等带所属窗口的弹出类窗口用不激活的 `NSPanel`，层级放在所属窗口之上，没有阴影。
- 拖动和缩放：用户在远端标题栏上按下时，服务器会发 `ServerLocalMoveSize`，Mac 本地执行窗口拖动或缩放，结束后发 `ClientWindowMove`。
- 状态同步：
  - 激活、最小化、最大化、关闭分别对应 `ClientActivate`、`ClientSystemCommand` 的 `SC_MINIMIZE/SC_MAXIMIZE/SC_RESTORE/SC_CLOSE`。
  - `ServerMinMaxInfo` 用来限制 Mac 窗口的最小和最大尺寸。
  - `ServerZOrderSync` 和 `ServerCloak` 用来同步层级和隐藏状态。
- 工作区：通过 `ClientSystemParam(SPI_SETWORKAREA)` 把 Mac 主屏扣除菜单栏和 Dock 后的区域告诉远端，这样 Windows 应用最大化时不会盖住 Mac 的菜单栏。

### D4：剪贴板在 RDP 期间只走 cliprdr（解决 G4）

- Windows 进入远程会话后，DeskFuse 桌面端会按现有规则停止剪贴板同步。所以 RDP 期间 Mac 和这台 Windows 之间的剪贴板唯一走 `cliprdr`，两条路径不会同时生效。
- 支持的格式：文本（`CF_UNICODETEXT`）、HTML（`HTML Format`）、RTF、图片（`CF_DIB`、PNG），以及双向的文件复制（`FileGroupDescriptorW` 和 `FileContents`，使用 FreeRDP 的 `cliprdr_file_context`）。
- 采用延迟渲染：只有对方真正粘贴时才传输内容。单次传输上限沿用 DeskFuse 剪贴板的 16 MiB 规则；文件不受这个上限约束，但要支持取消。
- Mac 端的 DeskFuse 剪贴板同步在这期间对这台 Windows 暂停发送，避免 RDP 结束后旧内容被回灌到 Windows。

### D5：键盘默认用“Mac 输入法”模式（解决 G5）

- 默认模式“Mac 输入法”：
  - 可以输入的字符走 `NSTextInputClient`，以 Unicode 事件发送，包括代理对和中文候选词上屏。
  - 不产生字符的键（方向键、功能键、Esc、Tab、退格、回车）和组合键走扫描码。
- 可选模式“Windows 输入法”：所有键都走扫描码，由 Windows 输入法处理。适合游戏和依赖按键位置的软件。
- ⌘ 的映射（设置项，默认开启）：
  - ⌘ 与字母、数字组合时映射为 Ctrl。
  - ⌥ 映射为 Alt，⌃ 保持为 Ctrl。
  - 单独按下再松开 ⌘ 映射为 Win 键。
  - ⌘⇥ 和 ⌘Space 保留给 macOS，不发送到远端。
- 窗口获得焦点时发送 `freerdp_input_send_synchronize_event`，同步 Caps Lock 和 Num Lock 状态。

## 4. 分阶段实施

每个阶段的退出条件都是“自动测试通过，并且本阶段的实机项通过”。阶段顺序就是依赖顺序。工作量是一名熟悉代码库的工程师的估算。

### P0 前置：修复测试基础（1 天）

1. 修复 Swift 测试目标的编译错误：
   - `HostingEnvironmentTests.swift` 改用 `dfOpaque`（改名来自 `abeac920`）。
   - `WifiPermissionTests.swift` 改为通过可测试的接口访问 `pane`，不放宽访问权限。
2. 把 `./scripts/build-freerdp-macos.sh --test` 和 RemoteApplication 包的 `swift test` 加入 `./scripts/check.sh`，单项测试超时 60 秒。
3. 准备 Windows 测试机清单：Windows 11 Pro 24H2、Windows 10 Pro 22H2、Windows Server 2022。每台记录是否加域、本地账户还是微软账户。

退出条件：`./scripts/check.sh` 全部通过，并且包含 RDP 相关的测试。

### P1 隧道与连接模型（D1 + D2，6 天）

Rust
- `deskfuse-protocol`：新增流类型 `RemoteRdpTunnel`。首帧是 `{ticket}`，之后是原始字节分块，每块不超过 64 KiB。双方都只保留有限大小的缓冲，满了就等待。
- `deskfuse-runtime` 和 `deskfuse-ffi`：新增 `open_remote_rdp_tunnel(peer, ticket, fd)`，由 Rust 接管 fd，在 Tokio 中双向复制。任何一端关闭，另一端也关闭，并上报关闭原因。
- `apps/deskfuse-service`：
  - 目录协议升到 v2。目录里增加 `host`（预检结果、证书指纹、本机会话占用情况）。新增请求 `prepare{name}`，回复 `ready{ticket, rdp}`。v1 请求直接返回 `unsupported_version`，提示用户升级双方。
  - 隧道服务端：校验票据（绑定设备、30 秒有效、只能用一次），然后连接 `127.0.0.1:<PortNumber>`。
  - 授权被撤销时，立刻关闭所有隧道。

C 桥
- 改成用 `freerdp_set_io_callbacks` 替换 `TCPConnect`，返回 socketpair 的一端；把另一端通过新接口 `rdp_bridge_take_tunnel_fd()` 交给上层。
- 证书校验只接受服务端下发的指纹。删除用户手动信任的流程，以及 `ConfigPath` 证书存储目录。

Swift
- 新增 `RemoteRdpHostConnection`（每台 Windows 一个），替代现在“每个应用一个 `RemoteRdpSession`”的做法。
- 删除 `peerHost`、`windowsHost()`、`trustCurrentCertificate` 和证书提示界面。

测试
- Rust：
  - 票据过期、重复使用、换设备使用都会被拒绝。
  - 隧道一端关闭时另一端也关闭。
  - 背压：Windows 端暂停读取时，Mac 端写入会阻塞而不是无限占内存。
- C：用 socketpair 加一个回环 TLS 测试服务器，验证 `TCPConnect` 的替换路径。

实机
- Windows 防火墙对局域网关闭 3389，仍然能连上。
- 依次打开记事本和画图，两个都能正常工作，也没有互相挤掉。
- 拔掉雷电线、改走 Wi-Fi，RDP 不断开（或在 P5 的自动重连时间内恢复）。

### P2 无缝窗口（D3，10 天）

C 桥
- 窗口表和事件队列：用一个有上限的环形队列装窗口事件、光标事件和错误事件。Swift 用一个信号量唤醒的读取线程来取，不再用定时器轮询。
- 画面：`EndPaint` 只拷贝本次的脏区（`gdi->primary->hdc->hwnd->invalid` 和 `cinvalid`）到一块 IOSurface 支撑的共享缓冲；按脏区通知 Swift。
- 远端桌面尺寸：连接前根据 Mac 屏幕设置 `DesktopWidth/Height`、`DesktopScaleFactor`、`DeviceScaleFactor` 和 `MonitorDefArray`。

Swift
- `RemoteRdpWindowController`：管理一个远端窗口对应的 `NSWindow` 或 `NSPanel`。内容层用 `CALayer` 指向共享的 IOSurface，再通过 `contentsRect` 裁出本窗口区域。窗口形状由可见区域生成，使用 `CAShapeLayer` 遮罩。
- 坐标换算放在纯函数 `RemoteRdpGeometry` 里：Mac 全局坐标（点，y 轴向上，多屏）和远端桌面像素之间互相换算。
- 本地拖动和缩放、最小化、最大化、关闭、激活，以及层级同步。
- 菜单栏增加“Windows 应用”菜单，Dock 菜单也列出当前所有远端窗口。
- 删除 `RemoteRdpWindow`、`RemoteRdpCanvasView` 这套单画布实现。

测试
- C：给窗口表喂合成的 `WINDOW_ORDER_INFO` 序列（创建、更新部分字段、删除、所属窗口关系），检查最终状态；字段越界或数量超限时拒绝。
- Swift：
  - `RemoteRdpGeometry` 覆盖单屏、左右双屏、上下双屏、主屏不在原点、混合缩放这几种布局。
  - 用可见区域生成窗口形状。

实机
- 记事本：右键菜单、“另存为”对话框、多个窗口。
- 画图：拖动、缩放、最大化后不盖住 Mac 菜单栏。
- Chrome：标签页拖出成新窗口。
- Office（如果有）：功能区菜单、弹出提示。
- 两块屏幕之间拖动窗口。

### P3 输入完善（D5，5 天）

- 键盘：
  - 把扫描码表补全：F1–F24、小键盘、Help/Insert、ISO 布局的 § 键、JIS 的英数、假名、¥ 和 _ 键。
  - 实现 Mac 输入法模式和 Windows 输入法模式，以及 ⌘ 映射规则。
  - 窗口失去焦点时释放所有按下的键。现在已经有这个逻辑，要扩展到多窗口：焦点在远端窗口之间切换时不释放，切换到非远端窗口才释放。
- 鼠标：
  - 修正滚轮：按 120 为一格累积精确滚动的增量，用 `PTR_FLAGS_WHEEL_NEGATIVE` 加 0–255 的幅度发送。
  - 支持水平滚轮（`PTR_FLAGS_HWHEEL`），侧键 4/5（`freerdp_input_send_extended_mouse_event`）。
- 光标：接管 `Pointer_New/Free/Set/SetNull/SetDefault/SetPosition`，把远端光标转成 `NSCursor` 并缓存。远端隐藏光标时，Mac 也隐藏。

测试
- Swift：
  - 扫描码表与 Windows 实际键位的对照表（来自 Windows 文档）逐项比对。
  - 滚轮累积：精确滚动的小增量、反向、超大增量都要正确。
  - ⌘ 映射规则：单独按 ⌘、⌘ 加字母、系统保留的组合。
- C：光标位图从 XOR/AND 格式转成 RGBA 的边界用例。

实机
- 中文输入法输入词组，并在 Word 或记事本里上屏。
- ⌘C、⌘V、⌘Z、⌘S。
- 在 Windows 输入法模式下切换 Windows 微软拼音。
- 触控板双指水平滚动。
- 文本框中显示 I 形光标，拖动窗口边缘时显示调整大小的光标。

### P4 剪贴板、声音与画质（D4，7 天）

- 剪贴板：按 D4 实现格式映射、延迟渲染和文件复制双向传输；RDP 期间暂停 Mac 端 DeskFuse 剪贴板对这台 Windows 的同步。
- 声音：构建时打开 `CHANNEL_RDPSND` 和 FreeRDP 自带的 macOS 播放后端（AudioQueue，不引入新的第三方库）；`.rdp` 改为 `audiomode:i:0`。设置里提供开关，默认开启。
- 画质：
  - 打开 RDPGFX 的 progressive/RemoteFX/planar 编码（不需要 H.264）。
  - 用 `disp` 通道在 Mac 屏幕布局变化（接上或拔掉显示器、改分辨率）时调用 `SendMonitorLayout`。
- H.264（AVC420/444）：FreeRDP 3.14.1 没有 VideoToolbox 解码后端，接入要自己写解码器。本阶段**不做**，原因是局域网带宽下 progressive 编码已经够用。这是否满足要求，以第 9 节的性能指标为准；如果达不到，再单独立项。

测试
- C：
  - 剪贴板格式映射：HTML 片段头的偏移、`CF_DIB` 与 PNG 互转、文件描述符的编码与解码。
  - 拒绝超大内容和格式错误的描述符。
- Rust 和 Swift：RDP 期间 DeskFuse 剪贴板对这台 Windows 暂停、结束后恢复的状态机。

实机
- 文本、带格式文本、截图在两个方向都能复制粘贴。
- 从 Finder 复制 1 GB 文件，粘贴到 Windows 资源管理器，中途可以取消；反方向也测一遍。
- 播放视频时声音和画面同步。
- 接上外接显示器后，远端分辨率跟着变化。

### P5 生命周期、可靠性与错误处理（5 天）

- 自动重连：开启 FreeRDP 自动重连（使用服务器下发的重连 cookie）。隧道断开后，在 DeskFuse 重新连上设备的 60 秒内重新建隧道并重连 RDP，窗口按 RAIL 状态恢复；超时后关闭窗口并给出提示。
- 睡眠与唤醒：睡眠前主动断开，唤醒后自动重连。
- 错误分类：把 `freerdp_get_last_error` 映射成明确的提示，每条都给出用户下一步可以做什么：
  - 登录失败、账户被锁、密码过期、需要修改密码。
  - 没有远程登录权限、远程桌面未开启、许可证问题。
  - 服务器拒绝连接、隧道被撤销。
- 凭据：
  - 支持 `域\用户` 和 `用户@域`；微软账户按 `MicrosoftAccount\邮箱` 处理。
  - 登录失败时弹出重新输入的对话框；成功后再更新钥匙串，失败的凭据不保存。
  - 桥接层释放凭据前先清零内存。
  - 把应用选择器里“必须先填用户名和密码”的限制改成“第一次连接时弹出登录框”。
- 本机会话占用提示：Windows 服务通过 `WTSGetActiveConsoleSessionId` 和 `WTSQuerySessionInformation` 上报本机当前是否有人登录、是哪个用户。Mac 在连接前提示“连接后 Windows 本机会锁屏”，或“会让当前登录的其他用户断开连接”，由用户确认后再继续。
- 撤销信任、关闭远程应用功能开关、设备被阻止时，立刻断开所有 RDP 连接。

测试
- Swift：
  - 错误码映射是一张完整的表，每个码都有提示文案。
  - 重连状态机：重连成功、超时、在重连过程中被撤销信任。
- Rust：上报本机会话占用情况的数据结构和解析。

实机
- 输错密码，重新输入后能登录。
- 合盖 5 分钟后打开，自动恢复。
- 断开 Wi-Fi 30 秒后恢复，自动恢复；断开 2 分钟，给出明确提示。
- Windows 本机已登录时连接，会先看到提示。

### P6 Windows 服务端完善（5 天）

- 预检：检查结果放进目录的 `host.checks`。Mac 逐项显示，并给出修复方法。
  - `fDenyTSConnections`、`TermService` 服务是否在运行、`UserAuthentication`（是否要求 NLA）、端口号。
  - 当前用户是否在 Administrators 或 Remote Desktop Users 组。
  - Windows 版本是否支持。
- 一键开启远程桌面：只能在 Windows 本机由用户点击触发。Windows 桌面端通过已认证的 IPC 请求服务修改 `fDenyTSConnections`，并启动 `TermService`。不允许从 Mac 远程触发，因为这会扩大 Windows 的暴露面。
- 应用发现：
  - 增加“所有用户”和当前活动用户的开始菜单 `.lnk`（解析目标、参数和图标），以及用户级安装的应用（HKCU 下的 `App Paths`，从活动用户的注册表加载）。
  - Microsoft Store/UWP 应用：先在 P0 的测试机上实测它们在 RemoteApp 下能否启动（例如通过 `explorer.exe shell:AppsFolder\<AUMID>` 启动）。能稳定启动才放进目录；不能就在目录中明确标注“不支持”，不默默丢掉。
- 图标：从可执行文件或 `.lnk` 中提取 64×64 PNG，单个不超过 16 KiB，在目录里以 base64 传输并缓存；Mac 的应用卡片显示真实图标。
- 注册表清理：DeskFuse 自动写入的 `TSAppAllowList` 条目都加上 `DeskFuseManaged=1` 标记。在应用已卸载、路径失效，或用户关闭远程应用功能时清理。用户手动注册的条目不动。

测试
- Rust：
  - 预检结果的组合。
  - `.lnk` 解析的边界：相对路径、环境变量、网络路径会被拒绝。
  - 图标大小超限时不放进目录。
  - 清理只删除带标记的条目。

实机
- 在全新安装的 Windows 11 Pro 上，从安装 DeskFuse 到在 Mac 上打开第一个应用，记录每一步。
- 卸载一个应用后，它从目录里消失，对应的注册表项也被清除。

### P7 界面、设置与诊断（4 天）

- 应用选择器：
  - 显示真实图标、预检状态和本机会话占用提示。
  - 去掉卡片上内嵌的凭据输入框，改成第一次连接时弹出登录框；钥匙串选项放进这个登录框。
- 设置页（远程应用 > Windows）：
  - 键盘模式、⌘ 映射、声音开关、剪贴板开关。
  - “为当前设备忘记登录信息”。
- 诊断：在 `diagnostics-*.jsonl` 中增加 `rdp_*` 事件，内容不包含密码、剪贴板内容和窗口标题：
  - 各连接阶段的耗时、隧道字节数、错误码、窗口数量、帧率和每帧耗时。
- 文档：重写 `WINDOWS_REMOTEAPP_INTEGRATION.md` 和 `WINDOWS_RDP_CATALOG.md`（协议 v2），更新 `CHANGELOG.md`。

## 5. 明确不在本次范围内的内容

| 内容 | 理由 |
| --- | --- |
| Windows 控制 Mac 的应用 | macOS 没有 RDP 服务端，需要另一套方案（复用 Mac 对 Mac 的视频协议），单独立项 |
| H.264 硬件解码（VideoToolbox） | 理由见 P4；性能不达标时再立项 |
| 打印机、智能卡、USB、串口、驱动器重定向 | 文件交换已由剪贴板文件复制和 DeskFuse 文件传输覆盖；其余需求在当前用户场景里没有出现 |
| 麦克风输入（`audin`） | DeskFuse 已有麦克风共享。在 RDP 会话里，Windows 的音频设备属于远程会话，是否需要 `audin` 要在 P4 实测 DeskFuse 麦克风在远程会话中能否使用后再定；如果不能用，就把 `audin` 纳入 P4 |
| RD 网关、Kerberos 单点登录、多主机集群 | 只面向点对点、已配对的 Windows |

## 6. 风险与应对

| 风险 | 应对 |
| --- | --- |
| 替换 `TCPConnect` 后，FreeRDP 内部仍有地方假设拿到的是 TCP socket（例如读 `getpeername`、调 `setsockopt`） | P1 第一天先用 socketpair 做最小验证；如果不行，改用 `TransportAttach` 或 `AttachLayer` 的自定义传输层。这两个接口都已在 3.14.1 头文件中确认存在 |
| RAIL 窗口与 Mac 窗口之间的焦点、层级来回同步造成闪烁 | 所有同步都以服务器为准，本地操作只发请求；在 P2 的实机矩阵里加入快速切换焦点的测试 |
| Windows 11 的圆角和阴影让可见区域变复杂 | 窗口形状完全按服务端下发的可见区域生成，不在本地猜测 |
| Retina 屏下远端桌面像素过多（如 5K 屏的两倍） | 设置像素总量上限：超过 8192×8192 或 3300 万像素时，按比例降到上限内，`DesktopScaleFactor` 也相应降低，并在诊断中记录 |
| Store 应用无法以 RemoteApp 方式启动 | P6 先实测，再决定是否放进目录，不做静默降级 |
| 本地缺少真实 Windows，无法自动化端到端测试 | 端到端只能靠实机矩阵；每个阶段结束都在第 9 节表格里更新结果 |

## 7. 里程碑

| 里程碑 | 包含阶段 | 累计估算 | 可以对外说的状态 |
| --- | --- | --- | --- |
| M1 连接可靠 | P0 + P1 | 7 天 | 多个应用共用一条连接，不需要开放防火墙，证书自动可信 |
| M2 无缝窗口 | P2 + P3 | 22 天 | 远端窗口与 Mac 窗口一一对应，键盘鼠标完整可用 |
| M3 功能完整 | P4 + P5 | 34 天 | 剪贴板、声音、自动重连、错误提示齐全 |
| M4 可发布 | P6 + P7 + 验收 | 43 天 + 实机验收 | 去掉 Beta 标记的候选版本 |

## 8. 每个阶段的检查命令

```sh
source scripts/rust-toolchain-env.sh
git diff --check
cargo check --locked
./scripts/check.sh                       # 包含 fmt、clippy、Rust 测试、原生构建
./scripts/build-freerdp-macos.sh --test  # C 桥回归测试
./scripts/build-native-macos.sh debug
swift test --package-path apps/deskfuse-macos/RemoteApplication   # 单项测试超时 60 秒
```

Windows 服务端的改动还要在 Windows 上运行 `cargo test --locked -p deskfuse-service` 和 `scripts/package-windows.ps1`。

## 9. 完成定义（DoD）

全部满足才能算“100% 完成”。每一行都要有测试名或实机记录（日期、机器、系统版本、结果）。

### 9.1 自动测试

| 项 | 要求 |
| --- | --- |
| 隧道 | 票据校验、背压、两端关闭联动、授权撤销 |
| 目录协议 v2 | 序列化往返；拒绝 v1；超大请求；未知应用 |
| 窗口表 | 创建、更新、删除、所属关系，越界拒绝 |
| 坐标与形状 | 5 种多屏布局，混合缩放 |
| 输入 | 扫描码表全覆盖；滚轮累积；⌘ 映射；光标转换 |
| 剪贴板 | 每种格式的双向映射；拒绝超限内容 |
| 错误映射 | 每个 FreeRDP 错误码都有提示文案 |
| Windows 服务 | 预检组合、`.lnk` 解析、图标限制、注册表清理只删带标记的条目 |

### 9.2 实机验收矩阵

覆盖：Windows 11 Pro 24H2、Windows 10 Pro 22H2、Windows Server 2022 × Apple Silicon Mac（另抽测一台 Intel Mac）× 单屏 Retina、双屏混合缩放。

| # | 场景 | 通过标准 |
| --- | --- | --- |
| A1 | 全新 Windows 从安装到打开第一个应用 | 不需要手动改防火墙或注册表；预检提示能引导完成所有设置 |
| A2 | 同时打开 3 个应用 | 互相不挤掉；每个窗口都能独立移动、缩放、最小化、关闭 |
| A3 | 菜单、对话框、提示框、拖出的标签页 | 位置正确，层级正确，没有黑块或残影 |
| A4 | 中文输入、快捷键、侧键、水平滚动、光标形状 | 与本机使用一致 |
| A5 | 剪贴板：文本、带格式文本、图片、1 GB 文件双向复制 | 内容完整；大文件可以取消 |
| A6 | 声音 | 能听到，延迟不影响看视频 |
| A7 | 雷电与 Wi-Fi 切换、断网 30 秒、合盖 5 分钟 | 自动恢复，窗口还在 |
| A8 | 密码错误、账户锁定、远程桌面未开启、用户不在远程桌面用户组 | 各自给出明确提示和下一步操作 |
| A9 | Windows 本机已有人登录 | 连接前有提示，确认后才连接 |
| A10 | 撤销信任或关闭功能开关 | 1 秒内断开所有 RDP 窗口 |
| A11 | 性能 | 1080p 窗口拖动时画面不低于 30 fps；Mac 端空闲时 CPU 低于 3%，持续编辑时低于 15%（M1 机型） |
| A12 | 稳定性 | 连续使用 8 小时，内存不持续增长，不崩溃 |

### 9.3 发布

- `python3 scripts/release.py validate` 通过。
- macOS 两种架构的 DMG 都携带并链接 RDP 静态库，没有非系统动态库依赖。
- Windows 安装器包含新服务。
- 签名、公证、Release 和 Homebrew cask 按 `AGENTS.md` 中的发布顺序完成并分别确认。
