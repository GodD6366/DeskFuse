#!/bin/bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/compatibility.sh" "$HERE/previous-installation.json"
echo '将安装 DeskFuse 独立麦克风驱动，需要管理员密码。原版 Roc VAD 与其它驱动不受影响。'
read -r -p '输入 INSTALL 确认安装，或直接回车取消：' ANSWER
[[ "$ANSWER" == INSTALL ]] || exit 0
sudo /bin/bash "$HERE/manage-driver.sh" install "$HERE/${MICROPHONE_BUNDLE_NAME}.driver"
read -r -p '现在重启音频服务？会短暂中断当前声音。[y/N] ' ANSWER
if [[ "$ANSWER" == y || "$ANSWER" == Y ]]; then sudo /usr/bin/killall coreaudiod; else echo '请稍后重启 Mac。'; fi
read -r -p '按回车关闭此窗口。' ANSWER
