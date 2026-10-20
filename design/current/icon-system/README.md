# DeskFuse · 屏幕排列

2026-10-20 按用户「重新设计一下图标和安装界面」要求更新，替换 2026-10-11 的「共享控制桥」方案。本方案已接入现有资源生成链路。

![图标系统预览](preview.png)

## 设计

- **应用图标**：三块屏幕按实际使用时的显示器排列边贴边放置、互不重叠：远端设备屏（信号橙 `#F2582A` 系）在左上，本机外接屏在其右侧并略微下沉，本机内建屏在远端屏下方、右边缘对齐，用排列本身表达“多台电脑拼成一张桌面”，不再使用光标元素。尺寸相对实际分辨率做了优化：两块外接屏同为约 16:10，内建屏约为其 0.8 倍宽、3:2 比例，屏间留 14 px 细缝。精修细节：每块屏幕有极轻的纵向明暗、顶部细菜单栏和上沿高光，下方是环境阴影 + 接触阴影两层柔影；不使用霓虹渐变、毛玻璃或光晕。浅色版为暖白底 + 石墨色本机屏（ICNS 使用此版），深色版为石墨底 + 纸白本机屏。
- **菜单栏图标**：同一排列的单色版本：远端屏实心，本机两块屏描边，屏间约 1 pt 间隙；18 pt 下光标过于拥挤，因此不画光标。18 pt 逻辑尺寸、36 px @2x，颜色交给系统 template 渲染。
- **安装界面（DMG）**：920×560 pt 暖白画布，顶部为产品名与一句话说明，两侧白色落位卡片分别承载 DeskFuse 与「应用程序」，中间灰色点线 + 橙色箭头指明拖拽方向，下方中英文操作说明。背景以 1x + 2x 多分辨率 TIFF 打包，Retina 屏不再模糊。
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
| DMG 背景 SVG 源文件 | `resources/dmg-background.svg` |
| DMG 背景 1x/2x TIFF | `resources/generated/dmg-background.tiff` |
| DMG Finder 布局 | `scripts/package-macos.sh`（图标中心 (250, 262) / (670, 262)，图标 128 pt） |
| 派生透明中心标识 | `resources/macos_center_icon.svg` |
| 原生菜单栏接入 | `apps/deskfuse-macos/Sources/DeskFuse/DeskFuseApp.swift` |

仓库根目录执行 `./scripts/generate-assets.sh` 可重建全部图标。打包与开发运行脚本已复制这些资源；本次不改变包标识、签名选择或配对身份。

上方为资源预览，不是菜单栏运行截图。浅深背景是静态模板着色示意；实际系统外观、选中态和辅助功能显示尚待实机验收。交付证据见 `artifacts/validation/2026-09-12-icon-redesign/README.md`。
