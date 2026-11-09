import { AlignLeft, ClipboardPaste, Copy, FileArchive, Images, Layers, Maximize2, Minimize2, PencilLine, TextSearch } from 'lucide-vue-next'
import type { Component } from 'vue'
import type { Key } from './util'

export type WorkflowContent = 'Any' | 'Text' | 'Image' | 'Files'
export type WorkflowAction =
  | 'Compress' | 'Resize' | 'Convert' | 'TextReplace' | 'TextTrim' | 'Rename' | 'Zip'
  | 'SystemShortcut' | 'Copy' | 'Paste'

export interface WorkflowStep {
  kind: WorkflowAction
  quality: number
  target_bytes: number | null
  max_edge: number | null
  scale_percent: number | null
  format: string
  shortcut_id: string
  shortcut_name: string
  local_shortcut_id?: string | null
  output_shape: string
  find: string
  replacement: string
  prefix: string
  suffix: string
  shortcut_output: WorkflowContent
}

export interface WorkflowDefinition {
  version: number
  id: string
  name: string
  enabled: boolean
  shortcut: Key | null
  input_type: WorkflowContent
  steps: WorkflowStep[]
}

export const CONTENT_TITLES: Record<WorkflowContent, string> = {
  Any: '任意内容', Text: '文字', Image: '图片', Files: '文件与文件夹',
}

export const ACTIONS: { kind: WorkflowAction; title: string }[] = [
  { kind: 'Compress', title: '压缩图片' },
  { kind: 'Resize', title: '调整尺寸' },
  { kind: 'Convert', title: '转换格式' },
  { kind: 'TextReplace', title: '查找替换文字' },
  { kind: 'TextTrim', title: '去除首尾空白' },
  { kind: 'Rename', title: '批量重命名' },
  { kind: 'Zip', title: '打包 ZIP' },
  { kind: 'SystemShortcut', title: '运行系统快捷指令' },
  { kind: 'Copy', title: '复制到剪贴板' },
  { kind: 'Paste', title: '粘贴到当前应用' },
]

export const actionTitle = (kind: WorkflowAction) => ACTIONS.find((a) => a.kind === kind)?.title ?? kind
/** 与原生 WorkflowAction.icon 对应的线性图标。 */
export const ACTION_ICONS: Record<WorkflowAction, Component> = {
  Compress: Minimize2,
  Resize: Maximize2,
  Convert: Images,
  TextReplace: TextSearch,
  TextTrim: AlignLeft,
  Rename: PencilLine,
  Zip: FileArchive,
  SystemShortcut: Layers,
  Copy: Copy,
  Paste: ClipboardPaste,
}
export const isTerminal = (kind: WorkflowAction) => kind === 'Copy' || kind === 'Paste'

export function newStep(kind: WorkflowAction): WorkflowStep {
  return {
    kind, quality: 85, target_bytes: null, max_edge: kind === 'Resize' ? 2048 : null, scale_percent: null,
    format: 'Jpeg', shortcut_id: '', shortcut_name: '', output_shape: 'FollowSource',
    find: '', replacement: '', prefix: '', suffix: '', shortcut_output: 'Any',
  }
}

export function newWorkflow(): WorkflowDefinition {
  return {
    version: 2, id: crypto.randomUUID(), name: '新工作流', enabled: true, shortcut: null,
    input_type: 'Any', steps: [newStep('Copy')],
  }
}

export function chatImageExample(): WorkflowDefinition {
  const flow = newWorkflow()
  const compress = newStep('Compress')
  compress.target_bytes = 512_000
  flow.name = '聊天图片'
  flow.input_type = 'Image'
  flow.steps = [compress, newStep('Convert'), newStep('Paste')]
  return flow
}

export function websiteImageExample(): WorkflowDefinition {
  const flow = newWorkflow()
  const resize = newStep('Resize')
  resize.max_edge = 1600
  const convert = newStep('Convert')
  convert.format = 'Webp'
  flow.name = '网站配图'
  flow.input_type = 'Image'
  flow.steps = [resize, convert, newStep('Copy')]
  return flow
}

const CONCRETE: WorkflowContent[] = ['Text', 'Image', 'Files']
const SHORT_TITLES: Record<WorkflowContent, string> = { Any: '任意内容', Text: '文字', Image: '图片', Files: '文件' }
/** 流水线连线上显示的内容类型名。 */
export const contentLabel = (type: WorkflowContent) => SHORT_TITLES[type]

/** 动作能处理的具体内容类型；不限制输入的动作返回全部。 */
export function actionAccepts(kind: WorkflowAction): WorkflowContent[] {
  if (kind === 'Compress' || kind === 'Resize' || kind === 'Convert') return ['Image', 'Files']
  if (kind === 'TextReplace' || kind === 'TextTrim') return ['Text']
  if (kind === 'Rename' || kind === 'Zip') return ['Files']
  return CONCRETE
}

