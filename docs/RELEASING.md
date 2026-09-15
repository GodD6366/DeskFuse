# 发行 uniDesk

## 发行种类

| 产物 | 用途 |
| --- | --- |
| `dist/uniDesk-arm64.dmg` | macOS 14+ Apple Silicon 安装包；GitHub bootstrap 为 ad-hoc，维护者本机保留其签名；均未公证 |
| `dist/release.json` | 版本、单调构建号和唯一发行 ID；与 bundle 内副本一致 |
| `dist/build-info.json` | 源 commit、工具链、架构、签名状态和 DMG 哈希 |
| `dist/SHA256SUMS` | 上述三个文件的 SHA-256；校验和本身不是发行签名 |
| `dist/uniDesk-arm64-update.zip` | 保留应用签名的原始 LAN 更新包；接收端按签名清单校验，不重新打包 |
| `dist/manifest.json` | 本机协调端签发的 Ed25519 发布清单；CI 没有私钥时不生成 |

根 `release.json` 是公开且随版本提交的输入，不含凭据。`dist/` 是输出，不提交到 Git。

## 首次启用 GitHub

现有工作流用于 `GodD6366/uniDesk`，不依赖 Apple 凭据或外部构建服务器。维护者确认仓库公开文件清单后，自行启用 Actions、private vulnerability reporting，并按实际协作需要设置 main 检查规则和发行 tag 管理。初期可限制 tag 创建／更新给维护者；不要强制要求个人项目必须有第二位审批人。

工作流不会修改仓库 public/private 状态，也不会自动推送 tag。私有仓库的 Actions 运行使用账户可用额度；实际额度与设置以 GitHub 为准。

## 准备版本（协调端，仅一次）

1. 修改根 Cargo.toml 的 workspace 版本为 `X.Y.Z`；所有自有包继承版本。同步 CHANGELOG，运行检查。当前初版是 `0.1.0`。
2. 首次准备运行 `./scripts/prepare-release.sh`。如果根目录已经有该版本的 descriptor，直接复用，不再运行分配器。
3. 准备**下一版本**时，确认版本变更后显式移除旧 `release.json`，再运行 prepare。不要为重试删除 descriptor。
4. 该命令调用现有 allocator，ledger 默认位于 `$HOME/Library/Application Support/uniDesk-release/build-ledger`，也可设置 `UNIDESK_RELEASE_LEDGER`。ledger 必须在仓库之外，应备份；失败编号不回收，不能重置或并用多个协调端。
5. 检查并提交版本、CHANGELOG、lockfile（仅在需要时）和新 `release.json`。不提交 ledger、发行私钥或运行证据。

`build_number` 当前受 CFBundleVersion 限制为 1..9999；到达上限时应先改造并验证编号格式，不得回绕。只支持 `vX.Y.Z` tag；测试性质通过 GitHub prerelease 标志表示，暂不支持带 `-rc` 后缀的 tag。

## 贡献者独立 bootstrap 包

需要完整 Xcode 27+（macOS 27+ SDK）、仓库固定 Rust 工具链、`rust-src` 和 arm64 target。最低运行目标仍为 macOS 14。

```sh
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- UNIDESK_DMG_FINDER_LAYOUT=0 ./scripts/package-macos.sh --bootstrap
./scripts/verify-dmg.sh dist/bootstrap/uniDesk-arm64.dmg
```

本地输出默认位于 `dist/bootstrap/`，使用已提交的根 `release.json`，不分配维护者构建号、不读取 LAN 发行私钥、不上传 NAS、不投递运行中应用的自动更新。显式设置 `DIST_DIR` 可以改输出目录。bootstrap 不是 LAN 自动安装包；`manifest.json` 不生成，也不保留旧清单。所有发行包仍须记录实际签名、公证和验收边界。

## 维护者本地构建／验证

Python 3.11+；已安装仓库固定 Rust 工具链及 aarch64-apple-darwin target。

```sh
python3 scripts/release.py validate --tag v0.1.0
CODESIGN_IDENTITY=- \
RELEASE_METADATA="$PWD/release.json" \
UNIDESK_DMG_FINDER_LAYOUT=0 \
./scripts/package-macos.sh
UNIDESK_DMG_FINDER_LAYOUT=0 ./scripts/verify-dmg.sh dist/uniDesk-arm64.dmg
```

无 Finder 模式仍包含应用与 Applications 链接，但不设置自定义 Finder 窗口位置。普通本机包默认使用 Finder 布局；开发签名选择保持不变，公开预发行必须显式设 `CODESIGN_IDENTITY=-`。

