import { ref } from 'vue'
import { backend } from '../../services/backend'

export type Snapshot = Record<string, any> | null

export interface Key {
  key_code: number
  modifiers: number
}

/** 读取字段，同时接受 snake_case 与 camelCase 两种桥接命名。 */
export function get<T = any>(obj: any, name: string): T | undefined {
  if (obj === null || obj === undefined) return undefined
  if (obj[name] !== undefined && obj[name] !== null) return obj[name]
  const camel = name.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())
  return obj[camel] ?? undefined
}

export function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error)
  } catch {
    return String(error)
  }
}

/** 与 Swift `friendlyError` 一致：内部英文错误换成可理解的说明。 */
export function friendlyError(message: string): string {
  const lower = message.toLowerCase()
  const rules: [string[], string][] = [
    [['timed out', 'timeout', '超时'], '连接超时，请确认两台电脑在同一网络后重试。'],
    [['connection refused', 'unreachable', 'no route', 'network is down'], '无法连接到对方电脑，请确认对方已打开 DeskFuse。'],
    [['permission', 'not permitted', 'accessibility', 'denied'], '缺少系统权限，请在「设置 → 权限」中授权后重试。'],
    [['disk full', 'no space'], '磁盘空间不足，请清理后重试。'],
    [['stream stalled', 'transport operation failed'], '网络传输中断，DeskFuse 会自动重试。'],
    [['store operation failed', 'database', 'sqlite'], '本地数据读写失败，请重新打开 DeskFuse；反复出现时，请通过「帮助 → 反馈问题」告诉我们。'],
    [['command channel is closed', 'runtime'], '后台服务未就绪，请稍后重试或重新打开 DeskFuse。'],
  ]
  const hit = rules.find(([keys]) => keys.some((key) => lower.includes(key)))
  if (hit) return hit[1]
  return /[一-鿿]/.test(message) ? message : '操作未完成，请稍后重试。反复出现时，请通过「帮助 → 反馈问题」告诉我们。'
}

/** 动作的真实执行状态：成功才返回 ok，失败保留错误文本，不伪造成功。 */
export function useAction() {
  const pending = ref(false)
  const error = ref<string | null>(null)
  const status = ref<string | null>(null)

  async function run<T = any>(
    action: string,
    params?: Record<string, unknown>,
    success?: string,
  ): Promise<{ ok: boolean; value?: T }> {
    if (pending.value) return { ok: false }
    pending.value = true
    error.value = null
    status.value = null
    try {
      const value = (await backend.invoke(action, params)) as T
      status.value = success ?? null
      return { ok: true, value }
    } catch (e) {
      error.value = errorText(e)
      return { ok: false }
    } finally {
      pending.value = false
    }
  }

  function reset() {
    error.value = null
    status.value = null
  }

  return { pending, error, status, run, reset }
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  const units = ['字节', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000
    unit += 1
  }
  // 与 ByteCountFormatter(.file) 对齐：大于等于 100 取整，10–99 一位小数，10 以下两位。
  const text = unit === 0 || value >= 100
    ? Math.round(value).toString()
    : value >= 10
      ? value.toFixed(1).replace(/\.0$/, '')
      : value.toFixed(2).replace(/0$/, '').replace(/\.0$/, '')
  return `${text} ${units[unit]}`
}

// 快捷键以 macOS 键码与 NSEvent 修饰键位保存。
export const MOD_SHIFT = 1 << 17
export const MOD_CONTROL = 1 << 18
export const MOD_OPTION = 1 << 19
export const MOD_COMMAND = 1 << 20
export const MOD_PRIMARY = MOD_CONTROL | MOD_OPTION | MOD_COMMAND
export const MOD_ANY = MOD_PRIMARY | MOD_SHIFT

const KEY_NAMES: Record<number, string> = {
  123: '←', 124: '→', 125: '↓', 126: '↑', 53: 'Esc', 49: 'Space', 36: '↩', 48: 'Tab', 51: '⌫', 117: '⌦',
  0: 'A', 1: 'S', 2: 'D', 3: 'F', 4: 'H', 5: 'G', 6: 'Z', 7: 'X', 8: 'C', 9: 'V', 11: 'B', 12: 'Q', 13: 'W',
  14: 'E', 15: 'R', 16: 'Y', 17: 'T', 31: 'O', 32: 'U', 34: 'I', 35: 'P', 37: 'L', 38: 'J', 40: 'K', 45: 'N', 46: 'M',
  18: '1', 19: '2', 20: '3', 21: '4', 23: '5', 22: '6', 26: '7', 28: '8', 25: '9', 29: '0',
  24: '=', 27: '−', 30: ']', 33: '[', 39: "'", 41: ';', 42: '\\', 43: ',', 44: '/', 47: '.', 50: '`',
  122: 'F1', 120: 'F2', 99: 'F3', 118: 'F4', 96: 'F5', 97: 'F6', 98: 'F7', 100: 'F8', 101: 'F9', 109: 'F10', 103: 'F11', 111: 'F12',
  105: 'F13', 107: 'F14', 113: 'F15', 106: 'F16', 64: 'F17', 79: 'F18', 80: 'F19', 90: 'F20',
  115: 'Home', 119: 'End', 116: 'PgUp', 121: 'PgDn', 114: 'Ins',
  65: 'Num.', 67: 'Num*', 69: 'Num+', 71: 'NumC', 75: 'Num/', 76: 'Num↩', 78: 'Num−', 81: 'Num=',
  82: 'Num0', 83: 'Num1', 84: 'Num2', 85: 'Num3', 86: 'Num4', 87: 'Num5', 88: 'Num6', 89: 'Num7', 91: 'Num8', 92: 'Num9',
  10: 'ISO \\', 93: '¥', 94: 'JIS _', 110: 'Menu',
}

