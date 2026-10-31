<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { ArrowUpRight, LayoutGrid, List, MoreHorizontal, Search } from 'lucide-vue-next'
import type { ItemMeta, LibraryCursor, LibraryItem, LibraryList } from '../types/clipboard'
import {
  deleteItems, deviceNames, errorMessage, library, newList, queryPage, refreshCounts, refreshState,
  renameItem, wantPayload,
} from './clipboard/library'
import HistoryRow from './clipboard/HistoryRow.vue'
import PanelCard from './clipboard/PanelCard.vue'
import ContextMenu from './clipboard/ContextMenu.vue'
import Icon from './clipboard/Icon.vue'
import WorkflowRunStatus from './clipboard/WorkflowRunStatus.vue'
import ListEditorDialog from './clipboard/ListEditorDialog.vue'
import TitleEditorDialog from './clipboard/TitleEditorDialog.vue'
import type { MenuItem } from './clipboard/menu'
import { itemMenu } from './clipboard/menus'
import { contentTypeOf, loadPeers, loadWorkflows, runWorkflow, shareToPeer } from './clipboard/actions'
import { backend } from '../services/backend'
import { estimatedHeight, masonryColumns, shouldPreview } from './clipboard/format'
import { snapshotWorkflowError, snapshotWorkflowRuns } from './clipboard/workflowRuns'

// 菜单栏剪贴板面板：与原生 MenuBarClipboardView 同构。单击选中、双击粘贴，
// 键盘、右键菜单、列表标签、来源筛选与快速预览都走原生同一批动作。
// 快速预览由 Go 进程放进贴住选中行的浮窗（Preview 服务），页面只提供行位置。
const search = ref('')
const items = ref<LibraryItem[]>([])
const selectedID = ref<string | null>(null)
const listID = ref<string | null>(null)
const source = ref<string | null>(null)
const sources = ref<{ id: string; name: string }[]>([])
const displayMode = ref<'list' | 'grid'>((localStorage.getItem('deskfuse.clipboard.displayMode') as 'list' | 'grid') || 'list')
const loading = ref(false)
const loadingMore = ref(false)
const searching = ref(false)
const error = ref('')
const copying = ref(false)
const commandHeld = ref(false)
const menu = ref<{ x: number; y: number; items: MenuItem[] } | null>(null)
const editingList = ref<LibraryList | null>(null)
const titleItem = ref<LibraryItem | null>(null)
const previewID = ref<string | null>(null)
const snapshot = ref<Record<string, any> | null>(null)
const searchBox = ref<HTMLInputElement | null>(null)
const rowsBox = ref<HTMLElement | null>(null)
const showCopyProgress = ref(false)
let generation = 0, disposed = false, cursor: LibraryCursor | null = null
let nextOffset: number | null = 0
let searchTimer: ReturnType<typeof setTimeout> | undefined
let libraryTimer: ReturnType<typeof setInterval> | undefined
let snapshotTimer: ReturnType<typeof setInterval> | undefined
let progressTimer: ReturnType<typeof setTimeout> | undefined

async function refreshSnapshot() {
  try { snapshot.value = await backend.snapshot() } catch { /* 面板保持上一次已知状态 */ }
}

const visibleItems = computed(() => items.value.filter((item) => !(item.file_count > 0 && item.remote && !item.available)))
const selected = computed(() => visibleItems.value.find((item) => item.id === selectedID.value) ?? null)
const deviceNameMap = computed(() => deviceNames(snapshot.value))
const lists = computed<LibraryList[]>(() => library.lists)
const gridColumns = computed(() => masonryColumns(visibleItems.value, 2))
const sourceName = computed(() => sources.value.find((choice) => choice.id === source.value)?.name ?? '全部设备')
const hasFilters = computed(() => search.value.trim() !== '' || source.value !== null || listID.value !== null)
const workflowRuns = computed(() => snapshotWorkflowRuns(snapshot.value))
const workflowError = computed(() => snapshotWorkflowError(snapshot.value))
const snapshotReady = computed(() => snapshot.value !== null)
const starting = computed(() => snapshot.value?.app?.starting === true || snapshot.value === null)
const engineReady = computed(() => snapshot.value?.engine_ready !== false)
const historyEmpty = computed(() => {
  const history = snapshot.value?.clipboard?.history
  return Array.isArray(history) ? history.length === 0 : false
})
const clipboardError = computed(() => {
  const value = snapshot.value?.clipboard?.error
  return typeof value === 'string' ? value : ''
})
const message = computed(() => error.value || library.error || clipboardError.value)
const appError = computed(() => {
  const value = snapshot.value?.app?.error
  return typeof value === 'string' ? value : ''
})

