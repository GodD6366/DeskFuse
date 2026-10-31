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

/** 逐步检查内容类型是否能衔接；与 Swift `incompatibilities` 相同。 */
export function incompatibilities(flow: Pick<WorkflowDefinition, 'input_type' | 'steps'>, start: WorkflowContent = flow.input_type): string[] {
  let current = start
  const issues: string[] = []
  flow.steps.forEach((step, index) => {
    let required: WorkflowContent[] = []
    if (['Compress', 'Resize', 'Convert'].includes(step.kind)) required = ['Image', 'Files']
    else if (['TextReplace', 'TextTrim'].includes(step.kind)) required = ['Text']
    else if (['Rename', 'Zip'].includes(step.kind)) required = ['Files']
    if (current !== 'Any' && required.length > 0 && !required.includes(current)) {
      issues.push(`第 ${index + 1} 步「${actionTitle(step.kind)}」不接受${CONTENT_TITLES[current]}`)
    }
    if (step.kind === 'SystemShortcut') current = step.shortcut_output
    if (step.kind === 'Zip' || step.kind === 'Rename') current = 'Files'
    if (step.kind === 'TextReplace' || step.kind === 'TextTrim') current = 'Text'
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
