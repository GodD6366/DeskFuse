#!/bin/bash
# Installs/removes only the separately identified DeskFuse HAL plugin.
set -euo pipefail
MICROPHONE_RESOURCES="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$MICROPHONE_RESOURCES/compatibility.sh" "$MICROPHONE_RESOURCES/previous-installation.json"
DRIVER_ID="$MICROPHONE_BUNDLE_ID"
HAL=/Library/Audio/Plug-Ins/HAL
DEST="$HAL/${MICROPHONE_BUNDLE_NAME}.driver"
[[ "$EUID" == 0 ]] || { echo '请通过随包安装或卸载入口运行。' >&2; exit 1; }
verify_identity() {
  [[ -d "$1" && ! -L "$1" ]] || { echo '驱动目录无效' >&2; exit 1; }
  [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$1/Contents/Info.plist")" == "$DRIVER_ID" ]] || { echo '驱动身份不符，未作修改。' >&2; exit 1; }
}
RESTART="${3:-no-restart}"
[[ "$RESTART" == no-restart || "$RESTART" == restart-audio ]] || { echo '无效的音频服务选项' >&2; exit 2; }
case "${1:-}" in
  install)
    SOURCE="${2:?missing driver source}"
    verify_identity "$SOURCE"
    /usr/bin/codesign --verify --deep --strict "$SOURCE"
    /bin/mkdir -p "$HAL"
    [[ ! -L "$HAL" && ! -L "$DEST" ]] || { echo '安装路径为符号链接，已停止。' >&2; exit 1; }
    if [[ -e "$DEST" ]]; then verify_identity "$DEST"; fi
    STAGE="$(/usr/bin/mktemp -d "$HAL/.${MICROPHONE_BUNDLE_NAME}.XXXXXX")"
    trap '/bin/rm -rf "$STAGE"' EXIT
    /usr/bin/ditto "$SOURCE" "$STAGE/${MICROPHONE_BUNDLE_NAME}.driver"
    /usr/sbin/chown -R root:wheel "$STAGE/${MICROPHONE_BUNDLE_NAME}.driver"
    /bin/chmod -R go-w "$STAGE/${MICROPHONE_BUNDLE_NAME}.driver"
    /usr/bin/codesign --verify --deep --strict "$STAGE/${MICROPHONE_BUNDLE_NAME}.driver"
    if [[ -e "$DEST" ]]; then /bin/mv "$DEST" "$STAGE/previous.driver"; fi
    if ! /bin/mv "$STAGE/${MICROPHONE_BUNDLE_NAME}.driver" "$DEST"; then
      [[ ! -e "$STAGE/previous.driver" ]] || /bin/mv "$STAGE/previous.driver" "$DEST"
      exit 1
    fi
    echo 'DeskFuse 麦克风驱动安装完成。'
    ;;
  uninstall)
    if [[ -e "$DEST" || -L "$DEST" ]]; then verify_identity "$DEST"; /bin/rm -rf "$DEST"; fi
    echo 'DeskFuse 麦克风驱动已卸载。未修改其它音频驱动。'
    ;;
  *) echo 'usage: manage-driver.sh install SOURCE | uninstall' >&2; exit 2 ;;
esac
if [[ "$RESTART" == restart-audio ]]; then
  /usr/bin/killall coreaudiod || { echo '驱动文件操作已完成，但重启音频服务失败，请重启 Mac。' >&2; exit 1; }
  echo '已请求重启音频服务。'
else
  echo '请重启 Mac，或按安装入口提示主动重启音频服务后生效。'
fi
