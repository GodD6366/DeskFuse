#!/bin/bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
echo '请先在 DeskFuse 设置中关闭麦克风共享。仅卸载 DeskFuse 自有驱动（含旧版名称遗留的驱动）。'
read -r -p '输入 UNINSTALL 确认卸载，或直接回车取消：' ANSWER
[[ "$ANSWER" == UNINSTALL ]] || exit 0
sudo /bin/bash "$HERE/manage-driver.sh" uninstall
read -r -p '现在重启音频服务？会短暂中断当前声音。[y/N] ' ANSWER
if [[ "$ANSWER" == y || "$ANSWER" == Y ]]; then sudo /usr/bin/killall coreaudiod; else echo '请稍后重启 Mac。'; fi
read -r -p '按回车关闭此窗口。' ANSWER
