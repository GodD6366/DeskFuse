# 远程窗口菜单、红绿灯与快捷键

本地菜单移到窗口顶部居中，悬停时显示；菜单按钮旁保留本地窗口拖动柄，避免遮挡远端应用右上角的功能。菜单支持键盘打开，包含画质、刷新率、显示比例、打开其他应用及窗口操作。

左上角常驻 AppKit 标准红绿灯，使用不透明底面覆盖远端控制标记。红色关闭 A 上的远程窗口，不退出 B 应用；黄色最小化并暂停该窗口采集；绿色按当前屏幕可用区域等比最大化，再次操作还原。多远程窗口各自处理这些动作。

| 快捷键 | 本地动作 |
| --- | --- |
| Control + Option + Command + Space | 打开菜单 |
| Control + Option + Command + W | 关闭远程窗口 |
| Control + Option + Command + M | 最小化 |
| Control + Option + Command + F | 最大化 / 还原 |
| Control + Option + Esc | 释放控制，恢复鼠标共享 |

窗口在画布转发快捷键之前拦截这些专用组合，并释放远端已按下的按键状态。普通 Command 快捷键继续交给远端应用。忽略长按重复事件，并按事件时间去重；最小化或打开弹窗导致抬键发生在其他窗口时，不影响下次按下同一快捷键。

## 验证范围

依照用户要求只完成官方构建、固定本地签名及 DMG 完整性检查，不运行本机交互测试。双机待验收：覆盖标记的位置、三色按钮及还原尺寸、最小化后再次使用相同快捷键、菜单与远端输入焦点切换、普通远端快捷键和退出控制后恢复鼠标共享。

交付 `0.2.3 build240`：`/Users/godd/workspace/uniDesk/dist/0.2.3-build240/uniDesk-arm64.dmg`。官方打包脚本退出 0，原生构建、签名与 `verify-dmg.sh` 检查通过。签名为 Apple Development: changchun.dcc@gmail.com (5GM2LLJFA6)，Team UNZ234UP68；未公证。SHA-256：`8ca122022970d16bc36a8b60b7f382b8d8bbce79826dda3e5de682a31854f5ef`。构建记录：`artifacts/remote-application/traffic-lights-shortcuts-package-final.log`。
