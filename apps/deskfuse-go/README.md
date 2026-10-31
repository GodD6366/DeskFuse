# DeskFuse WebView UI

以当前 macOS 界面为基准，使用 MyGo 系统 WebView、Vue 3、TypeScript 和 Vite 实现共享产品 UI。Go 负责窗口、生成的 TS IPC 与原生服务接线；本阶段不重写 Rust 业务。

## 构建和开发

在仓库根目录运行：

```sh
./scripts/build-mygo-ui.sh
# 输出 target/mygo-webview/deskfuse-mygo
npm --prefix apps/deskfuse-go/frontend run dev
DESKFUSE_UI_DEV_URL=http://127.0.0.1:5174 target/mygo-webview/deskfuse-mygo
```

脚本首次安装前端依赖，生成 `frontend/src/mygo.ts`、检查 TypeScript、构建 Vite 资源并嵌入 Go 客户端。不编译 Rust、音频库或 RDP 库，不接管正式签名、公证或更新流程。

- Go `1.27.3+`；MyGo 固定 `v0.7.1`。
- Vue 3 + TypeScript，沿用仓库官网相同的 Vite 技术栈，但应用前端与官网独立构建。
- macOS 使用系统 WKWebView；Windows 需要 WebView2，Linux 需要 WebKitGTK。框架兼容范围不等于 DeskFuse 实机验收范围。
- 当前独立运行只展示界面和明确的服务未连接状态，不提供模拟业务记录。

## 结构

```text
cmd/deskfuse/           MyGo WebView 窗口、资源嵌入、只读冒烟入口
frontend/src/          Vue 主窗口、页面、设置与辅助面板
frontend/src/mygo.ts   MyGo 自动生成的 TS 服务绑定
internal/webview/      Backend 方法和受控 blob handler
internal/bridge/       macOS 过渡 stdio 帧客户端
internal/winservice/   Windows 命名管道 v2 客户端、快照翻译与命令映射
internal/clipboard/    既有查询 schema 与状态回归
```

此前 `internal/ui` 是 Go 自绘探索代码，不再进入主程序。删除前保留其参考测试；最终交付是 WebView，不是两套 UI。

## 真实后端

macOS 宿主通过 `deskfuse-app --mygo-ui <客户端绝对路径>` 拉起子进程；仅 `DESKFUSE_MYGO_BRIDGE=stdio-v1` 启用过渡桥。所有日志走 stderr，stdout 只传四字节大端长度前缀 JSON 帧。绑定生成期间不打开协议管道。

Go Backend 暴露 `snapshot`、`hello`、`queryLibrary`、`mutateLibrary`、`invoke` 和 `content`。业务仍以原生服务为权威；未实现动作明确返回错误，不能把按钮或页面存在当成功能完成。

Windows 端不再使用 stdio 过渡层：`main` 在 `GOOS=windows` 时连接服务命名管道
`\\.\pipe\com.deskfuse.app.desktop.v2`（`DESKFUSE_WINDOWS_PIPE` 可覆盖），把同一套页面
接到 `desktop_protocol` v2 命令上。已接入范围与明确未实现范围见
[Windows 适配层](../../docs/MYGO_WINDOWS_ADAPTER.md)。

图片使用 `deskfuse://blob/thumbnail/<id>?size=240` 和 `deskfuse://blob/image/<id>`。只接受数值内容 ID，不接受任意文件路径；WebView 不接收图片 Base64。当前原生桥到 Go 的过渡链路仍有 Base64，不能称为端到端零拷贝。

数据库与身份仍由现有引擎持有。最终 UDS / Named Pipe v2 尚未替代 stdio 过渡层，不能把该阶段描述为守护进程切换完成。

## 验证

```sh
npm --prefix apps/deskfuse-go/frontend run build
go -C apps/deskfuse-go test ./...
go -C apps/deskfuse-go vet ./...
DESKFUSE_UI_SMOKE_OUTPUT="$PWD/target/mygo-webview/smoke" target/mygo-webview/deskfuse-mygo
```

最后一条启动真实 WKWebView，读取页面状态、切换设置、截图后退出；不请求权限、不修改业务数据。

真实引擎联调使用原生宿主：

```sh
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh build
DESKFUSE_UI_SMOKE_OUTPUT="$PWD/target/mygo-webview/smoke-real" \
  dist/DeskFuse-dev.app/Contents/MacOS/deskfuse-app \
  --mygo-ui "$PWD/target/mygo-webview/deskfuse-mygo"
```

`DESKFUSE_UI_SMOKE_OUTPUT` 会让子进程等桥接快照到达后逐页截图（剪贴板、设备、键鼠、传输、
远程应用、麦克风、鼠标、设置），并写出 `snapshot.json` 与服务响应，用于和 macOS 基准比对；
不带该变量时窗口保持打开供人工检查。冒烟不点击业务按钮，也不证明双设备行为。

完整清单与阻塞记录见 `docs/MYGO_UI_MIGRATION.md`。当前仍在迁移，不宣称与 macOS 全部页面和行为完全一致。
