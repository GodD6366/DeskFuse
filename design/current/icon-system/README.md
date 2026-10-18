# DeskFuse · 共享控制桥

2026-10-11 按用户「重新设计软件图标和状态栏图标」要求更新。本方案已接入现有资源生成链路。

![图标系统预览](preview.png)

## 设计

- **应用图标**：提供深色和浅色两套底面，分别适配系统外观；两个错位的发光桌面面板、中央共享输入光标和融合节点共同表达跨设备控制。
- **菜单栏图标**：保留两个错位面板的单色轮廓，并加入简化光标，18 pt 逻辑尺寸、36 px @2x；实际颜色交给系统 template 渲染。
- **维护方式**：原生 SVG 矢量设计，无生成式位图或外部图片依赖。应用图与状态栏轮廓分别维护，不从彩色图缩小得到菜单栏图标。

## 资源与接入

| 用途 | 文件 |
| --- | --- |
| 深色应用 SVG 源文件 | `resources/DeskFuse-icon.svg` |
| 浅色应用 SVG 源文件 | `resources/DeskFuse-icon-light.svg` |
| 菜单栏 SVG 源文件 | `resources/deskfuse-status-icon.svg` |
| 应用 1024 px PNG | `resources/generated/DeskFuse-icon-1024.png` |
| 浅色应用 1024 px PNG | `resources/generated/DeskFuse-icon-light-1024.png` |
| 原生 macOS ICNS | `resources/generated/DeskFuse.icns` |
| 菜单栏 36 px PNG | `resources/generated/deskfuse-status-icon.png` |
| 派生透明中心标识 | `resources/macos_center_icon.svg` |
| 原生菜单栏接入 | `apps/deskfuse-macos/Sources/DeskFuse/DeskFuseApp.swift` |

仓库根目录执行 `./scripts/generate-assets.sh` 可重建全部图标。打包与开发运行脚本已复制这些资源；本次不改变包标识、签名选择或配对身份。

上方为资源预览，不是菜单栏运行截图。浅深背景是静态模板着色示意；实际系统外观、选中态和辅助功能显示尚待实机验收。交付证据见 `artifacts/validation/2026-09-12-icon-redesign/README.md`。