打包会重新生成图标，并把自有／第三方许可证和依赖声明放进 bundle 的 Resources。`verify-dmg.sh` 检查实际挂载的 app、arm64、元数据与签名；不证明 Apple 公证或原生两机行为。

本机每次成功打包后，自动归档 DMG、原始 update ZIP、签名 `manifest.json`、`release.json` 和 `SHA256SUMS` 到
`/Volumes/Share/MAC软件/uniDesk/uniDesk-{version}-build{build_number}/`。
归档先写入临时目录，核对复制后的 SHA-256 后才发布版本目录；相同内容可重复执行，冲突不会覆盖已有包。
NAS 未挂载或复制失败时命令报错，本地 `dist/` 产物保留；挂载后可用
`python3 scripts/archive-macos.py` 补传，无须重新构建或分配构建号。
CI 不执行 NAS 归档；其他本机可通过 `UNIDESK_NAS_DIR` 指定归档根目录，或显式设置
`UNIDESK_NAS_ARCHIVE=0` 仅打本地包。

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
3. `xcode-27` Apple Silicon runner 使用已提交 descriptor 和固定 Rust target，显式 `--bootstrap` + headless + ad-hoc 打包，验证 DMG，再生成四件套。
4. publish job 负责建 draft、上传、下载校验；全部通过后转为 prerelease，不标记为 latest。
5. cask job 在 publish 确认资产公开后刷新 Homebrew cask 并提交到默认分支（见下）。publish 与 cask 是仅有的两个 `contents: write` job。

当前 `xcode-27` 是 GitHub 的预览镜像，用于满足已使用的 SDK 27 API；发布前须确认该镜像可用并检查实际运行结果。

Actions 产物保留 14 天；GitHub Release 资产不按此期限过期。首次远端运行必须独立检查结果，不能拿本地通过替代。Windows 不产出公开安装包。

### Homebrew cask

公开仓库同时作为 Homebrew tap（`brew tap godd6366/unidesk https://github.com/GodD6366/uniDesk`）。`Casks/unidesk.rb` 固定 `uniDesk-arm64.dmg` 的 SHA-256，只服务公开 GitHub 预发行，不改变 NAS 归档或 LAN 自动更新。

cask job 下载刚发布的 DMG、重新渲染 cask 并提交到默认分支。该 job 需要默认分支允许其 token 直接推送；分支保护会阻止提交，此时在本地手动刷新并提交：

```sh
gh release download v0.1.0 --repo GodD6366/uniDesk --pattern uniDesk-arm64.dmg --dir dist
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

GitHub CI 仍不持有 Apple 凭据或 LAN 发行私钥，不发布自动安装用的签名清单。Apple 公证尚未接入。本机签名与 LAN 分发流程见下文。


## 本机 LAN 自动更新

本机协调端的发行私钥固定存于 `~/Library/Application Support/uniDesk-release/signing-key.hex`（0600），公钥固定编译自 `resources/unidesk-release-public-key.hex`。不要重新生成或自动轮换；密钥不随源码、DMG 或 NAS 上传。`UNIDESK_RELEASE_KEY` 仅用于指定同一发行密钥的位置。

`package-macos.sh` 构建并嵌入 `unidesk-update-helper`，先保留现有 Apple Development 签名，再生成 ZIP、签署并验证发布清单。DMG 内 `.updates/` 保留原始 ZIP 和清单；验证和 NAS 归档通过后，完整资源目录原子发布到 `~/Library/Application Support/uniDesk/updates/inbox/`。运行中的正式应用自动导入并向已配对的物理局域网设备分发。开发包 build 0 不充当安装目标。

首次升级到支持此协议的基础包，需要每台 Mac 手动安装一次。之后没有更新开关或逐次确认：下载、验签、安全点安装和重启自动进行；在「设置 → 传输与存储 → 版本与自动更新」查看本机进度、对端版本和真实安装回执。版本列表尚未返回时显示未取得状态。

安装器在旧进程退出后备份应用及 SQLite/WAL/SHM，验证相同应用签名要求后替换。新进程先独立运行数据库迁移与 SwiftUI 窗口自检，60 秒内通过后才启动正常业务；失败恢复旧应用和数据库，并隔离该发行。网络断开不会撤销本机成功安装，回执会在重连后重试。应用必须位于可写目录；DMG 或无写权限时保留原安装并展示错误。

`updates/` 是应用自有独立目录，更新缓存限额 4 GiB，保护当前安装、候选和正在发送的包。回执包含事务、版本、build、release ID、目标和 SHA-256，与已发送任务精确匹配；跨重启复用缓存或本机导入时，另以当前安装库存和本地已验签原始包共同核对。恢复流程不删除或重建设备身份与配对数据。
