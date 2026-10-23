# 发行 DeskFuse

## 发行种类

| 产物 | 用途 |
| --- | --- |
| `dist/DeskFuse-arm64.dmg` | macOS 14+ Apple Silicon 磁盘映像；Homebrew 与只认该文件名的旧客户端使用。GitHub 预发行在配置凭据后为 Developer ID + 公证；贡献者 Homebrew 仍可 ad-hoc |
| `dist/DeskFuse-x86_64.dmg` | macOS 14+ Intel 磁盘映像；与 arm64 使用同一发行描述，签名、公证和设备行为单独记录。Homebrew cask 不使用它 |
| `dist/DeskFuse-windows-x64-setup.exe` | Windows x64 NSIS 安装包；发布 job 要求 Authenticode 有效签名，安装、SCM、GUI 和双机行为单独记录 |
| `dist/release.json` | 版本、单调构建号和唯一发行 ID；与 bundle 内副本一致 |
| `dist/build-info.json` | 源 commit、工具链、macOS/Windows 目标、签名状态和资产哈希 |
| `dist/SHA256SUMS` | 上述发行资产的 SHA-256；校验和本身不是发行签名 |
| `dist/DeskFuse-arm64-update.zip` | 签过名的 `DeskFuse.app`。官网下载这个包，打开应用后走安装窗口；局域网设备和已安装的正式版用它直接替换应用，不再显示安装窗口。公证成功后会把票据打进这个 zip。已发布的旧客户端只认同一发布里的 DMG，所以 DMG 继续上传 |
| `dist/DeskFuse-x86_64-update.zip` | Intel 版同一个应用包。官网 Intel 下载用它；当前自动更新只在 Apple Silicon 上静默安装 |
| `dist/manifest.json` | 本机协调端签发的 Ed25519 发布清单；CI 没有私钥时不生成 |

根 `release.json` 是公开且随版本提交的输入，不含凭据。`dist/` 是输出，不提交到 Git。

## 首次启用 GitHub

