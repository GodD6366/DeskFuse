// 剪贴板库的前端投影：只缓存后端返回的列表、设备与计数，不维护业务真相。
// 请求体对应 ClipboardLibraryStore.swift 的 itemRequest / mutate / refreshCounts。

import { reactive } from 'vue'
import { backend } from '../../services/backend'
import type {
  ClipboardContent, ClipboardId, LibraryChoice, LibraryCursor, LibraryItem, LibraryList, LibraryPage,
} from '../../types/clipboard'
import {
  newKey, normalizeContent, normalizePage, normalizeState, serializeList,
} from './format'

export const PAGE_SIZE = 20

export function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message
  if (typeof error === 'string' && error) return error
  const message = (error as { message?: unknown } | null)?.message
  if (typeof message === 'string' && message) return message
  return '未知错误'
}

export const library = reactive({
  lists: [] as LibraryList[],
  devices: [] as LibraryChoice[],
  apps: [] as { bundle_id: string; name: string }[],
  counts: {} as Record<string, number>,
  revision: 0,
  countsRevision: 0,
  loaded: false,
  error: null as string | null,
})

let countsKey = ''

function dayBoundaries() {
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
  const tomorrow = new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1)
  return { today, yesterday, tomorrow }
}

function parseDay(text: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
  return match ? new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3])) : null
}

export function itemRequest(listID: string | null, search = '', source: string | null = null): Record<string, unknown> {
  const { today, yesterday, tomorrow } = dayBoundaries()
  const windows: Record<string, { start: number; end: number }> = {}
  const list = library.lists.find((l) => l.id === listID)
  if (list) {
    list.rules.forEach((rule, index) => {
      if (rule.field !== 'date') return
      let start: Date | null = null
      let end: Date | null = null
      switch (rule.op) {
        case 'today': start = today; end = tomorrow; break
        case 'yesterday': start = yesterday; end = today; break
        case 'last_days': {
          const days = Number.parseInt(rule.value, 10)
          if (days > 0) { start = new Date(today.getFullYear(), today.getMonth(), today.getDate() + 1 - days); end = tomorrow }
          break
        }
        case 'range': {
          start = parseDay(rule.value)
          const last = parseDay(rule.value2)
          if (last) end = new Date(last.getFullYear(), last.getMonth(), last.getDate() + 1)
          break
        }
      }
      if (start && end && start < end) windows[String(index)] = { start: start.getTime(), end: end.getTime() }
    })
  }
  return {
    op: 'items',
    list_id: listID,
    query: search,
    kind: 'all',
    source,
    limit: PAGE_SIZE,
    include_total: false,
    day_start_ms: today.getTime(),
    yesterday_start_ms: yesterday.getTime(),
    day_end_ms: tomorrow.getTime(),
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    rule_windows: windows,
  }
}

function applyState(response: unknown) {
  const state = normalizeState((response ?? {}) as Record<string, unknown>)
  // 比已应用版本更旧的轮询结果不能回滚刚完成的变更。
  if (state.revision < library.revision) return
  if (state.lists) library.lists = state.lists
  if (state.devices) library.devices = state.devices
  if (state.apps) library.apps = state.apps
  if (state.counts_revision !== undefined) library.countsRevision = state.counts_revision
  library.revision = state.revision
  library.error = null
}

/** 读取库状态；返回库修订号是否变化。 */
export async function refreshState(): Promise<boolean> {
  const before = library.revision
  try {
    applyState(await backend.queryLibrary({ op: 'state' }))
    library.loaded = true
  } catch (error) {
    library.error = errorMessage(error)
    return false
  }
  return library.revision !== before
}

export async function refreshCounts(force = false): Promise<void> {
  const day = dayBoundaries().today.getTime()
  const key = `${library.countsRevision}|${day}|${Intl.DateTimeFormat().resolvedOptions().timeZone}`
  if (!force && key === countsKey) return
  const expected = library.countsRevision
  const requests = [itemRequest(null), ...library.lists.map((l) => itemRequest(l.id))]
  try {
    const response = await backend.queryLibrary({ op: 'counts', requests })
    if (library.countsRevision !== expected) return
    const counts: Record<string, number> = {}
    for (const [id, value] of Object.entries((response ?? {}) as Record<string, unknown>)) counts[id] = Number(value)
    library.counts = counts
    countsKey = key
  } catch (error) {
    library.error = errorMessage(error)
  }
}

export async function loadLibrary(): Promise<void> {
  await refreshState()
  await refreshCounts()
}