/** 行内序号用于 ⌘1-9 与面板卡片右上角的快捷键提示。 */
function indexOf(item: LibraryItem): number {
  return visibleItems.value.findIndex((row) => row.id === item.id)
}

/** 与主页面相同的自动 OCR 规则：关闭时不提交，同一 id 只提交一次。 */
const OCR_VERSION = 1
const scheduledOCR = new Set<string>()
function scheduleOCR(rows: LibraryItem[]) {
  if (snapshot.value?.clipboard?.automatic_ocr === false) return
  const ids = rows
    .filter((item) => item.has_image && item.ocr_version < OCR_VERSION && !scheduledOCR.has(item.id))
    .map((item) => item.id)
  if (!ids.length) return
  for (const id of ids) scheduledOCR.add(id)
  void backend.invoke('clipboard.indexOCR', { ids }).catch(() => {
    for (const id of ids) scheduledOCR.delete(id)
  })
}

function metaFor(item: LibraryItem): ItemMeta {
  return {
    remote: item.remote,
    available: item.available,
    originDevice: item.source_device_id ? deviceNameMap.value[item.source_device_id] ?? null : null,
    ocrText: item.ocr_text,
  }
}
function ocrHit(item: LibraryItem): boolean {
  const query = search.value.trim().toLowerCase()
  return query !== '' && !!item.ocr_text && item.ocr_text.toLowerCase().includes(query)
}

function pasteTargetName(): string | null {
  const target = snapshot.value?.clipboard?.paste_target
  return typeof target === 'string' && target ? target : null
}

/** 计数依赖 state 里的 counts_revision；与 state 并发会让计数结果被判为过期。 */
async function refreshLibrary() {
  await refreshState()
  await refreshCounts()
}

async function loadSources() {
  try {
    const rows = await backend.invoke('clipboard.sources')
    if (Array.isArray(rows)) sources.value = rows.map((row: any) => ({ id: String(row.id), name: String(row.name ?? '') }))
  } catch {
    sources.value = library.devices.map((device) => ({ id: device.id, name: device.name }))
  }
}

async function loadPage(append = false) {
  if (append && (loadingMore.value || nextOffset === null)) return
  const current = ++generation
  if (append) loadingMore.value = true
  else { loading.value = true; searching.value = true }
  error.value = ''
  try {
    const page = await queryPage(listID.value, search.value, source.value, append ? nextOffset ?? 0 : 0, append ? cursor : null)
    if (disposed || current !== generation) return
    items.value = append ? [...items.value, ...page.items] : page.items
    cursor = page.next_cursor ?? null
    nextOffset = page.next_offset ?? null
    if (!items.value.some((item) => item.id === selectedID.value)) selectedID.value = visibleItems.value[0]?.id ?? null
    scheduleOCR(page.items)
    void refreshState()
  } catch (e) {
    if (disposed || current !== generation) return
    error.value = errorMessage(e)
    if (!append) items.value = []
  } finally {
    if (!disposed && current === generation) { loading.value = false; loadingMore.value = false; searching.value = false }
  }
}

function scheduleSearch() {
  clearTimeout(searchTimer)
  searching.value = true
  searchTimer = setTimeout(() => { void loadPage(false) }, 120)
}