源码、CI、Release 安装包、Homebrew cask（`Casks/`）和站点（`web/`）都在公开仓库 `GodD6366/DeskFuse`。Actions 触发与权限必须遵守 [参与开发](../CONTRIBUTING.md#ci-与-actions-规则) 中的规则。

六项 [公证凭据](NOTARIZE.md) 放在 `release` environment。publish、cask 两个 job 用本仓库 `GITHUB_TOKEN` 的 job 级 `contents: write` 发布 Release、提交 cask；pages job 用 `actions: write` 在 main 上触发站点部署。`RELEASE_REPOSITORY_TOKEN` 只在过渡期用于镜像到旧发行仓库。

Release 直接挂在源码 tag 上；`build-info.json` 的 commit 记录源码 SHA。所有产物先校验、上传 draft、下载复核，再公开。cask job 在 main 更新 `Casks/deskfuse.rb` 和 `web/src/config/distribution.json`，随后 pages job 部署站点。

### 发行仓库合并过渡

已安装客户端在编译时写死了发行仓库地址（`crates/deskfuse-engine/build.rs` 读取 `config/distribution.json`），v0.7.0 及更早的客户端只查询 `GodD6366/deskfuse-release`。因此：

1. `config/distribution.json` 保留 `legacy_release_repository` 期间，发布流程的 `mirror-legacy` job 把同一批资产镜像到旧仓库，cask job 给旧 tap 的 cask 加上重新 tap 的提示，站点清单合并两个仓库。
2. 过渡版本发布并确认旧客户端能检测到更新后：删除 `legacy_release_repository`，重新生成站点配置（`python3 scripts/distribution-config.py --website-output web/src/config/distribution.json`），提交后归档旧仓库。旧仓库的 Release 下载链接在归档后仍然有效。
3. 站点自定义域名从旧仓库移到本仓库时，先在旧仓库 Pages 移除 `deskfuse.godd.cc`，再在本仓库设置；DNS 仍指向 `godd6366.github.io`，无需修改。

合并不重新推送原有源码 tag，不重跑历史发行。本仓库里 v0.1.0–v0.2.3 的早期 Release 保持不变，客户端按版本号取最高版本，不受影响。

## 准备版本（协调端，仅一次）

1. 修改根 Cargo.toml 的 workspace 版本为 `X.Y.Z`；所有自有包继承版本。同步 CHANGELOG，运行检查。当前初版是 `0.1.0`。
2. 首次准备运行 `./scripts/prepare-release.sh`。如果根目录已经有该版本的 descriptor，直接复用，不再运行分配器。
3. 准备**下一版本**时，确认版本变更后显式移除旧 `release.json`，再运行 prepare。不要为重试删除 descriptor。
4. 该命令调用现有 allocator，ledger 默认位于 `$HOME/Library/Application Support/DeskFuse-release/build-ledger`，也可设置 `DESKFUSE_RELEASE_LEDGER`。ledger 必须在仓库之外，应备份；失败编号不回收，不能重置或并用多个协调端。
5. 检查并提交版本、CHANGELOG、lockfile（仅在需要时）和新 `release.json`。不提交 ledger、发行私钥或运行证据。

`build_number` 当前受 CFBundleVersion 限制为 1..9999；到达上限时应先改造并验证编号格式，不得回绕。只支持 `vX.Y.Z` tag；测试性质通过 GitHub prerelease 标志表示，暂不支持带 `-rc` 后缀的 tag。

## 授权开发者独立 Homebrew 包

需要完整 Xcode 27+（macOS 27+ SDK）、仓库固定 Rust 工具链、`rust-src` 和 arm64 target。最低运行目标仍为 macOS 14。

```sh
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- DESKFUSE_DMG_FINDER_LAYOUT=0 ./scripts/package-macos.sh --homebrew
./scripts/verify-dmg.sh dist/homebrew/DeskFuse-arm64.dmg
```

本地输出默认位于 `dist/homebrew/`，使用已提交的根 `release.json`，不分配维护者构建号、不读取 LAN 发行私钥、不投递运行中应用的自动更新。显式设置 `DIST_DIR` 可以改输出目录。Homebrew 不是 LAN 自动安装包；`manifest.json` 不生成，也不保留旧清单。所有发行包仍须记录实际签名、公证和验收边界。

## 维护者本地构建／验证

### 双架构手动安装包

```sh
./scripts/package-macos.sh --release
```

使用本机现有签名证书，依次构建 arm64 和 Intel x86_64 安装包。默认只分配一次新构建号和发行 ID，两个包共用同一描述，输出到 `dist/{version}-build{number}/`。目录包含两个 DMG、对应 ZIP、`release.json`、`.signing-arm64`、`.signing-x86_64` 和合并的 `SHA256SUMS`。已有产物拒绝覆盖；两个架构都验证成功后才汇总产物。

此模式写入 `update-channel=manual`，应用不启动 GitHub 或 LAN 更新器，仅支持手动安装更新。它不会创建 GitHub Release，也不会自动安装本机应用。签名沿用 `.codesign-identity` / `CODESIGN_IDENTITY`；需要 ad-hoc 时显式使用 `CODESIGN_IDENTITY=- ./scripts/package-macos.sh --release`。本机 Apple Development 签名不等于 Developer ID 公证。

可用 `RELEASE_METADATA=/绝对路径/release.json` 指定发行描述，或用失败输出目录中的描述重试；根目录 `release.json` 不会被自动读取或改写。`DIST_DIR` 可覆盖输出目录。`--release` 不与 `--lan`、`--homebrew`、`--auto`、`DESKFUSE_MACOS_ARCH` 或 `DMG_PATH` 混用。单独构建一个手动安装包可使用 `DESKFUSE_MACOS_ARCH=x86_64 ./scripts/package-macos.sh --manual`。

### 单架构局域网包

Python 3.11+；已安装仓库固定 Rust 工具链及 aarch64-apple-darwin target。

```sh
./scripts/package-macos.sh
```

`package-macos.sh` 是唯一打包入口：无参数或 `--lan` 固定生成局域网包（`update-channel=lan`），不由 `CI` 环境变量切换渠道。Homebrew/GitHub 包必须显式传入 `--homebrew`（`update-channel=github`）；CI 已显式指定。普通本地打包每次从协调端 ledger 分配新构建号与发行 ID，自动完成构建、签名和 DMG 完整性检查，不需要先运行 prepare，也不读取或修改根 `release.json`。语义版本取自 Cargo.toml；本地打包只递增 build。

新 descriptor 在编译前保存至 `dist/{version}-build{number}/release.json`。失败后可用该文件显式重试，例如：

```sh
RELEASE_METADATA="$PWD/dist/0.2.2-build218/release.json" \
./scripts/package-macos.sh
```

此命令仅适用于尚未生成 DMG 的失败构建。已有 DMG 时拒绝覆盖；需要新包就取消 `RELEASE_METADATA`，重新运行默认命令。失败编号不回收，因此构建号跳号本身不代表缺少流程。根 `release.json` 是公开发行输入，不代表最新本地构建；以打包输出目录中的 descriptor 和包内版本为准。`build_and_run.sh` 是开发运行入口，使用 build 0，不分配发行构建号。

### 本地快速构建（仅本机迭代）

```sh
./scripts/package-macos.sh --auto --fast
```

`--fast` 只接受本地 LAN 包（`--lan`/`--auto`），且与 `--homebrew`、`--release`、`CI=true` 互斥。它保留 release Rust 库、签名身份、LAN 更新清单和 seed，但：

- Swift 改用 `debug` 配置 + `-Xswiftc -O` 的增量编译，并使用独立的 `apps/deskfuse-macos/.build-fast/` 缓存，不影响 `build_and_run.sh` 的 debug 缓存或发布构建的 release 缓存；
- 默认跳过 DMG（`hdiutil`、Finder 布局、压缩和 `verify-dmg.sh`），改由 `scripts/verify-app.sh` 校验 app bundle 后直接安装；
- 需要 DMG 时追加 `--with-dmg`：跳过 Finder 布局并使用更快的压缩级别，安装会在压缩前完成。

快速模式的输出目录会写入 `build-mode=local-fast`，并在终端打印警告；该目录不得发布。首次切换会全量编译一次 Swift 缓存，之后只重编改动文件。完整发布仍必须使用不带 `--fast` 的默认流程。

### 构建缓存清理

构建失败时脚本只在检测到缓存损坏特征（ICE/SIGSEGV、`invalid metadata`、找不到 crate、Swift 构建数据库损坏等）时自动清理并重试；普通源码错误、签名、公证、网络或 Finder 失败不会清理缓存。手动维护入口：

```sh
./scripts/clean-build-cache.sh --rust --target arm64 --dry-run
./scripts/clean-build-cache.sh --swift --yes
./scripts/clean-build-cache.sh --all --dry-run
./scripts/clean-build-cache.sh --prune --yes   # 仅清 incremental 和旧 dist 目录
```

`--rebuild-rust` / `--rebuild-swift` 可在一次打包运行中强制重建对应产物。Rust 工具链、SDK、target 或部署目标变化时，脚本会依据缓存 key 自动丢弃旧产物，避免换 Xcode/Rust 后继续复用不兼容的缓存。

无 Finder 模式仍包含应用与 Applications 链接，但不设置自定义 Finder 窗口位置。普通本机包默认写入 `dist/{version}-build{number}/`；显式 `DIST_DIR`、CI 产物目录和本地 Homebrew 的 `dist/homebrew/` 不变。普通本机包默认使用 Finder 布局；本地开发签名选择保持不变。公开预发行由 CI 导入 Developer ID 并公证，不要再对 tag 工作流设 `CODESIGN_IDENTITY=-`。

打包会重新生成图标，并把自有／第三方许可证和依赖声明放进 bundle 的 Resources。`verify-dmg.sh` 检查实际挂载的 app、目标架构、元数据与签名；`DESKFUSE_REQUIRE_NOTARIZE=1` 时额外要求 Developer ID、hardened runtime 和已 staple 的票据。两机行为仍须实机验收。

Intel Mac 使用相同的发行描述构建 x86_64 包。Intel 包和公开 Release 不使用 LAN 更新清单：

```sh
DESKFUSE_MACOS_ARCH=x86_64 CODESIGN_IDENTITY=- DESKFUSE_DMG_FINDER_LAYOUT=0 \
  DIST_DIR="$PWD/dist/0.4.0-intel" RELEASE_METADATA="$PWD/release.json" \
  ./scripts/package-macos.sh --homebrew
DESKFUSE_MACOS_ARCH=x86_64 DESKFUSE_DMG_FINDER_LAYOUT=0 \
  ./scripts/verify-dmg.sh dist/0.4.0-intel/DeskFuse-x86_64.dmg
```

该包包含 x86_64 主程序、输入服务和更新 helper，默认使用 GitHub 更新渠道；签名、公证和 Intel 设备行为仍需单独验收。

只有在已提交、与两个 macOS 产物一致的源码上，发布 job 才生成完整对外构建信息。发布 job 会在合并 arm64 和 x86_64 资产后运行：

```sh
python3 scripts/release.py inventory --directory dist --commit "$(git rev-parse HEAD)"
(cd dist && sha256sum -c SHA256SUMS)
```

## GitHub 预发行流程

准备工作完成并决定公开发布时，由维护者提交、推送源码，再创建和推送同名 tag。以下为维护者明确决定创建预发行时使用的操作（普通源码 push 不会创建 Release）：

```sh
git push origin main
# 等待 main CI 通过后，在包含完整版本文件的最新提交上创建 tag。
git tag -a vX.Y.Z -m 'DeskFuse X.Y.Z prerelease'
git push origin vX.Y.Z
```

手动重试相同源码时，在 Actions 中选择已有的 `vX.Y.Z` tag，或运行 `gh workflow run release.yml --ref vX.Y.Z`。选择 `main` 等分支会在 preflight 立即失败；手动入口不会创建或移动 tag。

`scripts/check.sh` 与 macOS 构建入口都会通过 `scripts/rust-toolchain-env.sh` 选择仓库固定的 Rust 工具链；CI 也会将该工具链加入后续步骤的 PATH，避免 Homebrew 的 `cargo`/`rustc` 抢先。检查日志同时记录实际使用的 `rustc` 和 `cargo` 版本。

1. preflight 验证 tag／workspace／descriptor；已发布同 tag 时下载并核对全部资产、版本和 source commit，成功后不重建或覆盖。
2. 调用 macOS 和 Windows CI 检查。Windows job 在 `windows-2022` 上运行固定 Rust/MSVC、服务与目录测试，并构建 x64 服务/桌面输入适配器；发布 job 还必须使用 `WINDOWS_AUTHENTICODE_PFX_B64` / `WINDOWS_AUTHENTICODE_PFX_PASSWORD` 签署 Windows 安装包，缺少签名会 fail closed。
3. Self-hosted Apple Silicon runner 分别构建 arm64 和 x86_64 DMG。macOS jobs 使用已提交 descriptor、Developer ID、`--homebrew` + headless 打包、hardened runtime、`notarytool` 公证并 staple。缺少 Apple secret 时 macOS jobs 失败。
4. publish job 负责建 draft、上传、下载校验；全部通过后转为 prerelease，不标记为 latest。
5. cask job 在 publish 确认资产公开后刷新 Homebrew cask 并提交到默认分支（见下），pages job 随即部署站点。publish 与 cask 是仅有的两个 `contents: write` job，pages 只有 `actions: write`。

当前 `xcode-27` 是 GitHub 的预览镜像，用于满足已使用的 SDK 27 API；发布前须确认该镜像可用并检查实际运行结果。

Actions 产物保留 14 天；GitHub Release 资产不按此期限过期。首次远端运行必须独立检查结果，不能拿本地通过替代。Windows 安装、SCM、Explorer、剪贴板、键鼠和两机行为仍需独立验收；跨编译与 Authenticode 通过不等价于这些实机结论。

### Homebrew cask

本仓库同时作为 Homebrew tap（`brew tap godd6366/deskfuse https://github.com/GodD6366/DeskFuse`）。`Casks/deskfuse.rb` 固定 `DeskFuse-arm64.dmg` 的 SHA-256，只服务公开 GitHub 预发行。公开发行版运行时可以把本机应用打包传给局域网中的其他 GitHub 渠道设备，不依赖 LAN 发行私钥。

cask job 下载刚发布的 DMG、重新渲染 cask 并提交到本仓库的 main。该 job 需要默认分支允许其 token 直接推送；分支保护会阻止提交，此时在本地手动刷新并提交：

```sh
gh release download v0.4.0 --repo GodD6366/DeskFuse --pattern DeskFuse-arm64.dmg --dir dist
python3 scripts/update-cask.py --dmg dist/DeskFuse-arm64.dmg --version 0.1.0
git add Casks/deskfuse.rb
git commit -m 'chore(cask): pin v0.4.0 DMG checksum'
```

`update-cask.py` 拒绝与根 `Cargo.toml` 版本不一致的输入；cask 版本或校验和未刷新时 `brew install` 会报校验失败。Homebrew 7 要求用户先 `brew trust --cask godd6366/deskfuse/deskfuse` 才能 tap，README 已列出该步骤。

## 失败、重试与撤回

- 检查／打包失败：不创建 Release，修复源码后使用新版本发布；同一源码的瞬时失败可以重新运行工作流。
- 上传／发布失败留下 draft：核对该 draft 尚未公开，检查其资产后删除这个**未公开草稿**，再重跑同一 tag；descriptor 不变。脚本不会自动覆盖草稿或删除资产。
- 同 tag 已公开：只校验已有资产；冲突则失败，不覆盖、不移动 tag。修复应用需新版本和新发行号。
- 撤回有问题的预发行版：由维护者取消其公开发行并在说明中标明原因；不自动降级用户、不删除用户身份。已下载旧包的行为需另行沟通。

GitHub CI 用 Actions secrets 持有 Developer ID 与公证凭据。LAN 分发流程见下文；Apple 凭据操作见 [公证凭据](NOTARIZE.md)。


## 本机 LAN 自动更新

局域网更新不再使用 Ed25519 发行清单。每台 Mac 启动后把正在运行的 `DeskFuse.app` 打包到 `~/Library/Application Support/DeskFuse/updates/self-package/`，经已配对、同一物理局域网的可信连接通告给对端；对端只接受同一更新通道（`lan` 或 `github`）且版本更高的包，下载到 `updates/peer-packages/` 后核对 SHA-256。`package-macos.sh` 不再签署发布清单，App 与 DMG 也不再内嵌 `LanUpdate/`、`.updates/` 种子；开发包 build 0 不充当安装目标。

信任边界：任何已配对设备都能推送可安装的新版，因此只与自己控制的设备配对。安装器仍要求新包与当前应用的代码签名要求（同一证书身份与 bundle ID）和更新通道一致，临时签名或他人证书签署的包会被拒绝。

从签名清单协议升级到自打包协议的第一个版本，需要每台 Mac 手动安装一次（旧版只认签名清单）。之后没有更新开关或逐次确认：下载、校验、安全点安装和重启自动进行；在「设置 → 传输与存储 → 版本与自动更新」查看本机进度、对端版本和真实安装回执。版本列表尚未返回时显示未取得状态。

安装器在旧进程退出后备份应用及 SQLite/WAL/SHM，验证相同应用签名要求后替换。新进程先独立运行数据库迁移与 SwiftUI 窗口自检，60 秒内通过后才启动正常业务；失败恢复旧应用和数据库，并隔离该发行。网络断开不会撤销本机成功安装，回执会在重连后重试。应用必须位于可写目录；DMG 或无写权限时保留原安装并展示错误。

`updates/` 是应用自有独立目录，只保留本机当前版本的自打包和高于当前版本的已下载包；旧签名缓存（`cache/`、`inbox/`）在启动时删除。回执包含事务、版本、build、release ID、目标和 SHA-256，与已发送任务精确匹配；跨重启复用已下载包时，另以当前安装库存和本地包共同核对。恢复流程不删除或重建设备身份与配对数据。