/** 动作能否接收 `content`；'Any' 视为全部兼容。 */
export const stepAccepts = (kind: WorkflowAction, content: WorkflowContent) =>
  content === 'Any' || actionAccepts(kind).includes(content)

/** 动作的输入要求，例如「图片或文件」；不限制输入时为空。 */
export function acceptRequirement(kind: WorkflowAction): string {
  const accepted = actionAccepts(kind)
  return accepted.length >= CONCRETE.length ? '' : accepted.map((t) => SHORT_TITLES[t]).join('或')
}

/** 步骤执行后输出的内容类型。 */
export function contentAfter(step: Pick<WorkflowStep, 'kind' | 'shortcut_output'>, current: WorkflowContent): WorkflowContent {
  if (step.kind === 'SystemShortcut') return step.shortcut_output
  if (step.kind === 'Zip' || step.kind === 'Rename') return 'Files'
  if (step.kind === 'TextReplace' || step.kind === 'TextTrim') return 'Text'
  return current
}

/** 每个步骤执行之前流入的内容类型；与 Swift `incompatibilities` 使用同一套规则。 */
export function contentFlow(flow: Pick<WorkflowDefinition, 'input_type' | 'steps'>, start: WorkflowContent = flow.input_type): WorkflowContent[] {
  let current = start
  return flow.steps.map((step) => {
    const before = current
    current = contentAfter(step, current)
    return before
  })
}

/** 逐步检查内容类型是否能衔接；与 Swift `incompatibilities` 相同。 */
export function incompatibilities(flow: Pick<WorkflowDefinition, 'input_type' | 'steps'>, start: WorkflowContent = flow.input_type): string[] {
  const incoming = contentFlow(flow, start)
  const issues: string[] = []
  flow.steps.forEach((step, index) => {
    if (!stepAccepts(step.kind, incoming[index])) {
      issues.push(`第 ${index + 1} 步「${actionTitle(step.kind)}」不接受${CONTENT_TITLES[incoming[index]]}`)
    }
  })
  return issues
}

/** 与 Swift `WorkflowDefinition.accepts(_:)` 相同：剪贴板内容类型能否走这条工作流。 */
export function acceptsContent(flow: Pick<WorkflowDefinition, 'input_type' | 'steps'>, type: 'text' | 'image' | 'files'): boolean {
  const target: WorkflowContent = type === 'text' ? 'Text' : type === 'image' ? 'Image' : 'Files'
  const input = flow.input_type
  if (input !== 'Any' && input !== target && !(input === 'Image' && target === 'Files')) return false
  return incompatibilities(flow, target).length === 0
}

export function summary(flow: WorkflowDefinition): string {
  return flow.enabled ? `${flow.steps.length} 个步骤 · ${CONTENT_TITLES[flow.input_type]}` : '已停用'
}

/** 步骤选择器里的分组。 */
export const ACTION_GROUPS: { title: string; kinds: WorkflowAction[] }[] = [
  { title: '图片', kinds: ['Compress', 'Resize', 'Convert'] },
  { title: '文字', kinds: ['TextReplace', 'TextTrim'] },
  { title: '文件', kinds: ['Rename', 'Zip'] },
  { title: '系统', kinds: ['SystemShortcut'] },
]

const FORMAT_TITLES: Record<string, string> = { Png: 'PNG', Jpeg: 'JPEG', Webp: 'WebP', Heic: 'HEIC' }
const clip = (text: string, max = 14) => (text.length > max ? `${text.slice(0, max)}…` : text)

/** 步骤卡片头部的参数摘要；没有可显示的参数时为 null。 */
export function stepSummary(step: WorkflowStep): string | null {
  switch (step.kind) {
    case 'Compress':
      return step.target_bytes !== null ? `目标 ${Math.round(step.target_bytes / 1024)} KiB` : `质量 ${step.quality}%`
    case 'Resize':
      return step.max_edge !== null ? `最长边 ${step.max_edge} px` : `缩放 ${step.scale_percent ?? 75}%`
    case 'Convert':
      return `转为 ${FORMAT_TITLES[step.format] ?? step.format}`
    case 'TextReplace':
      return step.find ? `“${clip(step.find)}” → “${clip(step.replacement)}”` : null
    case 'Rename': {
      const parts: string[] = []
      if (step.find) parts.push(`“${clip(step.find)}” → “${clip(step.replacement)}”`)
      if (step.prefix) parts.push(`前缀“${clip(step.prefix)}”`)
      if (step.suffix) parts.push(`后缀“${clip(step.suffix)}”`)
      return parts.length ? parts.join(' · ') : null
    }
    case 'SystemShortcut':
      return step.shortcut_name ? `快捷指令：${step.shortcut_name}` : null
    default:
      return null
  }
}
