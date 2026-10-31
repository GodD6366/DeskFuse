// 工作流运行状态的展示投影。快照里 `workflow_runs` / `workflow_error` 位于顶层，
// 字段与 MyGoBridgeServer.swift 的 snapshot 输出一致；运行名称来自 workflow.list。

import { reactive } from 'vue'
import { loadWorkflows } from './actions'

export interface WorkflowRun {
  id: string
  workflowId: string
  /** 快照当前没有下发 step_index；存在时按原生格式显示「第 n 步」。 */
  stepIndex: number | null
  phase: string
  message: string
  outputKind: string
  outputMime: string
  outputPaths: string[]
  outputBytes: number
  inputBytes: number
  width: number
  height: number
}

const str = (value: unknown): string => (typeof value === 'string' ? value : '')
const idOf = (value: unknown): string => (value === null || value === undefined ? '' : String(value))
const num = (value: unknown): number => {
  const parsed = typeof value === 'string' ? Number(value) : value
  return typeof parsed === 'number' && Number.isFinite(parsed) ? parsed : 0
}

export function normalizeWorkflowRun(raw: Record<string, any>): WorkflowRun {
  return {
    id: idOf(raw.id),
    workflowId: idOf(raw.workflow_id),
    stepIndex: raw.step_index === null || raw.step_index === undefined ? null : num(raw.step_index),
    phase: str(raw.phase),
    message: str(raw.message),
    outputKind: str(raw.output_kind).toLowerCase(),
    outputMime: str(raw.output_mime),
    outputPaths: Array.isArray(raw.output_paths) ? raw.output_paths.map(String) : [],
    outputBytes: num(raw.output_bytes),
    inputBytes: num(raw.input_bytes),
    width: num(raw.width),
    height: num(raw.height),
  }
}

export function snapshotWorkflowRuns(snapshot: Record<string, any> | null | undefined): WorkflowRun[] {
  const rows = snapshot?.workflow_runs
  return Array.isArray(rows)
    ? rows.map((row) => normalizeWorkflowRun((row ?? {}) as Record<string, any>))
    : []
}

export function snapshotWorkflowError(snapshot: Record<string, any> | null | undefined): string {
  const value = snapshot?.workflow_error
  return typeof value === 'string' ? value : ''
}

// 定义列表只取一次；与原生 WorkflowStore.definitions 一样用于把 workflow_id 映射成名称。
const names = reactive<Record<string, string>>({})
let namesLoaded = false
let namesPending: Promise<void> | null = null

export function ensureWorkflowNames(): void {
  if (namesLoaded || namesPending) return
  namesPending = loadWorkflows()
    .then((flows) => {
      for (const flow of flows) {
        if (flow.id) names[flow.id] = flow.name || '工作流'
      }
      namesLoaded = true
    })
    .catch(() => { /* 名称取不到时退回「工作流」，下个页面挂载再试一次。 */ })
    .finally(() => { namesPending = null })
}

export function workflowName(id: string): string {
  return names[id] || '工作流'
}

/** 原生 WorkflowRunStatusView.formatName：图片输出只显示格式缩写。 */
export function outputFormatName(mime: string): string {
  switch (mime) {
    case 'image/jpeg': return 'JPEG'
    case 'image/png': return 'PNG'
    case 'image/webp': return 'WebP'
    case 'image/heic': return 'HEIC'
    default: return mime
  }
}
