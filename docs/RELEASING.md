# 发行 uniDesk

## 发行种类

| 产物 | 用途 |
| --- | --- |
| `dist/uniDesk-arm64.dmg` | macOS 14+ Apple Silicon 安装包；GitHub 预发行在配置凭据后为 Developer ID + 公证；贡献者 Homebrew 仍可 ad-hoc |
| `dist/release.json` | 版本、单调构建号和唯一发行 ID；与 bundle 内副本一致 |
| `dist/build-info.json` | 源 commit、工具链、架构、签名状态和 DMG 哈希 |
| `dist/SHA256SUMS` | 上述三个文件的 SHA-256；校验和本身不是发行签名 |
| `dist/uniDesk-arm64-update.zip` | 保留应用签名的原始 LAN 更新包；接收端按签名清单校验，不重新打包 |
| `dist/manifest.json` | 本机协调端签发的 Ed25519 发布清单；CI 没有私钥时不生成 |

根 `release.json` 是公开且随版本提交的输入，不含凭据。`dist/` 是输出，不提交到 Git。

## 首次启用 GitHub

源码与 CI 位于私有仓库 `GodD6366/uniDesk`；公开仓库 `GodD6366/unidesk-release` 只维护下载说明、Homebrew cask 和 Release 安装包。原源码仓库直接改为私有，原提交、tag、Release 和 Apple secrets 保持不变。公开安装包仓库使用独立的说明文档历史，不复制源码对象。

原仓库已有的六项 [公证凭据](NOTARIZE.md) 无需重新设置。跨仓库发布新增 `RELEASE_REPOSITORY_TOKEN`：仅授予公开仓库 Contents read/write 的 fine-grained PAT 或 GitHub App token。仓库自带的 `GITHUB_TOKEN` 无权写另一个仓库。迁移保留原仓库身份及 secrets，不导出密钥。

源码 tag 仅推送到私有仓库。发布工作流在公开仓库 main 的发行说明提交上创建同名 tag，绝不向公开仓库推送私有源码或源码 tag；`build-info.json` 的 commit 仍记录真正的私有源码 SHA。所有产物先校验、上传 draft、下载复核，再公开。cask job 在公开仓库更新 `Casks/unidesk.rb`。

迁移不重新推送原有源码 tag，不重跑历史发行。首次跨仓库发布前需配置 `RELEASE_REPOSITORY_TOKEN`，且按现有流程运行全部 CI 门禁。

## 准备版本（协调端，仅一次）

1. 修改根 Cargo.toml 的 workspace 版本为 `X.Y.Z`；所有自有包继承版本。同步 CHANGELOG，运行检查。当前初版是 `0.1.0`。
2. 首次准备运行 `./scripts/prepare-release.sh`。如果根目录已经有该版本的 descriptor，直接复用，不再运行分配器。
3. 准备**下一版本**时，确认版本变更后显式移除旧 `release.json`，再运行 prepare。不要为重试删除 descriptor。
4. 该命令调用现有 allocator，ledger 默认位于 `$HOME/Library/Application Support/uniDesk-release/build-ledger`，也可设置 `UNIDESK_RELEASE_LEDGER`。ledger 必须在仓库之外，应备份；失败编号不回收，不能重置或并用多个协调端。
5. 检查并提交版本、CHANGELOG、lockfile（仅在需要时）和新 `release.json`。不提交 ledger、发行私钥或运行证据。

`build_number` 当前受 CFBundleVersion 限制为 1..9999；到达上限时应先改造并验证编号格式，不得回绕。只支持 `vX.Y.Z` tag；测试性质通过 GitHub prerelease 标志表示，暂不支持带 `-rc` 后缀的 tag。

## 授权开发者独立 Homebrew 包

需要完整 Xcode 27+（macOS 27+ SDK）、仓库固定 Rust 工具链、`rust-src` 和 arm64 target。最低运行目标仍为 macOS 14。

```sh
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- UNIDESK_DMG_FINDER_LAYOUT=0 ./scripts/package-macos.sh --homebrew
./scripts/verify-dmg.sh dist/homebrew/uniDesk-arm64.dmg
```

