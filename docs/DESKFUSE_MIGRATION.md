# DeskFuse 独立产品切换

DeskFuse 从 0.4.0 起使用独立的应用身份、协议、存储和发行命名。
不实现旧产品协议、安装身份、资产名称或数据目录的兼容回退。
旧产品最后一版的源码保留在 `v0.3.5` 标签，当前源码不包含其升级提示。

## 新安装边界

- macOS Bundle ID 为 `com.deskfuse.app`，应用为 `DeskFuse.app`。
- 数据保存在 `~/Library/Application Support/DeskFuse`，日志使用独立目录。
- Windows 使用 `DeskFuseService` 和新的管道、数据目录及安装器标识。
- 用户需要重新授权、重新配对设备和配置偏好；历史剪贴板不自动导入。
- 不读取、移动或删除另一产品的数据，也不复用其设备身份。
- 控制协议主版本为 2，ALPN 为 `deskfuse/2`；签名域、Bonjour 和 TLS 名称同步切换。
- 仓库与发行配置来自 `config/deskfuse.json` 和 `config/distribution.json`。

## 发布前提

新发行仓库必须实际存在后才能发布或宣称下载可用。仓库改名、remote 切换、
官网部署和 Homebrew 安装入口需要一并完成；本分支不创建公开仓库或发布新版本。
新安装不能以旧产品的自动更新测试结果代替验收。

关键字检查器接受外部传入的禁用名称，不包含品牌特例、兼容目录豁免或隐藏编码：

```sh
python3 scripts/check-branding.py --forbidden "$RETIRED_BRAND"
```

`RETIRED_BRAND` 由检查者或 CI 仓库变量提供。检查涵盖文件路径、文本、二进制
可见字符串和符号链接目标，不扫描 Git 历史或忽略的旧构建缓存。
