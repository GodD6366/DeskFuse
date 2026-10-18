# DeskFuse 独立产品切换

DeskFuse 从 0.4.0 起使用独立的应用身份、协议、存储和发行命名。
不实现旧产品协议、安装身份、资产名称或数据目录的兼容回退。
旧产品最后一版的源码保留在 `v0.3.5` 标签，当前源码不包含其升级提示。

## 新安装边界

- macOS Bundle ID 为 `com.deskfuse.app`，应用为 `DeskFuse.app`。
- 数据保存在 `~/Library/Application Support/DeskFuse`，日志使用独立目录。
- Windows 使用 `DeskFuseService` 和新的管道、数据目录及安装器标识。
- 用户需要重新授权、重新配对设备和配置偏好。首次启动检测到旧安装或数据库时提示导入历史；用户确认后才导入。
- 导入在引擎启动之前完成，备份和事务保护现有记录，不复用旧设备身份。已完成导入时直接提供旧应用清理选项。
- 导入成功后可以单独确认将旧应用移到废纸篓；只处理 Bundle ID 匹配的应用包，保留旧数据和备份。取消或失败不删除应用。
- 旧安装名称仅保存在 `config/previous-installation.json`，随包提供给启动检测，协议和普通业务模块不含旧名称。
- 控制协议主版本为 2，ALPN 为 `deskfuse/2`；签名域、Bonjour 和 TLS 名称同步切换。
- 仓库与发行配置来自 `config/deskfuse.json` 和 `config/distribution.json`。

## 一次性剪贴板历史导入

先运行一次 DeskFuse 以创建数据库，再退出源应用和 DeskFuse：

```sh
python3 scripts/import-clipboard-history.py \
  --source "$PREVIOUS_DATA_DIRECTORY" \
  --destination "$HOME/Library/Application Support/DeskFuse"
```

工具取得两边的实例锁，使用 SQLite backup API 保存含 WAL 的一致快照，
再在单个事务中合并历史、归档列表和内容。相同全局 ID 保留目标记录，
本地数字 ID 重新分配；完成标记防止重复导入。失败回滚，源库始终不变。
备份位于目标数据目录的 `history-import-backups`，含私人剪贴板内容，请妥善保管。

仅支持匹配的数据库结构；存在文件归档资源表时拒绝导入，避免产生不可用的文件引用。
未下载的远端内容只能保留索引。此工具不迁移设备密钥、信任关系、系统权限或同步游标，
不自动修改其他产品管理的文件路径，也不删除旧数据。

## 历史接收文件

首次启动导入同时将旧文件缓存复制到 `~/Library/Application Support/DeskFuse/imported-files/clipboard/`。
逐文件校验 SHA-256 后，按存储格式重写剪贴板路径和归档 Blob 引用，并记录独立的文件迁移标记。
已完成数据库导入的安装仍会补做此步骤；失败可重试，不覆盖不同内容的目标文件，不删除来源。
文件保存在持久数据目录，不进入普通缓存清理。用户已粘贴到其他目录的文件保持原位。
传输任务队列仅存在内存中，不能恢复已退出进程的任务列表；已被旧版清理的文件不能恢复。

## 发布前提

新发行仓库必须实际存在后才能发布或宣称下载可用。仓库改名、remote 切换、
官网部署和 Homebrew 安装入口需要一并完成；本分支不创建公开仓库或发布新版本。
新安装不能以旧产品的自动更新测试结果代替验收。

关键字检查器接受外部传入的禁用名称，仅允许上述明确的迁移配置文件，不豁免整个目录，不使用隐藏编码：

```sh
python3 scripts/check-branding.py --forbidden "$RETIRED_BRAND"
```

`RETIRED_BRAND` 由检查者或 CI 仓库变量提供。检查涵盖文件路径、文本、二进制
可见字符串和符号链接目标，不扫描 Git 历史或忽略的旧构建缓存。
