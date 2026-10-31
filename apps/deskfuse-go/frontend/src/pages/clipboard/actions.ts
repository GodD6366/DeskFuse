// 需要原生宿主执行的剪贴板动作。全部走 backend.invoke，失败时原样抛出，
// 由调用方展示；这里不做任何“假成功”。对应 ClipboardItemActions.swift。

import { backend } from '../../services/backend'
import { acceptsContent } from '../settings/workflow'
import type { ClipboardId, LibraryItem, PeerTarget, WorkflowTarget } from '../../types/clipboard'

type Raw = Record<string, any>

export const copyItem = (id: ClipboardId, plainText = false) =>
  backend.invoke('clipboard.copy', plainText ? { id, plain_text: true } : { id })
export const saveItem = (item: Pick<LibraryItem, 'id' | 'has_image' | 'file_count'>) =>
  backend.invoke('clipboard.save', { id: item.id, has_image: item.has_image, file_count: item.file_count })
export const revealItem = (id: ClipboardId) => backend.invoke('clipboard.reveal', { id })
export const openInPreview = (id: ClipboardId) => backend.invoke('clipboard.openInPreview', { id })
/** 图片交给系统外部编辑器、文本打开原生编辑窗口；两者共用原生 edit 动作。 */
export const editExternally = (item: Pick<LibraryItem, 'id' | 'has_image' | 'file_count'>, title?: string | null) =>
  backend.invoke('clipboard.edit', { id: item.id, has_image: item.has_image, file_count: item.file_count, title: title ?? null })
export const stageItem = (id: ClipboardId) => backend.invoke('clipboard.stage', { id })
export const showShelf = () => backend.invoke('shelf.show')
export const shareToPeer = (id: ClipboardId, peerId: string, workflowId?: string) =>
  backend.invoke('clipboard.share', {
    id, peer_id: peerId, ...(workflowId ? { workflow_id: workflowId } : {}),
  })
export const addPathsToShelf = (paths: string[]) => backend.invoke('shelf.add', { paths })
export const runWorkflow = (workflowId: string, id: ClipboardId) =>
  backend.invoke('workflow.run', { workflow_id: workflowId, history_id: id })

export async function loadPeers(): Promise<PeerTarget[]> {
  const rows = ((await backend.invoke('clipboard.peers')) ?? []) as Raw[]
  return rows.map((r) => ({ id: String(r.id), name: String(r.name ?? ''), connected: Boolean(r.connected) }))
}

export interface WorkflowDefinition extends WorkflowTarget {
  enabled: boolean
  /** 能直接处理的内容类型；`acceptsKnown` 为 false 时未知，按“都可用”处理。 */
  accepts: string[]
  acceptsKnown: boolean
}

const CONTENT_TYPES = ['text', 'image', 'files'] as const

export async function loadWorkflows(): Promise<WorkflowDefinition[]> {
  const result = await backend.invoke('workflow.list')
  const rows = (Array.isArray(result) ? result : result?.definitions ?? []) as Raw[]
  return rows.map((r) => {
    // 原生已按 `accepts(type)` 过滤；定义里带 input_type/steps 时在前端做同样
    // 判断，避免不适用于当前内容类型的工作流出现在菜单里。旧定义没有
    // accepts 字段时退回旧行为。
    const known = typeof r.input_type === 'string' && Array.isArray(r.steps)
    const flow = { input_type: r.input_type, steps: r.steps } as Parameters<typeof acceptsContent>[0]
    return {
      id: String(r.id),
      name: String(r.name ?? ''),
      enabled: r.enabled !== false,
      accepts: known
        ? CONTENT_TYPES.filter((type) => acceptsContent(flow, type))
        : Array.isArray(r.accepts) ? r.accepts.map(String) : [],
      acceptsKnown: known || Array.isArray(r.accepts),
    }
  })
}

export const contentTypeOf = (item: LibraryItem): 'files' | 'image' | 'text' =>
  item.file_count > 0 ? 'files' : item.has_image ? 'image' : 'text'
