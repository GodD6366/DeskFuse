# Windows 图标资源

`DeskFuse.ico` 复用当前 `resources/generated/DeskFuse-icon-1024.png`，不重新设计图案。ICO 包含 16、20、24、32、40、48、64、128、256 px 九种 PNG/RGBA 表示，保留透明外缘。

- `DeskFuse.Windows.csproj` 的 `ApplicationIcon` 嵌入 EXE 图标，同时将 ICO 编译为 WPF Resource。
- `AppIcon` 提供窗口及系统托盘图标。托盘使用现有彩色标记及浅色底面，不直接使用依赖 macOS template 自动着色的黑色菜单栏图，因此无需猜测 Windows 浅深主题。
- 安装器可以直接使用本目录的 `DeskFuse.ico`；安装快捷方式也可以使用生成后的 `DeskFuse.Windows.exe` 索引 0。
- `icons-manifest.json` 记录原 PNG 与 ICO 的 SHA-256 及全部尺寸。

在仓库根目录运行 `python3 scripts/generate-windows-icons.py` 可重新进行格式转换（需要 Pillow）。格式检查和构建不等于真实 Windows 资源管理器、任务栏或系统托盘视觉验收。
