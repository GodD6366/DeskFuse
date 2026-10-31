import { shortcutParts } from '../settings/util'

export type ModelID = 'g502Lightspeed' | 'mxMaster3' | 'mxMaster4' | 'mxAnywhere3' | 'mxErgo' | 'mxVertical'

export interface MouseModel {
  id: string
  name: string
  summary: string
  connected?: boolean
  battery_percent?: number | null
  battery_charging?: boolean
  wheel_available?: boolean
  wheel_freespin?: boolean | null
}

/** 中间层未返回 models 时的兜底；顺序与 LogitechHIDProtocol.Model 一致。 */
export const MODELS: MouseModel[] = [
  { id: 'g502Lightspeed', name: 'G502 LIGHTSPEED', summary: '11 个实体键 · 保留原生功能' },
  { id: 'mxMaster3', name: 'MX Master 3 / 3S', summary: '手势键、模式切换键与标准按键' },
  { id: 'mxMaster4', name: 'MX Master 4', summary: '手势键、模式切换键与标准按键' },
  { id: 'mxAnywhere3', name: 'MX Anywhere 3 / 3S', summary: '可编程扩展按键' },
  { id: 'mxErgo', name: 'MX Ergo / Ergo S', summary: '可编程扩展按键' },
  { id: 'mxVertical', name: 'MX Vertical', summary: '可编程扩展按键' },
]

export function isG502Button(button: number): boolean {
  return button >= 208 && button < 224
}

export function modelOf(name: string): ModelID | null {
  const value = name.toLowerCase()
  if (value.includes('g502 lightspeed')) return 'g502Lightspeed'
  if (value.includes('mx master 4')) return 'mxMaster4'
  if (value.includes('mx master 3')) return 'mxMaster3'
  if (value.includes('mx anywhere 3') || value.includes('mx anywhere3')) return 'mxAnywhere3'
  if (value.includes('mx ergo')) return 'mxErgo'
  if (value.includes('mx vertical')) return 'mxVertical'
  return null
}

const NAMES: Record<number, string> = {
  0: '左键',
  1: '右键',
  2: '中键 / 滚轮按下',
  3: '后退侧键（按钮 4）',
  4: '前进侧键（按钮 5）',
  195: '拇指手势键（MX Master）',
  196: '滚轮模式切换键（MX Master）',
}
export function buttonName(id: number): string {
  if (isG502Button(id)) return `G502 实体键 ${id - 207}`
  return NAMES[id] ?? `按钮 ${id + 1}`
}

/** 与 MouseSettingsView 的菜单完全一致；id 直接对应要提交的动作。 */
export const CHOICES: Array<[string, string]> = [
  ['default', '保持原始功能'],
  ['disabled', '停用此按键'],
  ['mouse:0', '主键点击'],
  ['mouse:1', '副键点击'],
  ['mouse:2', '中键点击'],
  ['shortcut:33:1048576', '后退'],
  ['shortcut:30:1048576', '前进'],
  ['shortcut:123:262144', '向左切换桌面'],
  ['shortcut:124:262144', '向右切换桌面'],
  ['shortcut:126:262144', 'Mission Control'],
  ['shortcut:125:262144', '应用窗口'],
  ['shortcut:103:0', '显示桌面（F11）'],
  ['shortcut:8:1048576', '复制'],
  ['shortcut:9:1048576', '粘贴'],
  ['shortcut:6:1048576', '撤销'],
  ['shortcut:13:1048576', '关闭窗口'],
]

export function actionFor(id: string): any {
  if (id === 'default') return null
  if (id === 'disabled') return { kind: 'disabled' }
  if (id.startsWith('mouse:')) return { kind: 'mouse', button: Number(id.split(':')[1]) }
  const [, key, mods] = id.split(':')
  return { kind: 'shortcut', key_code: Number(key), modifiers: Number(mods) }
}

export function actionValue(action: any): string {
  if (!action) return 'default'
  if (action.kind === 'mouse') return `mouse:${action.button}`
  if (action.kind === 'shortcut') return `shortcut:${action.key_code}:${action.modifiers ?? 0}`
  return action.kind
}

export function actionLabel(action: any, modifiers = 0): string {
  if (!action) return modifiers ? '沿用默认映射' : '保持原始功能'
  const value = actionValue(action)
  const match = CHOICES.find(([id]) => id === value)
  if (match) return match[1]
  if (action.kind === 'shortcut') {
    return shortcutParts({ key_code: Number(action.key_code), modifiers: Number(action.modifiers ?? 0) }).join('')
  }
  return '原始按钮'
}

