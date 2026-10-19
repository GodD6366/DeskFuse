#!/bin/bash
# 只安装/卸载身份可校验的 DeskFuse HAL 插件；旧安装遗留的驱动在身份校验通过后一并移除。
set -euo pipefail
MICROPHONE_RESOURCES="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$MICROPHONE_RESOURCES/compatibility.sh" "$MICROPHONE_RESOURCES/previous-installation.json"
DRIVER_ID="$MICROPHONE_BUNDLE_ID"
HAL=/Library/Audio/Plug-Ins/HAL
DEST="$HAL/${MICROPHONE_BUNDLE_NAME}.driver"
LEGACY_DEST="$HAL/${LEGACY_MICROPHONE_BUNDLE_NAME}.driver"
[[ "$EUID" == 0 ]] || { echo '请通过随包安装或卸载入口运行。' >&2; exit 1; }
verify_identity() {
  [[ -d "$1" && ! -L "$1" ]] || { echo '驱动目录无效' >&2; exit 1; }
  [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$1/Contents/Info.plist")" == "$DRIVER_ID" ]] || { echo '驱动身份不符，未作修改。' >&2; exit 1; }
}
# 旧版驱动与当前驱动共用插件 UUID，同时存在时 Core Audio 只会加载其中一个，必须清理。
# 只删除 Bundle ID 与旧安装记录一致的真实目录；不符时保留并提示，不影响本次操作。
remove_legacy() {
  [[ -e "$LEGACY_DEST" || -L "$LEGACY_DEST" ]] || return 0
  if [[ -d "$LEGACY_DEST" && ! -L "$LEGACY_DEST" ]] \
    && [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$LEGACY_DEST/Contents/Info.plist" 2>/dev/null)" == "$LEGACY_MICROPHONE_BUNDLE_ID" ]]; then
    /bin/rm -rf "$LEGACY_DEST"
    echo '已移除旧版麦克风驱动。'
  else
    echo "旧版驱动位置的身份不符，已保留：$LEGACY_DEST" >&2
  fi
}
RESTART="${3:-no-restart}"
[[ "$RESTART" == no-restart || "$RESTART" == restart-audio ]] || { echo '无效的音频服务选项' >&2; exit 2; }
case "${1:-}" in
  restart-audio)
    /usr/bin/killall coreaudiod || { echo '无法重启音频服务，请重试或重新启动 Mac。' >&2; exit 1; }
    echo '已请求重启音频服务。未修改任何音频驱动文件。'
    exit 0
    ;;
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
    remove_legacy
    echo 'DeskFuse 麦克风驱动安装完成。'
    ;;
  uninstall)
    if [[ -e "$DEST" || -L "$DEST" ]]; then verify_identity "$DEST"; /bin/rm -rf "$DEST"; fi
    remove_legacy
    echo 'DeskFuse 麦克风驱动已卸载。未修改其它音频驱动。'
    ;;
  *) echo 'usage: manage-driver.sh install SOURCE | uninstall | restart-audio' >&2; exit 2 ;;
esac
if [[ "$RESTART" == restart-audio ]]; then
  /usr/bin/killall coreaudiod || { echo '驱动文件操作已完成，但重启音频服务失败，请重启 Mac。' >&2; exit 1; }
  echo '已请求重启音频服务。'
else
  echo '请重启 Mac，或按安装入口提示主动重启音频服务后生效。'
fi