export async function queryPage(
  listID: string | null, search: string, source: string | null,
  offset = 0, cursor: LibraryCursor | null = null,
): Promise<LibraryPage> {
  const request = itemRequest(listID, search, source)
  request.offset = offset
  if (cursor) request.cursor = { sort_at_ms: cursor.sort_at_ms, id: cursor.id }
  return normalizePage(await backend.queryLibrary(request))
}

export async function queryTotal(listID: string | null, search: string, source: string | null): Promise<number> {
  const request = itemRequest(listID, search, source)
  request.count_only = true
  const response = (await backend.queryLibrary(request)) as { total?: unknown }
  return Number(response?.total ?? 0)
}

/** 变更请求：成功后用返回的库状态覆盖投影，失败则抛出，由调用方展示。 */
export async function mutate(request: Record<string, unknown>): Promise<void> {
  try {
    applyState(await backend.mutateLibrary({ ...request, compact: true }))
  } catch (error) {
    library.error = errorMessage(error)
    throw error
  }
  await refreshCounts()
}

export const saveList = (list: LibraryList) => mutate({ op: 'save_list', list: serializeList(list) })
/** 调用前必须已经过界面确认；宿主要求破坏性操作显式带 confirm。 */
export const deleteList = (id: string) => mutate({ op: 'delete_list', id, confirm: true })
export const reorderLists = (ids: string[]) => mutate({ op: 'reorder', ids })
export const setMembership = (listID: string, historyID: ClipboardId, present: boolean) =>
  mutate({ op: 'membership', list_id: listID, history_id: historyID, present })
export const renameItem = (id: ClipboardId, title: string) => mutate({ op: 'set_item_title', id, title })
export const updateItemText = (id: ClipboardId, text: string, rtf: number[] | null) =>
  mutate(rtf ? { op: 'set_item_text', id, text, rtf } : { op: 'set_item_text', id, text })
export const wantPayload = (id: ClipboardId) => mutate({ op: 'want_item_payload', id })

/** 历史条目和手动列表归档共享一个动作：删除时带上全部关联 ID。调用前必须已经过界面确认。 */
export function deleteItems(items: LibraryItem[]) {
  const ids = Array.from(new Set(items.flatMap((item) => (item.related_ids.length ? item.related_ids : [item.id]))))
  return mutate({ op: 'delete_items', ids, confirm: true })
}

export function newList(): LibraryList {
  return { id: newKey(), name: '', kind: 'manual', match_mode: 'all', position: 0, rules: [] }
}

export async function loadContent(id: ClipboardId): Promise<ClipboardContent> {
  return normalizeContent((await backend.content(id)) ?? {}, id)
}

export function deviceNames(snapshot: Record<string, any> | null = null): Record<string, string> {
  const names: Record<string, string> = {}
  for (const device of library.devices) names[device.id] = displaySourceName(device.name, device.id, snapshot)
  return names
}

/**
 * 来源展示名，对应 ClipboardLibraryStore.swift 的 displaySource(snapshot:)：
 * 本机统一叫「这台 Mac」，已知设备用配对名，未知的十六进制标识截断显示。
 */
export function displaySourceName(
  name: string,
  id: string | null | undefined,
  snapshot: Record<string, any> | null = null,
): string {
  const clean = (name ?? '').trim()
  const candidate = id ?? clean.replace(/^远端 /, '')
  const localId = typeof snapshot?.local?.id === 'string' ? snapshot.local.id : ''
  if ((localId !== '' && candidate === localId) || clean === '本机') return '这台 Mac'
  const devices: any[] = Array.isArray(snapshot?.devices) ? snapshot!.devices : []
  const device = devices.find((entry) =>
    entry && entry.id === candidate && typeof entry.name === 'string' && entry.name !== '')
  if (device) return String(device.name)
  // Unknown identities remain distinguishable without displaying a full key.
  const raw = clean.replace(/^远端 /, '')
  if (raw.length >= 32 && /^[0-9a-f]+$/i.test(raw)) return `远端设备 · ${raw.slice(0, 6)}`
  return clean || '未知来源'
}

/** 来源设备筛选项：库里的设备 + 历史里出现过的旧名称，按显示名去重后排序。 */
export function sourceChoices(legacyNames: string[]): LibraryChoice[] {
  const result = [...library.devices]
  const known = new Set(library.devices.map((d) => d.name))
  for (const name of Array.from(new Set(legacyNames)).sort()) {
    if (!known.has(name)) result.push({ id: name, name: name || '未知来源' })
  }
  const seen = new Set<string>()
  return result
    .map((c) => ({ id: c.id, name: displaySourceName(c.name, c.id) }))
    .filter((c) => {
      const key = c.name.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase()
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    .sort((a, b) => a.name.localeCompare(b.name, 'zh-CN', { numeric: true }))
}