本地输出默认位于 `dist/homebrew/`，使用已提交的根 `release.json`，不分配维护者构建号、不读取 LAN 发行私钥、不投递运行中应用的自动更新。显式设置 `DIST_DIR` 可以改输出目录。Homebrew 不是 LAN 自动安装包；`manifest.json` 不生成，也不保留旧清单。所有发行包仍须记录实际签名、公证和验收边界。

## 维护者本地构建／验证

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

无 Finder 模式仍包含应用与 Applications 链接，但不设置自定义 Finder 窗口位置。普通本机包默认写入 `dist/{version}-build{number}/`；显式 `DIST_DIR`、CI 产物目录和本地 Homebrew 的 `dist/homebrew/` 不变。普通本机包默认使用 Finder 布局；本地开发签名选择保持不变。公开预发行由 CI 导入 Developer ID 并公证，不要再对 tag 工作流设 `CODESIGN_IDENTITY=-`。

打包会重新生成图标，并把自有／第三方许可证和依赖声明放进 bundle 的 Resources。`verify-dmg.sh` 检查实际挂载的 app、arm64、元数据与签名；`UNIDESK_REQUIRE_NOTARIZE=1` 时额外要求 Developer ID、hardened runtime 和已 staple 的票据。两机行为仍须实机验收。

Intel Mac 目前只提供本机实验性 DMG，不加入公开 arm64 Release，也不生成当前仅支持 arm64 的 LAN 更新清单。使用相同的发行描述构建 x86_64 包：

```sh
UNIDESK_MACOS_ARCH=x86_64 CODESIGN_IDENTITY=- UNIDESK_DMG_FINDER_LAYOUT=0 \
  DIST_DIR="$PWD/dist/0.3.0-intel" RELEASE_METADATA="$PWD/release.json" \
  ./scripts/package-macos.sh --homebrew
UNIDESK_MACOS_ARCH=x86_64 UNIDESK_DMG_FINDER_LAYOUT=0 \
  ./scripts/verify-dmg.sh dist/0.3.0-intel/uniDesk-x86_64.dmg
```

该包包含 x86_64 主程序、输入服务和更新 helper，默认使用 GitHub 更新渠道；签名、公证和 Intel 设备行为仍需单独验收。

只有从已提交、与产物一致的源码构建后，才能生成对外构建信息：

```sh
python3 scripts/release.py inventory --commit "$(git rev-parse HEAD)"
(cd dist && shasum -a 256 -c SHA256SUMS)
```

## GitHub 预发行流程

准备工作完成并决定公开发布时，由维护者提交、推送源码，再创建和推送同名 tag。以下为维护者明确决定创建预发行时使用的操作（普通源码 push 不会创建 Release）：

```sh
git tag -a v0.1.0 -m 'uniDesk 0.1.0 prerelease'
git push origin main
git push origin v0.1.0
```

1. preflight 验证 tag／workspace／descriptor；已发布同 tag 时下载并核对全部资产、版本和 source commit，成功后不重建或覆盖。
2. 调用相同 CI：macOS 检查、Windows 原生适配器和服务检查均成功才继续。
3. `xcode-27` Apple Silicon runner 使用已提交 descriptor 和固定 Rust target，导入 Developer ID、`--homebrew` + headless 打包、hardened runtime、`notarytool` 公证并 staple，验证 DMG，再生成四件套。缺少 Apple secret 时失败。
4. publish job 负责建 draft、上传、下载校验；全部通过后转为 prerelease，不标记为 latest。
5. cask job 在 publish 确认资产公开后刷新 Homebrew cask 并提交到默认分支（见下）。publish 与 cask 是仅有的两个 `contents: write` job。

当前 `xcode-27` 是 GitHub 的预览镜像，用于满足已使用的 SDK 27 API；发布前须确认该镜像可用并检查实际运行结果。

Actions 产物保留 14 天；GitHub Release 资产不按此期限过期。首次远端运行必须独立检查结果，不能拿本地通过替代。Windows 不产出公开安装包。

### Homebrew cask