export interface PhotoPosition {
  id: string
  title: string
  detail: string
  /** 该位置同时对应的标准鼠标事件；G502 只有 0…4。 */
  genericButton?: number
  /** 原生图片像素坐标（G502）或 0…1 归一化坐标（MX）。 */
  x: number
  y: number
  side: 'top' | 'side'
}

/** 与 G502ButtonPosition 的标题、说明、锚点一致。 */
export const G502_POSITIONS: PhotoPosition[] = [
  { id: 'left', title: '左键', detail: 'G1 · 食指主键', genericButton: 0, x: 615, y: 190, side: 'top' },
  { id: 'right', title: '右键', detail: 'G2 · 右侧主键', genericButton: 1, x: 850, y: 205, side: 'top' },
  { id: 'middle', title: '滚轮按下', detail: 'G3 · 向下按压滚轮', genericButton: 2, x: 735, y: 270, side: 'top' },
  { id: 'tiltLeft', title: '滚轮左拨', detail: '向左轻拨滚轮', x: 735, y: 270, side: 'top' },
  { id: 'tiltRight', title: '滚轮右拨', detail: '向右轻拨滚轮', x: 735, y: 270, side: 'top' },
  { id: 'dpiUp', title: 'DPI +', detail: 'G8 · 左键旁，靠前的小键', x: 515, y: 240, side: 'top' },
  { id: 'dpiDown', title: 'DPI −', detail: 'G7 · 左键旁，靠后的小键', x: 515, y: 325, side: 'top' },
  { id: 'top', title: '顶部 G9', detail: 'G9 · 滚轮模式键后方', x: 740, y: 490, side: 'top' },
  { id: 'forward', title: '前进侧键', detail: 'G5 · 拇指上方，靠前的小键', genericButton: 4, x: 630, y: 515, side: 'side' },
  { id: 'back', title: '后退侧键', detail: 'G4 · 拇指上方，靠后的小键', genericButton: 3, x: 840, y: 480, side: 'side' },
  { id: 'thumb', title: '拇指键', detail: 'G6 · 拇指前方的 DPI Shift 键', x: 540, y: 620, side: 'side' },
]

export const G502_FACTORY: Record<string, number> = {
  left: 208, right: 209, middle: 210, back: 211, forward: 212,
  thumb: 213, dpiDown: 214, dpiUp: 215, top: 216, tiltLeft: 217, tiltRight: 218,
}

export const G502_WHEEL_POSITIONS = ['middle', 'tiltLeft', 'tiltRight'] as const

export interface MXPosition {
  button: number
  title: string
  x: number
  y: number
  side: 'top' | 'side'
}

/** 与 MXMasterMouseEditor 的热点与顺序一致。 */
export const MX_POSITIONS: MXPosition[] = [
  { button: 0, title: '左键', x: 0.46, y: 0.16, side: 'top' },
  { button: 1, title: '右键', x: 0.65, y: 0.18, side: 'top' },
  { button: 2, title: '中键 / 滚轮按下', x: 0.56, y: 0.23, side: 'top' },
  { button: 196, title: '滚轮模式切换键（MX Master）', x: 0.56, y: 0.4, side: 'top' },
  { button: 4, title: '前进侧键（按钮 5）', x: 0.47, y: 0.485, side: 'side' },
  { button: 3, title: '后退侧键（按钮 4）', x: 0.58, y: 0.445, side: 'side' },
  { button: 195, title: '拇指手势键（MX Master）', x: 0.61, y: 0.685, side: 'side' },
]

/** 原生图片尺寸：G502 1384×1038；MX Master 3 2400×2061。 */
export const G502_PHOTO = { width: 1384, height: 1038 }
export const MX_PHOTO = { width: 2400, height: 2061 }

export function photoName(model: string, side: 'top' | 'side'): string {
  return model === 'g502Lightspeed' ? `g502-lightspeed-${side}` : `mx-master-3-${side}`
}

export function photoURL(name: string): string {
  return `deskfuse://blob/mouse/${name}`
}

export function optionFor(option: any, fallbackName: string): MouseModel | null {
  if (!option?.model) return null
  return MODELS.find((model) => model.id === option.model) ?? { id: option.model, name: fallbackName, summary: '' }
}