function rowElement(id: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-id="${id}"]`)
}

function selectItem(item: LibraryItem) {
  selectedID.value = item.id
  if (previewID.value) void showPreview(item)
  if (item.remote && !item.available) wantPayload(item.id)
}

// 快速预览：非文件条目按原生 ClipboardPreviewPolicy 决定是否可预览；
// 文件条目改为在访达中显示。浮窗跟随选择移动，不抢面板键盘。
function togglePreview() {
  if (previewID.value) { closePreview(); return }
  const item = selected.value
  if (!item) return
  if (contentTypeOf(item) === 'files') { void revealItem(item); return }
  if (shouldPreview(item, displayMode.value)) void showPreview(item)
}

async function showPreview(item: LibraryItem, attempt = 0) {
  if (!shouldPreview(item, displayMode.value)) { closePreview(); return }
  const element = rowElement(item.id)
  if (!element) {
    // 行可能还在滚动入位（原生同样重试 15 次）。
    if (attempt < 15 && !disposed) requestAnimationFrame(() => { void showPreview(item, attempt + 1) })
    return
  }
  const rect = element.getBoundingClientRect()
  previewID.value = item.id
  try {
    await backend.previewShow({
      id: item.id,
      wait: item.remote && !item.available,
      x: Math.round(rect.left), y: Math.round(rect.top),
      width: Math.round(rect.width), height: Math.round(rect.height),
    })
  } catch (e) {
    previewID.value = null
    error.value = errorMessage(e)
  }
}

function closePreview() {
  if (!previewID.value) return
  previewID.value = null
  void backend.previewHide().catch(() => undefined)
}

async function copyItem(item: LibraryItem, paste: boolean) {
  if (copying.value) return
  copying.value = true
  progressTimer = setTimeout(() => (showCopyProgress.value = true), 150)
  try {
    await backend.invoke(paste ? 'clipboard.paste' : 'clipboard.copy', { id: item.id })
    await backend.invoke('panel.hide')
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    copying.value = false
    clearTimeout(progressTimer)
    progressTimer = setTimeout(() => (showCopyProgress.value = false), 150)
  }
}

/** 原生在保存面板、外部编辑器与“预览”前先收起面板；窗口事件是异步的，稍等一拍。 */
async function hidePanel() {
  await backend.invoke('panel.hide')
  await new Promise((resolve) => setTimeout(resolve, 60))
}

async function saveTitle(title: string) {
  const item = titleItem.value
  titleItem.value = null
  if (!item) return
  try { await renameItem(item.id, title) }
  catch (e) { error.value = errorMessage(e) }
}

async function pastePlain(item: LibraryItem) {
  if (copying.value) return
  copying.value = true
  try {
    await backend.invoke('clipboard.paste', { id: item.id, plain_text: true })
    await backend.invoke('panel.hide')
  } catch (e) { error.value = errorMessage(e) }
  finally { copying.value = false }
}

// 原生在打开保存面板、外部编辑器与“预览”前先关闭面板：它们是独立窗口，
// 面板留在下面只会挡住保存位置。
async function saveItem(item: LibraryItem) {
  try {
    closePreview()
    await hidePanel()
    await backend.invoke('clipboard.save', { id: item.id, has_image: item.has_image, file_count: item.file_count })
  } catch (e) { error.value = errorMessage(e) }
}
async function editItem(item: LibraryItem, closeFirst = false) {
  try {
    if (closeFirst) {
      closePreview()
      await hidePanel()
    }
    await backend.invoke('clipboard.edit', { id: item.id, has_image: item.has_image, file_count: item.file_count, title: item.title })
  } catch (e) { error.value = errorMessage(e) }
}
async function revealItem(item: LibraryItem) {
  try { await backend.invoke('clipboard.reveal', { id: item.id }) }
  catch (e) { error.value = errorMessage(e) }
}
async function openInPreview(item: LibraryItem) {
  try {
    closePreview()
    await hidePanel()
    await backend.invoke('clipboard.openInPreview', { id: item.id })
  } catch (e) { error.value = errorMessage(e) }
}

async function deleteSelected() {
  const item = selected.value
  if (!item) return
  try {
    await deleteItems([item])
    const index = visibleItems.value.findIndex((row) => row.id === item.id)
    items.value = items.value.filter((row) => row.id !== item.id)
    if (previewID.value === item.id) closePreview()
    selectedID.value = items.value[index]?.id ?? items.value[index - 1]?.id ?? null
    if (previewID.value) {
      const next = items.value.find((row) => row.id === selectedID.value)
      if (next) void nextTick(() => showPreview(next))
    }
  } catch (e) { error.value = errorMessage(e) }
}

function showItemMenu(item: LibraryItem, event: MouseEvent) {
  event.preventDefault()
  selectItem(item)
  menu.value = {
    x: event.clientX,
    y: event.clientY,
    items: itemMenu({
      item,
      title: item.title,
      meta: metaFor(item),
      copyEnabled: true,
      workflowsEnabled: snapshot.value?.workflows?.enabled !== false,
      pasteTarget: pasteTargetName(),
      onPaste: () => void copyItem(item, true),
      onPastePlainText: () => void pastePlain(item),
      showsPreview: shouldPreview(item, displayMode.value),
      onPreview: () => { selectedID.value = item.id; void showPreview(item) },
      onCopy: () => void copyItem(item, false),
      onEditTitle: () => { titleItem.value = item },
      onEditText: () => void editItem(item),
      onDelete: () => void deleteSelected(),
      fail: (value) => (error.value = value),
    }),
  }
}

function showSourceMenu(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  menu.value = {
    x: rect.left,
    y: rect.bottom + 2,
    items: [
      { label: '全部设备', checked: source.value === null, run: () => { source.value = null; void loadPage(false) } },
      ...sources.value.map((choice) => ({
        label: choice.name,
        checked: source.value === choice.id,
        run: () => { source.value = choice.id; void loadPage(false) },
      })),
    ],
  }
}

function showMoreMenu(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  menu.value = {
    x: Math.max(4, rect.right - 200),
    y: rect.bottom + 4,
    items: [
      { label: '打开 DeskFuse', run: () => void backend.invoke('app.showMain') },
      { label: '文件暂存区', run: () => void backend.invoke('shelf.show') },
      { separator: true },
      { label: '设置…', run: () => void backend.invoke('native.openPage', { page: 'settings' }) },
      { label: '反馈问题…', run: () => void backend.invoke('support.report', {}) },
      { separator: true },
      { label: '退出 DeskFuse', run: () => void backend.invoke('app.quit') },
    ],
  }
}

function showFooterMenu(event: MouseEvent, kind: 'workflow' | 'share') {
  const item = selected.value
  if (!item) return
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const type = contentTypeOf(item)
  menu.value = {
    x: Math.max(4, rect.right - 220),
    y: rect.top - 8,
    items: [{
      label: kind === 'workflow' ? '使用工作流' : '发送到设备',
      load: async () => {
        if (kind === 'share') {
          const peers = await loadPeers()
          if (!peers.length) return [{ label: '请先添加一台设备', disabled: true }]
          return peers.map((peer) => ({ label: peer.name, disabled: !peer.connected, run: () => void shareToPeer(item.id, peer.id) }))
        }
        const flows = (await loadWorkflows()).filter((flow) => flow.enabled && (!flow.acceptsKnown || flow.accepts.includes(type)))
        if (!flows.length) return [{ label: '没有适合此内容的已启用工作流', disabled: true }]
        return flows.map((flow) => ({ label: flow.name, run: () => void runWorkflow(flow.id, item.id) }))
      },
    }],
  }
}

function showAll() {
  void backend.invoke('clipboard.showInMain', {
    query: search.value, source: source.value, list_id: listID.value, selected_id: selectedID.value,
  })
}

function selectList(id: string | null) {
  listID.value = id
  selectedID.value = null
  void loadPage(false)
}

function clearFilters() {
  search.value = ''
  source.value = null
  listID.value = null
  void loadPage(false)
}

/** 瀑布流方向键：与原生 ClipboardMasonryNavigation 相同的列内就近移动。 */
function gridTarget(direction: 'left' | 'right' | 'up' | 'down'): string | null {
  const layout = gridColumns.value.map((column) => {
    let y = 0
    return column.map((item) => {
      const height = estimatedHeight(item)
      const center = y + height / 2
      y += height + 12
      return { item, center }
    })
  })
  let columnIndex = -1
  let rowIndex = -1
  for (let c = 0; c < layout.length; c += 1) {
    const index = layout[c].findIndex((entry) => entry.item.id === selectedID.value)
    if (index >= 0) { columnIndex = c; rowIndex = index; break }
  }
  if (columnIndex < 0) return null
  const current = layout[columnIndex][rowIndex]
  if (direction === 'left' || direction === 'right') {
    const adjacent = layout[columnIndex + (direction === 'right' ? 1 : -1)]
    if (!adjacent?.length) return null
    return adjacent.reduce((best, entry) => (Math.abs(entry.center - current.center) < Math.abs(best.center - current.center) ? entry : best)).item.id
  }
  const downward = direction === 'down'
  const candidates = layout[columnIndex].filter((entry) => (downward ? entry.center > current.center : entry.center < current.center))
  if (!candidates.length) return null
  return candidates.reduce((best, entry) => (Math.abs(entry.center - current.center) < Math.abs(best.center - current.center) ? entry : best)).item.id
}

function move(direction: 'left' | 'right' | 'up' | 'down') {
  const rows = visibleItems.value
  if (!rows.length) return
  let targetID: string | null = null
  if (displayMode.value === 'grid') {
    targetID = gridTarget(direction)
  } else if (direction === 'up' || direction === 'down') {
    const index = rows.findIndex((item) => item.id === selectedID.value)
    const next = Math.max(0, Math.min(rows.length - 1, (index < 0 ? 0 : index) + (direction === 'down' ? 1 : -1)))
    targetID = rows[next].id
  }
  if (!targetID) return
  selectedID.value = targetID
  const item = rows.find((row) => row.id === targetID)
  void nextTick(() => {
    rowElement(targetID!)?.scrollIntoView({ block: 'nearest' })
    if (previewID.value && item) requestAnimationFrame(() => { void showPreview(item) })
  })
  if (item?.remote && !item.available) wantPayload(item.id)
}

function keydown(event: KeyboardEvent) {
  if (event.key === 'Meta' || event.key === 'Control') { commandHeld.value = true; return }
  const editing = (event.target as HTMLElement | null)?.tagName === 'INPUT'
  const meta = event.metaKey || event.ctrlKey
  if (meta && event.key.toLowerCase() === 'f') { event.preventDefault(); searchBox.value?.focus(); return }
  if (meta && event.key.toLowerCase() === 'c') {
    const field = event.target as HTMLInputElement | null
    if (editing && field && (field.selectionStart ?? 0) !== (field.selectionEnd ?? 0)) return
    event.preventDefault(); if (selected.value) void copyItem(selected.value, false); return
  }
  if (meta && !editing) {
    const key = event.key.toLowerCase()
    if (key === 's' && selected.value) { event.preventDefault(); void saveItem(selected.value); return }
    if (key === 'e' && selected.value) { event.preventDefault(); void editItem(selected.value, true); return }
    if (key === 'o' && selected.value && (selected.value.file_count > 0 || selected.value.has_image)) {
      event.preventDefault(); void openInPreview(selected.value); return
    }
    if (key === 'r' && event.shiftKey && selected.value?.file_count) { event.preventDefault(); void revealItem(selected.value); return }
    if (/^[1-9]$/.test(event.key)) {
      const item = visibleItems.value[Number(event.key) - 1]
      if (item) { event.preventDefault(); void copyItem(item, true) }
      return
    }
  }
  if (event.key === 'Escape') {
    event.preventDefault()
    if (menu.value) { menu.value = null; return }
    if (previewID.value) { closePreview(); return }
    void backend.invoke('panel.hide')
    return
  }
  if (event.key === ' ' && !editing) { event.preventDefault(); togglePreview(); return }
  if (event.key === 'Enter' && !editing) { event.preventDefault(); if (selected.value) void copyItem(selected.value, true); return }
  if ((event.key === 'Backspace' || event.key === 'Delete') && !editing) { event.preventDefault(); void deleteSelected(); return }
  if (event.key === 'ArrowDown') { event.preventDefault(); move('down'); return }
  if (event.key === 'ArrowUp') { event.preventDefault(); move('up'); return }
  if (event.key === 'ArrowLeft' && displayMode.value === 'grid') { event.preventDefault(); move('left'); return }
  if (event.key === 'ArrowRight' && displayMode.value === 'grid') { event.preventDefault(); move('right'); return }
}
function keyup(event: KeyboardEvent) {
  if (event.key === 'Meta' || event.key === 'Control') commandHeld.value = false
}

function onScroll() {
  const box = rowsBox.value
  if (!box) return
  if (box.scrollTop + box.clientHeight >= box.scrollHeight - 80) void loadPage(true)
}

function onPanelShown() {
  closePreview()
  selectedID.value = null
  cursor = null; nextOffset = 0
  void loadSources(); void refreshLibrary(); void loadPage(false)
  searchBox.value?.focus()
}

onMounted(() => {
  ;(window as any).__deskfusePanelShown = onPanelShown
  // popover 持有键盘时（系统 popover 打开动画结束后可能重新夺回），
  // 把 Space/Esc/方向键/回车转回面板，和原生 popover 的 key monitor 一致。
  ;(window as any).__deskfusePreviewKey = (key: string) => {
    if (!previewID.value) return
    if (key === 'Escape' || key === ' ') { closePreview(); return }
    if (key === 'Enter') { if (selected.value) void copyItem(selected.value, true); return }
    if (key === 'ArrowDown') { move('down'); return }
    if (key === 'ArrowUp') { move('up'); return }
    if (key === 'ArrowLeft') { move('left'); return }
    if (key === 'ArrowRight') { move('right') }
  }
  void loadSources()
  void refreshLibrary()
  void loadPage(false)
  window.addEventListener('keydown', keydown)
  window.addEventListener('keyup', keyup)
  searchBox.value?.focus()
  void refreshSnapshot()
  snapshotTimer = setInterval(() => { if (document.visibilityState !== 'hidden') void refreshSnapshot() }, 1500)
  libraryTimer = setInterval(() => {
    if (document.visibilityState === 'hidden') return
    void (async () => {
      await refreshLibrary()
      if (!search.value && !listID.value && source.value === null) await loadPage(false)
    })()
  }, 30000)
})
onBeforeUnmount(() => {
  disposed = true; ++generation
  closePreview()
  clearTimeout(searchTimer); clearTimeout(progressTimer); clearInterval(libraryTimer); clearInterval(snapshotTimer)
  window.removeEventListener('keydown', keydown)
  window.removeEventListener('keyup', keyup)
  delete (window as any).__deskfusePanelShown
  delete (window as any).__deskfusePreviewKey
})

function setDisplayMode(mode: 'list' | 'grid') {
  displayMode.value = mode
  localStorage.setItem('deskfuse.clipboard.displayMode', mode)
}
</script>

<template>
  <div class="panel">
    <header class="head">
      <Icon name="copy" :size="16" />
      <strong>最近剪贴板</strong>
      <span class="spacer" />
      <div class="modes" role="group" aria-label="显示方式">
        <button type="button" :class="{ active: displayMode === 'list' }" aria-label="列表" title="列表" @click="setDisplayMode('list')"><List :size="15" /></button>
        <button type="button" :class="{ active: displayMode === 'grid' }" aria-label="瀑布流" title="瀑布流" @click="setDisplayMode('grid')"><LayoutGrid :size="15" /></button>
      </div>
      <button type="button" class="icon source" :title="`来源：${sourceName}`" aria-label="来源设备" @click="showSourceMenu">
        <span class="source-name">{{ sourceName }}</span>
      </button>
      <button type="button" class="icon" title="打开文件暂存区" aria-label="文件暂存区" @click="backend.invoke('shelf.show')"><Icon name="tray" :size="16" /></button>
      <button type="button" class="icon" title="更多操作" aria-label="更多操作" @click="showMoreMenu"><MoreHorizontal :size="16" /></button>
    </header>

    <label class="search">
      <Search :size="13" />
      <input ref="searchBox" v-model="search" placeholder="搜索最近内容" aria-label="搜索最近内容" @input="scheduleSearch" />
      <button v-if="search" type="button" class="clear" aria-label="清除搜索" @click="search = ''; scheduleSearch()"><Icon name="xmarkFill" :size="14" /></button>
      <kbd v-else :class="{ held: commandHeld }">⌘ F</kbd>
    </label>

    <div class="tabs-row">
      <div class="tabs" role="tablist" aria-label="剪贴板列表">
        <button type="button" role="tab" :aria-selected="listID === null" :class="{ active: listID === null }" @click="selectList(null)">
          全部<small v-if="library.counts['all']">{{ library.counts['all'] }}</small>
        </button>
        <button v-for="list in lists" :key="list.id" type="button" role="tab" :aria-selected="listID === list.id" :class="{ active: listID === list.id }" @click="selectList(list.id)">
          {{ list.name }}<small v-if="library.counts[list.id]">{{ library.counts[list.id] }}</small>
        </button>
      </div>
      <button type="button" class="add" title="新建列表" aria-label="新建列表" @click="editingList = newList()"><Icon name="plusCircle" :size="16" /></button>
    </div>
    <div class="divider" />

    <div ref="rowsBox" class="rows" role="listbox" aria-label="剪贴板历史" @scroll="onScroll">
      <div v-if="!snapshotReady || starting" class="state">{{ starting ? '正在启动…' : '正在搜索…' }}</div>
      <div v-else-if="searching && !items.length" class="state">正在搜索…</div>
      <div v-else-if="!visibleItems.length" class="empty">
        <Icon :name="historyEmpty ? 'clipboard' : 'magnifyingglass'" :size="24" class="empty-icon" />
        <strong>{{ !engineReady ? '引擎尚未就绪' : error ? '搜索失败' : historyEmpty ? '还没有剪贴板记录' : '没有匹配内容' }}</strong>
        <p>{{ !engineReady ? (appError || '请稍后重试') : error || (historyEmpty ? '复制文字、图片或文件后，会显示在这里' : '试试其他关键词，或清除筛选') }}</p>
        <button v-if="!historyEmpty && hasFilters" type="button" class="button small" @click="clearFilters">清除筛选</button>
      </div>
      <template v-else>
        <template v-if="displayMode === 'list'">
          <div v-for="(item, index) in visibleItems" :key="item.id" :data-id="item.id" class="panel-row"
               :class="{ picked: item.id === selectedID }" role="option" :aria-selected="item.id === selectedID"
               @click="selectItem(item)" @dblclick="copyItem(item, true)" @contextmenu="showItemMenu(item, $event)">
            <HistoryRow
              :item="item" :meta="metaFor(item)" :selected="false" :focused="false"
              :ocr-hit="ocrHit(item)" :shortcut="index < 9 ? index + 1 : null" :show-shortcut="commandHeld" :hoverable="false" :identify="false"
            />
          </div>
        </template>
        <div v-else class="grid">
          <div v-for="(column, columnIndex) in gridColumns" :key="columnIndex" class="grid-column">
            <PanelCard
              v-for="item in column" :key="item.id" :item="item" :meta="metaFor(item)"
              :selected="item.id === selectedID" :ocr-hit="ocrHit(item)" :index="indexOf(item)" :show-shortcut="commandHeld"
              @click="selectItem(item)" @dblclick="copyItem(item, true)" @contextmenu="showItemMenu(item, $event)"
              @edit-title="titleItem = item"
            />
          </div>
        </div>
        <div v-if="loadingMore" class="state small">正在加载更多…</div>
      </template>
    </div>

    <footer class="foot">
      <p v-if="message" class="error" role="alert"><Icon name="warning" :size="12" />{{ message }}</p>
      <div class="foot-row">
        <button type="button" class="button" @click="showAll">查看全部<ArrowUpRight :size="13" /></button>
        <span class="spacer" />
        <span v-if="showCopyProgress" class="spin" role="status" aria-label="正在准备剪贴板内容" />
        <button v-if="selected" type="button" class="button" :disabled="copying || !engineReady" @click="showFooterMenu($event, 'workflow')"><Icon name="stack" :size="13" />使用工作流…</button>
        <button v-if="selected" type="button" class="button" :disabled="copying || !engineReady" @click="showFooterMenu($event, 'share')"><Icon name="paperplane" :size="13" />发送到设备</button>
      </div>
      <WorkflowRunStatus :runs="workflowRuns" :error="workflowError" compact />
      <p v-if="appError" class="error strong">{{ appError }}</p>
    </footer>

    <ContextMenu v-if="menu" :items="menu.items" :x="menu.x" :y="menu.y" @close="menu = null" />
    <ListEditorDialog v-if="editingList" :initial="editingList" @saved="(id: string) => { editingList = null; selectList(id) }" @close="editingList = null" />
    <TitleEditorDialog v-if="titleItem" :title="titleItem.title" @save="saveTitle" @close="titleItem = null" />
  </div>
</template>

<style scoped>
.panel{display:flex;flex-direction:column;height:100vh;overflow:hidden;background:var(--surface);color:var(--text)}
.head{display:flex;align-items:center;gap:10px;padding:16px 16px 10px}
.head strong{font-size:15px;font-weight:600}
.spacer{flex:1}
.modes{display:flex;gap:2px;background:var(--control);border-radius:8px;padding:2px}
.modes button{display:grid;place-items:center;width:34px;height:30px;border-radius:8px;color:var(--secondary)}
.modes button.active{background:color-mix(in srgb,var(--accent) 16%,transparent);color:var(--accent)}
.icon{display:inline-flex;align-items:center;gap:4px;height:30px;padding:0 6px;border-radius:8px;color:var(--secondary)}
.icon:hover{background:var(--control);color:var(--text)}
.source{padding:0 2px}
.source-name{font-size:11px;line-height:1.2;max-width:100px;min-width:72px;text-align:right;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.search{display:flex;align-items:center;gap:8px;margin:0 16px 10px;padding:0 11px;height:32px;border-radius:8px;background:var(--control)}
.search input{flex:1;border:0;background:none;outline:none;font:13px/1.4 -apple-system,BlinkMacSystemFont,sans-serif;color:inherit;min-width:0;min-height:0;padding:0}
.search kbd{font-size:11px;color:var(--secondary);border:0;background:none;padding:0;opacity:0;transition:opacity .12s ease}
.search kbd.held{opacity:1}
.clear{display:grid;place-items:center;color:var(--secondary)}
.tabs-row{display:flex;align-items:center;gap:6px;padding:0 16px 12px}
.tabs{flex:1;min-width:0;display:flex;align-items:center;gap:6px;overflow-x:auto}
.tabs button{display:inline-flex;align-items:center;gap:6px;height:34px;padding:0 11px;border-radius:100px;background:transparent;color:var(--secondary);font-size:12px;white-space:nowrap;flex:none}
.tabs button.active{background:color-mix(in srgb,var(--accent) 18%,transparent);color:var(--accent)}
.tabs small{font-size:11px;opacity:.8;font-variant-numeric:tabular-nums}
.add{flex:none;width:32px;height:32px;display:grid;place-items:center;border-radius:8px;padding:0;color:var(--secondary)}
.add:hover{background:var(--control);color:var(--text)}
.divider{height:.5px;background:var(--line);flex:none}
.rows{flex:1;min-height:0;overflow:auto;padding:10px 12px}
.rows .panel-row{position:relative;display:flex;align-items:stretch;cursor:default;min-height:64px;border-radius:8px;border:1px solid transparent;overflow:hidden}
.rows .panel-row+.panel-row{margin-top:3px}
.rows .panel-row.picked{background:color-mix(in srgb,var(--accent) 18%,transparent);border-color:color-mix(in srgb,var(--accent) 25%,transparent)}
.rows .panel-row :deep(.row){flex:1;min-width:0}
.rows :deep(.title){-webkit-line-clamp:2}
.grid{display:flex;align-items:flex-start;gap:12px}
.grid-column{flex:1;min-width:0;display:flex;flex-direction:column;gap:12px}
.state{padding:28px 12px;text-align:center;color:var(--secondary);font-size:12px}
.state.small{padding:10px}
.empty{display:flex;flex-direction:column;align-items:center;text-align:center;gap:10px;padding:45px 15px;color:var(--secondary)}
.empty-icon{color:var(--secondary);opacity:.7}
.empty strong{font-size:13px;color:var(--text)}
.empty p{font-size:12px;max-width:300px}
.empty .button{min-height:28px;padding:0 12px;font-size:12px}
.foot{padding:13px 16px;border-top:.5px solid var(--line);display:flex;flex-direction:column;gap:8px}
.foot-row{display:flex;align-items:center;gap:8px}
.foot .button{min-height:28px;padding:0 10px;font-size:12px;display:inline-flex;align-items:center;gap:5px}
.foot .button:disabled{opacity:.45}
.error{font-size:11px;color:var(--warning);margin:0;display:flex;align-items:flex-start;gap:5px;line-height:1.35}
.error.strong{color:var(--danger)}
.spin{width:12px;height:12px;border:2px solid var(--line);border-top-color:var(--accent);border-radius:50%;animation:spin 1s linear infinite}
@keyframes spin{to{transform:rotate(360deg)}}
</style>
