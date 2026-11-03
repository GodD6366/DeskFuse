# Windows 麦克风驱动包

Windows 驱动包必须包含：

- 已签名的 `DeskFuseMicrophone.inf` 及其同目录文件；
- `manifest.json`，声明 `schema=1`、INF 文件名和 SHA-256；
- 发布时使用 `install-driver.ps1` 安装，使用 `uninstall-driver.ps1` 卸载。

安装脚本只接受管理员 PowerShell，先校验 manifest 和 INF 的 SHA-256，再调用 `pnputil /add-driver /install`，最后重启 `Audiosrv`。卸载同样通过 `pnputil`，不直接删除系统驱动目录。

当前仓库还没有 Windows 虚拟音频驱动的 `.inf/.sys/.cat` 二进制；这些文件必须由签名构建产物提供，不能用脚本伪造。 

Roc sender 使用独立的 Windows CMake 工程构建：

```powershell
./scripts/build-roc-microphone-windows.ps1 -RocRoot C:\roc\install
```

该工程只要求 Roc 的 `include/` 和 `lib/`，不会引入 macOS CoreAudio、HAL 或 gRPC 驱动依赖。