公开安装包仓库同时作为 Homebrew tap（`brew tap godd6366/unidesk https://github.com/GodD6366/unidesk-release`）。`Casks/unidesk.rb` 固定 `uniDesk-arm64.dmg` 的 SHA-256，只服务公开 GitHub 预发行，不改变 LAN 自动更新。

cask job 下载刚发布的 DMG、重新渲染 cask 并提交到公开安装包仓库的 main。该 job 需要默认分支允许其 token 直接推送；分支保护会阻止提交，此时在本地手动刷新并提交：

```sh
gh release download v0.1.0 --repo GodD6366/unidesk-release --pattern uniDesk-arm64.dmg --dir dist
python3 scripts/update-cask.py --dmg dist/uniDesk-arm64.dmg --version 0.1.0
git add Casks/unidesk.rb
git commit -m 'chore(cask): pin v0.1.0 DMG checksum'
```

`update-cask.py` 拒绝与根 `Cargo.toml` 版本不一致的输入；cask 版本或校验和未刷新时 `brew install` 会报校验失败。Homebrew 7 要求用户先 `brew trust --cask godd6366/unidesk/unidesk` 才能 tap，README 已列出该步骤。

## 失败、重试与撤回

- 检查／打包失败：不创建 Release，修复源码后使用新版本发布；同一源码的瞬时失败可以重新运行工作流。
- 上传／发布失败留下 draft：核对该 draft 尚未公开，检查其资产后删除这个**未公开草稿**，再重跑同一 tag；descriptor 不变。脚本不会自动覆盖草稿或删除资产。
- 同 tag 已公开：只校验已有资产；冲突则失败，不覆盖、不移动 tag。修复应用需新版本和新发行号。
- 撤回有问题的预发行版：由维护者取消其公开发行并在说明中标明原因；不自动降级用户、不删除用户身份。已下载旧包的行为需另行沟通。

GitHub CI 用 Actions secrets 持有 Developer ID 与公证凭据，不持有 LAN 发行私钥，不发布自动安装用的签名清单。本机签名与 LAN 分发流程见下文；Apple 凭据操作见 [公证凭据](NOTARIZE.md)。


## 本机 LAN 自动更新

本机协调端的发行私钥固定存于 `~/Library/Application Support/uniDesk-release/signing-key.hex`（0600），公钥固定编译自 `resources/unidesk-release-public-key.hex`。不要重新生成或自动轮换；密钥不随源码或 DMG 分发。`UNIDESK_RELEASE_KEY` 仅用于指定同一发行密钥的位置。

`package-macos.sh` 构建并嵌入 `unidesk-update-helper`，先保留现有 Apple Development 签名，再生成 ZIP、签署并验证发布清单。DMG 内 `.updates/` 保留原始 ZIP 和清单；包完整性验证通过后，完整资源目录原子发布到 `~/Library/Application Support/uniDesk/updates/inbox/`。运行中的正式应用自动导入并向已配对的物理局域网设备分发。开发包 build 0 不充当安装目标。

首次升级到支持此协议的基础包，需要每台 Mac 手动安装一次。之后没有更新开关或逐次确认：下载、验签、安全点安装和重启自动进行；在「设置 → 传输与存储 → 版本与自动更新」查看本机进度、对端版本和真实安装回执。版本列表尚未返回时显示未取得状态。

安装器在旧进程退出后备份应用及 SQLite/WAL/SHM，验证相同应用签名要求后替换。新进程先独立运行数据库迁移与 SwiftUI 窗口自检，60 秒内通过后才启动正常业务；失败恢复旧应用和数据库，并隔离该发行。网络断开不会撤销本机成功安装，回执会在重连后重试。应用必须位于可写目录；DMG 或无写权限时保留原安装并展示错误。

`updates/` 是应用自有独立目录，更新缓存限额 4 GiB，保护当前安装、候选和正在发送的包。回执包含事务、版本、build、release ID、目标和 SHA-256，与已发送任务精确匹配；跨重启复用缓存或本机导入时，另以当前安装库存和本地已验签原始包共同核对。恢复流程不删除或重建设备身份与配对数据。