/** KeyboardEvent.code → macOS 键码。 */
export const CODE_TO_KEY: Record<string, number> = {
  KeyA: 0, KeyS: 1, KeyD: 2, KeyF: 3, KeyH: 4, KeyG: 5, KeyZ: 6, KeyX: 7, KeyC: 8, KeyV: 9, KeyB: 11, KeyQ: 12,
  KeyW: 13, KeyE: 14, KeyR: 15, KeyY: 16, KeyT: 17, KeyO: 31, KeyU: 32, KeyI: 34, KeyP: 35, KeyL: 37, KeyJ: 38,
  KeyK: 40, KeyN: 45, KeyM: 46,
  Digit1: 18, Digit2: 19, Digit3: 20, Digit4: 21, Digit5: 23, Digit6: 22, Digit7: 26, Digit8: 28, Digit9: 25, Digit0: 29,
  Equal: 24, Minus: 27, BracketRight: 30, BracketLeft: 33, Quote: 39, Semicolon: 41, Backslash: 42, Comma: 43,
  Slash: 44, Period: 47, Backquote: 50,
  ArrowLeft: 123, ArrowRight: 124, ArrowDown: 125, ArrowUp: 126, Escape: 53, Space: 49, Enter: 36, Tab: 48,
  Backspace: 51, Delete: 117,
  F1: 122, F2: 120, F3: 99, F4: 118, F5: 96, F6: 97, F7: 98, F8: 100, F9: 101, F10: 109, F11: 103, F12: 111,
  F13: 105, F14: 107, F15: 113, F16: 106, F17: 64, F18: 79, F19: 80, F20: 90,
  Home: 115, End: 119, PageUp: 116, PageDown: 121, Insert: 114,
  NumpadDecimal: 65, NumpadMultiply: 67, NumpadAdd: 69, NumpadClear: 71, NumpadDivide: 75, NumpadEnter: 76,
  NumpadSubtract: 78, NumpadEqual: 81,
  Numpad0: 82, Numpad1: 83, Numpad2: 84, Numpad3: 85, Numpad4: 86,
  Numpad5: 87, Numpad6: 88, Numpad7: 89, Numpad8: 91, Numpad9: 92,
  IntlBackslash: 10, IntlYen: 93, IntlRo: 94, ContextMenu: 110,
}

export function keyOf(value: any): Key | null {
  if (!value) return null
  const key_code = get<number>(value, 'key_code')
  if (key_code === undefined) return null
  return { key_code, modifiers: get<number>(value, 'modifiers') ?? 0 }
}

export function shortcutParts(key: Key): string[] {
  const parts: string[] = []
  for (const [flag, symbol] of [[MOD_CONTROL, '⌃'], [MOD_OPTION, '⌥'], [MOD_SHIFT, '⇧'], [MOD_COMMAND, '⌘']] as const) {
    if (key.modifiers & flag) parts.push(symbol)
  }
  parts.push(KEY_NAMES[key.key_code] ?? `Key ${key.key_code}`)
  return parts
}

export function sameKey(a: Key | null, b: Key | null): boolean {
  return !!a && !!b && a.key_code === b.key_code && a.modifiers === b.modifiers
}

/** 保留快捷键：⌘V、紧急收回、文件暂存区。 */
export function reservedConflict(key: Key): string | null {
  if (key.key_code === 9 && key.modifiers === MOD_COMMAND) return '⌘ V 保留给普通粘贴'
  if (key.key_code === 53 && (key.modifiers & MOD_PRIMARY) === MOD_PRIMARY) return '这组按键用于紧急收回'
  if (key.key_code === 2 && key.modifiers === (MOD_SHIFT | MOD_COMMAND)) return '这组按键用于文件暂存区'
  return null
}

/** 本机偏好（Dock、保持唤醒、工作流开关、更新通道等）：原生快照放在 `app`，工作流开关另见 `workflows.enabled`。 */
export function preference(snapshot: Snapshot, name: string): any {
  const value = get(get(snapshot, 'app'), name) ?? get(get(snapshot, 'preferences'), name) ?? get(snapshot, name)
  if (value === undefined && name === 'workflows_enabled') return get(get(snapshot, 'workflows'), 'enabled')
  return value
}

/** 键鼠共享的 6 个快捷键（方向、锁定、紧急收回）。 */
export function inputShortcuts(snapshot: Snapshot): Key[] {
  const list = get<any[]>(get(get(snapshot, 'input'), 'settings'), 'shortcuts') ?? []
  return list.map((item) => keyOf(item)).filter((item): item is Key => item !== null)
}
