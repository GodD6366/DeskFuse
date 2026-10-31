<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { ItemMeta, LibraryChoice, LibraryCursor, LibraryItem } from '../types/clipboard'
import {
  library, loadLibrary, queryPage, queryTotal, renameItem, deleteItems, errorMessage, loadContent,
  deviceNames, displaySourceName,
} from './clipboard/library'
import { backend } from '../services/backend'
import { copyItem } from './clipboard/actions'
import {
  filtersOnlyImages, masonryColumns, estimatedHeight, startsDayGroup, dayTitle, normalizeItem,
} from './clipboard/format'
import { itemMenu } from './clipboard/menus'
import { snapshotWorkflowError, snapshotWorkflowRuns } from './clipboard/workflowRuns'
import WorkspaceSidebar from './clipboard/WorkspaceSidebar.vue'
import HistoryRow from './clipboard/HistoryRow.vue'
import HistoryCard from './clipboard/HistoryCard.vue'
import PreviewPane from './clipboard/PreviewPane.vue'
import TitleEditorDialog from './clipboard/TitleEditorDialog.vue'
import ContentEditorDialog from './clipboard/ContentEditorDialog.vue'
import ConfirmDialog from './clipboard/ConfirmDialog.vue'
import ContextMenu from './clipboard/ContextMenu.vue'
import Icon from './clipboard/Icon.vue'
import FeatureGate from './features/FeatureGate.vue'
import type { MenuItem } from './clipboard/menu'

const props = defineProps<{ snapshot: Record<string, any> | null }>()
const emit = defineEmits<{ navigate: [page: string] }>()

// 显示模式按原生分开记忆：文本/文件列表默认 list，仅图片筛选默认 grid。
const TEXT_MODE_KEY = 'deskfuse.clipboard.textDisplayMode'
const IMAGE_MODE_KEY = 'deskfuse.clipboard.imageDisplayMode'
const NAVIGATION_TOKEN_KEY = 'deskfuse.clipboard.navigationToken'
function readMode(key: string, fallback: 'grid' | 'list'): 'grid' | 'list' {
  try {
    const value = localStorage.getItem(key)
    return value === 'list' || value === 'grid' ? value : fallback
  } catch { return fallback }
}

const search = ref('')
const source = ref<string | null>(null)
const listID = ref<string | null>(null)
const textMode = ref<'grid' | 'list'>(readMode(TEXT_MODE_KEY, 'list'))
const imageMode = ref<'grid' | 'list'>(readMode(IMAGE_MODE_KEY, 'grid'))
const items = ref<LibraryItem[]>([])
const selectedID = ref<string | null>(null)
const checked = ref<Set<string>>(new Set())
const loading = ref(false)
const loadingMore = ref(false)
const error = ref('')
const pageError = ref('')
const nextOffset = ref<number | null>(null)
const cursor = ref<LibraryCursor | null>(null)
const filteredTotal = ref<number | null>(null)
const titleItem = ref<LibraryItem | null>(null)
const editItem = ref<LibraryItem | null>(null)
const editText = ref('')
const editRuns = ref<Array<{ text: string; bold?: boolean; italic?: boolean; underline?: boolean; strike?: boolean }>>([])
const pendingDelete = ref<LibraryItem[] | null>(null)
const menu = ref<{ x: number; y: number; items: MenuItem[] } | null>(null)
const body = ref<HTMLElement | null>(null)
const historyPane = ref<HTMLElement | null>(null)
const searchInput = ref<HTMLInputElement | null>(null)
const searchFocused = ref(false)
const columnCount = ref(2)
const bridgeSources = ref<LibraryChoice[]>([])
const dayRevision = ref(0)
/** 「查看全部」带入的选中项；跨页到达前保持等待，应用一次后清空。 */
const pendingSelectedID = ref<string | null>(null)
/** 本地已删除的 id：快照轮询还会带回来一两次，合并前先剔除。 */
const deletedIDs = ref<Set<string>>(new Set())
const marqueeRect = ref<{ left: number; top: number; width: number; height: number } | null>(null)

let generation = 0
let disposed = false
let mounted = false
let timer: ReturnType<typeof setTimeout> | undefined
let historyObserver: ResizeObserver | undefined
let libraryTimer: ReturnType<typeof setInterval> | undefined
let dayTimer: ReturnType<typeof setInterval> | undefined
let dayKey = ''
let navigationToken = ''

let dragOrigin: { x: number; y: number } | null = null
let dragBase = new Set<string>()
let dragFrames: Array<{ id: string; left: number; top: number; right: number; bottom: number }> = []
let marqueeActive = false

const featureEnabled = computed(() => props.snapshot?.features?.clipboard !== false)
const imageFilter = computed(() => filtersOnlyImages(library.lists.find((list) => list.id === listID.value)))
const mode = computed<'grid' | 'list'>(() => (imageFilter.value ? imageMode.value : textMode.value))

const snapshotItems = computed<LibraryItem[]>(() => {
  const rows = props.snapshot?.clipboard?.history
  return Array.isArray(rows) ? rows.map((row) => normalizeItem(row as Record<string, any>)) : []
})
/** 只有未筛选的主历史视图才把快照里的近期条目并进来（原生 rebuildPresentation）。 */
const mergeSnapshot = computed(() => search.value.trim() === '' && source.value === null && listID.value === null)

function idOrder(a: string, b: string): number {
  if (/^\d+$/.test(a) && /^\d+$/.test(b)) {
    if (a.length !== b.length) return b.length - a.length
    return a < b ? 1 : a > b ? -1 : 0
  }
  return a < b ? 1 : a > b ? -1 : 0
}

const mergedItems = computed(() => {
  const base = deletedIDs.value.size ? items.value.filter((item) => !deletedIDs.value.has(item.id)) : items.value
  if (!mergeSnapshot.value) return base
  const seen = new Set(base.map((item) => item.id))
  const fresh = snapshotItems.value.filter((item) => !deletedIDs.value.has(item.id) && !seen.has(item.id))
  if (!fresh.length) return base
  const rows = [...base, ...fresh]
  rows.sort((left, right) => {
    const a = left.sort_at_ms ?? left.copied_at_ms ?? 0
    const b = right.sort_at_ms ?? right.copied_at_ms ?? 0
    return a === b ? idOrder(left.id, right.id) : b - a
  })
  return rows
})
const deviceNameMap = computed(() => deviceNames(props.snapshot))
const visibleItems = computed(() =>
  mergedItems.value
    .filter((item) => !(item.file_count > 0 && item.remote && !item.available))
    .map((item) => {
      const name = displaySourceName(item.source, item.source_device_id, props.snapshot)
      return name === item.source ? item : { ...item, source: name }
    }))
const historyEmpty = computed(() => {
  const rows = props.snapshot?.clipboard?.history
  return Array.isArray(rows) ? rows.length === 0 : mergedItems.value.length === 0
})
const columns = computed(() => masonryColumns(visibleItems.value, columnCount.value))
const selection = computed(() => checked.value.size ? checked.value : selectedID.value ? new Set([selectedID.value]) : new Set<string>())
const selectionCount = computed(() => selection.value.size)
const selected = computed(() => visibleItems.value.find((item) => item.id === selectedID.value) ?? null)
const hasMore = computed(() => cursor.value !== null || nextOffset.value !== null)
const sources = computed<LibraryChoice[]>(() => {
  const seen = new Set<string>()
  const rows: LibraryChoice[] = []
  for (const choice of bridgeSources.value) {
    if (seen.has(choice.id)) continue
    seen.add(choice.id)
    rows.push(choice)
  }
  // 桥接口不可用时回退到库里的设备列表（含本机显示名映射）。
  for (const device of library.devices) {
    if (seen.has(device.id)) continue
    seen.add(device.id)
    rows.push({ id: device.id, name: displaySourceName(device.name, device.id, props.snapshot) })
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name, 'zh-CN', { numeric: true }))
})
const sourceName = computed(() =>
  source.value === null
    ? '所有设备'
    : sources.value.find((choice) => choice.id === source.value)?.name
      ?? displaySourceName(source.value, source.value, props.snapshot))
const hasFilters = computed(() => search.value.trim() !== '' || source.value !== null || listID.value !== null)
const headerTotal = computed(() =>
  search.value.trim() === '' && source.value === null
    ? library.counts[listID.value ?? 'all'] ?? null
    : filteredTotal.value)
const notice = computed(() => error.value || library.error || props.snapshot?.clipboard?.error || '')
const workflowRuns = computed(() => snapshotWorkflowRuns(props.snapshot))
const workflowError = computed(() => snapshotWorkflowError(props.snapshot))

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

async function loadSources() {
  if (disposed) return
  try {
    const rows = await backend.invoke('clipboard.sources')
    if (disposed) return
    if (Array.isArray(rows)) {
      bridgeSources.value = rows.map((row: any) => ({ id: String(row.id), name: String(row.name ?? '') }))
    }
  } catch {
    // 宿主未提供来源列表时保持为空，sources 计算属性会回退到 library.devices。
    if (!disposed) bridgeSources.value = []
  }
}

/**
 * 与原生 ClipboardView 的 selection 保持同步：首次加载选中最新一条；
 * 选中项消失（删除、换筛选、导航目标就绪）后落到列表第一条。
 */
function ensureSelection() {
  const list = visibleItems.value
  if (pendingSelectedID.value) {
    const target = list.find((item) => item.id === pendingSelectedID.value)
    pendingSelectedID.value = null
    const next = target?.id ?? list[0]?.id ?? null
    selectedID.value = next
    checked.value = new Set()
    return
  }
  if (selectedID.value && list.some((item) => item.id === selectedID.value)) return
  if (checked.value.size) {
    const first = list.find((item) => checked.value.has(item.id))
    if (first) { selectedID.value = first.id; return }
  }
  selectedID.value = list[0]?.id ?? null
  checked.value = new Set()
}

async function loadFilteredTotal(current: number) {
  if (search.value.trim() === '' && source.value === null) { filteredTotal.value = null; return }
  try {
    const value = await queryTotal(listID.value, search.value, source.value)
    if (!disposed && current === generation) filteredTotal.value = value
  } catch {
    if (!disposed && current === generation) filteredTotal.value = null
  }
}

/** 原生 ClipboardOCRIndexer.version；低于它且尚未调度过的图片才需要识别。 */
const OCR_VERSION = 1
const scheduledOCR = new Set<string>()

/** 原生 scheduleOCR：自动识别关闭时不调度；同一 id 只提交一次，version 由宿主回写。 */
function scheduleOCR(rows: LibraryItem[]) {
  if (props.snapshot?.clipboard?.automatic_ocr === false) return
  const ids = rows
    .filter((item) => item.has_image && item.ocr_version < OCR_VERSION && !scheduledOCR.has(item.id))
    .map((item) => item.id)
  if (!ids.length) return
  for (const id of ids) scheduledOCR.add(id)
  void backend.invoke('clipboard.indexOCR', { ids }).catch(() => {
    // 提交失败时放开，下一轮刷新可以重试。
    for (const id of ids) scheduledOCR.delete(id)
  })
}

async function refresh(reset = true) {
  if (!mounted || !featureEnabled.value) return
  if (!reset && (!hasMore.value || loading.value || loadingMore.value)) return
  const current = reset ? ++generation : generation
  const offset = reset ? 0 : nextOffset.value ?? items.value.length
  const requestCursor = reset ? null : cursor.value
  if (reset) { loading.value = true; error.value = ''; pageError.value = '' } else { loadingMore.value = true }
  if (reset) { nextOffset.value = null; cursor.value = null }
  try {
    const page = await queryPage(listID.value, search.value, source.value, offset, requestCursor)
    if (disposed || current !== generation) return
    const rows = reset ? [] : [...items.value]
    const ids = new Set(rows.map((item) => item.id))
    for (const item of page.items) if (!ids.has(item.id)) { rows.push(item); ids.add(item.id) }
    items.value = rows
    cursor.value = page.next_cursor
    nextOffset.value = page.next_offset
    scheduleOCR(page.items)
    if (checked.value.size) {
      const present = new Set(visibleItems.value.map((item) => item.id))
      checked.value = new Set([...checked.value].filter((id) => present.has(id)))
    }
    if (selectedID.value && !visibleItems.value.some((item) => item.id === selectedID.value) && !pendingSelectedID.value) selectedID.value = null
    if (reset) {
      await loadFilteredTotal(current)
      if (disposed || current !== generation) return
    } else if (page.items.length) void nextTick(loadMoreIfNeeded)
    ensureSelection()
  } catch (e) {
    if (!disposed && current === generation) {
      if (reset) {
        error.value = errorMessage(e)
        // 查询失败时页面仍可能展示快照里的近期条目，保持“最新一条”选中。
        ensureSelection()
      } else {
        pageError.value = errorMessage(e)
      }
    }
  } finally {
    if (!disposed && current === generation) { loading.value = false; loadingMore.value = false }
  }
}

function retryPagination() {
  pageError.value = ''
  if (hasMore.value) void refresh(false)
  else void refresh(true)
}

function appliedNavigationToken(): string {
  try { return sessionStorage.getItem(NAVIGATION_TOKEN_KEY) ?? '' } catch { return '' }
}

watch([search, source, listID], () => {
  clearSelection()
  error.value = ''
  pageError.value = ''
  clearTimeout(timer)
  timer = setTimeout(() => void refresh(), search.value.trim() ? 120 : 0)
})
watch(() => props.snapshot?.engine_ready, (ready, old) => { if (ready && !old && featureEnabled.value) { void loadLibrary().then(() => { void loadSources(); return refresh() }) } })
watch(() => visibleItems.value.map((item) => item.id).join('|'), () => {
  if (mounted && !loading.value) ensureSelection()
})
watch(() => library.revision, () => { if (mounted && featureEnabled.value) void loadSources() })
watch(() => props.snapshot?.clipboard?.navigation, (navigation) => {
  const token = typeof navigation?.token === 'string' ? navigation.token : ''
  if (!token || token === navigationToken || token === appliedNavigationToken()) return
  navigationToken = token
  try { sessionStorage.setItem(NAVIGATION_TOKEN_KEY, token) } catch { /* ignore */ }
  pendingSelectedID.value = typeof navigation?.selected_id === 'string' && navigation.selected_id ? navigation.selected_id : null
  const nextQuery = typeof navigation?.query === 'string' ? navigation.query : ''
  const nextSource = typeof navigation?.source === 'string' && navigation.source ? navigation.source : null
  const nextList = typeof navigation?.list_id === 'string' && navigation.list_id ? navigation.list_id : null
  const changed = nextQuery !== search.value || nextSource !== source.value || nextList !== listID.value
  search.value = nextQuery
  source.value = nextSource
  listID.value = nextList
  if (!mounted) return
  // 筛选项没变时上面的 watcher 不会触发，这里补一次查询。
  if (!changed) {
    clearSelection()
    void refresh()
  }
}, { immediate: true })
watch(featureEnabled, (enabled) => {
  if (!enabled || !mounted || disposed) return
  void loadLibrary().then(async () => {
    if (disposed) return
    await refresh()
    startPolling()
    void loadSources()
  })
})

async function syncLibrary() {
  const before = library.revision
  await loadLibrary()
  if (!disposed && library.revision !== before) {
    await refresh()
    void loadSources()
  }
}

function startPolling() {
  if (!libraryTimer) libraryTimer = setInterval(() => { void syncLibrary() }, 30000)
  startDayWatcher()
}

function currentDayKey() {
  const now = new Date()
  return `${now.getFullYear()}-${now.getMonth()}-${now.getDate()}|${Intl.DateTimeFormat().resolvedOptions().timeZone}`
}

/** 跨日或时区变化时重算「今天/昨天」分组（原生 NSCalendarDayChanged / NSSystemTimeZoneDidChange）。 */
function startDayWatcher() {
  dayKey = currentDayKey()
  if (dayTimer) return
  dayTimer = setInterval(() => {
    const key = currentDayKey()
    if (key !== dayKey) {
      dayKey = key
      dayRevision.value += 1
      // 日期窗口（今天/昨天、智能列表）也要按新边界重新查询。
      void refresh()
    }
  }, 30000)
}

onMounted(async () => {
  mounted = true
  window.addEventListener('keydown', keydown)
  window.addEventListener('pointermove', onPointerMove)
  window.addEventListener('pointerup', onPointerUp)
  window.addEventListener('pointercancel', onPointerUp)
  window.addEventListener('selectstart', onSelectStart)
  if (historyPane.value && typeof ResizeObserver !== 'undefined') {
    historyObserver = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (entry) columnCount.value = entry.contentRect.width < 300 ? 1 : 2
    })
    historyObserver.observe(historyPane.value)
  }
  if (!featureEnabled.value) return
  startPolling()
  void loadSources()
  await loadLibrary()
  if (!disposed) await refresh()
})
onBeforeUnmount(() => {
  disposed = true
  ++generation
  clearTimeout(timer)
  if (libraryTimer) clearInterval(libraryTimer)
  if (dayTimer) clearInterval(dayTimer)
  historyObserver?.disconnect()
  window.removeEventListener('keydown', keydown)
  window.removeEventListener('pointermove', onPointerMove)
  window.removeEventListener('pointerup', onPointerUp)
  window.removeEventListener('pointercancel', onPointerUp)
  window.removeEventListener('selectstart', onSelectStart)
})

function setMode(value: 'grid' | 'list') {
  if (imageFilter.value) {
    imageMode.value = value
    try { localStorage.setItem(IMAGE_MODE_KEY, value) } catch { /* ignore */ }
  } else {
    textMode.value = value
    try { localStorage.setItem(TEXT_MODE_KEY, value) } catch { /* ignore */ }
  }
}

function clearSelection() {
  checked.value = new Set()
  selectedID.value = null
}

function clearSearch() {
  search.value = ''
  searchInput.value?.focus()
}

function clearFilters() {
  search.value = ''
  source.value = null
  listID.value = null
}

function select(item: LibraryItem, e?: MouseEvent) {
  const list = visibleItems.value
  const index = list.findIndex((row) => row.id === item.id)
  const anchor = list.findIndex((row) => row.id === selectedID.value)
  if (e?.shiftKey && anchor >= 0 && index >= 0) {
    const [start, end] = anchor < index ? [anchor, index] : [index, anchor]
    const range = list.slice(start, end + 1).map((row) => row.id)
    checked.value = e.metaKey || e.ctrlKey ? new Set([...checked.value, ...range]) : new Set(range)
    selectedID.value = item.id
    return
  }
  if (e?.metaKey || e?.ctrlKey) {
    const next = new Set(checked.value)
    if (!next.size && selectedID.value) next.add(selectedID.value)
    if (next.has(item.id)) next.delete(item.id); else next.add(item.id)
    checked.value = next
    selectedID.value = next.has(item.id) ? item.id : list.find((row) => next.has(row.id))?.id ?? null
    return
  }
  checked.value = new Set([item.id])
  selectedID.value = item.id
}

function selectItem(item: LibraryItem) {
  checked.value = new Set([item.id])
  selectedID.value = item.id
  void nextTick(revealSelected)
}

function revealSelected() {
  const id = selectedID.value
  if (!id) return
  body.value?.querySelector(`[data-id="${id}"]`)?.scrollIntoView({ block: 'nearest' })
}

function selectionItems(): LibraryItem[] {
  return visibleItems.value.filter((item) => selection.value.has(item.id))
}

function requestDelete(targets: LibraryItem[]) {
  const unique = targets.filter((item, index, self) => self.findIndex((row) => row.id === item.id) === index)
  if (!unique.length) return
  if (unique.length === 1) { void removeItems(unique); return }
  pendingDelete.value = unique
}

async function confirmPendingDelete() {
  const targets = pendingDelete.value
  pendingDelete.value = null
  if (targets) await removeItems(targets)
}

async function removeItems(targets: LibraryItem[]) {
  const before = visibleItems.value
  const deleted = new Set(targets.map((item) => item.id))
  const index = before.findIndex((item) => item.id === selectedID.value)
  const wasSelected = !!selectedID.value && deleted.has(selectedID.value)
  try {
    await deleteItems(targets)
    deletedIDs.value = new Set([...deletedIDs.value, ...deleted])
    clearSelection()
    await refresh()
    // 原生删除选中项后把选中落到列表第一条；这里优先保留原位置的“下一条”。
    if (wasSelected) {
      const order = [...before.slice(index + 1), ...before.slice(0, Math.max(index, 0)).reverse()]
      const next = order.find((item) => !deleted.has(item.id))
      const target = next ? visibleItems.value.find((item) => item.id === next.id) : null
      if (target) {
        selectedID.value = target.id
        checked.value = new Set()
      }
    }
  } catch (e) { error.value = errorMessage(e) }
}

async function copy(item: LibraryItem): Promise<boolean> {
  try { await copyItem(item.id); return true } catch (e) { error.value = errorMessage(e); return false }
}

async function saveTitle(title: string) {
  const item = titleItem.value
  titleItem.value = null
  if (!item) return
  try {
    await renameItem(item.id, title)
    item.title = title.trim() || null
    await refresh()
  } catch (e) { error.value = errorMessage(e) }
}

async function edit(item: LibraryItem) {
  try {
    const content = await loadContent(item.id)
    if (content.truncated) { error.value = '内容不完整，不能保存编辑。'; return }
    editText.value = content.text
    editRuns.value = content.richRuns ?? []
    editItem.value = item
  } catch (e) { error.value = errorMessage(e) }
}

async function saveText(text: string, runs: Array<{ text: string; bold?: boolean; italic?: boolean; underline?: boolean; strike?: boolean }>) {
  if (!editItem.value) return
  try {
    const request = { op: 'set_item_text', id: editItem.value.id, text, ...(runs.length ? { runs } : {}) }
    await backend.invoke('clipboard.updateRichText', request)
    editItem.value = null
    await refresh()
  } catch (e) { error.value = errorMessage(e) }
}

function openHeaderMenu(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  menu.value = {
    x: rect.left,
    y: rect.bottom + 2,
    items: [
      { label: '全选已加载', disabled: visibleItems.value.length === 0, run: () => { checked.value = new Set(visibleItems.value.map((item) => item.id)) } },
      {
        label: `删除所选（${selectionCount.value} 条）`, danger: true, disabled: selectionCount.value === 0,
        run: () => requestDelete(selectionItems()),
      },
    ],
  }
}

function openSourceMenu(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  menu.value = {
    x: rect.left,
    y: rect.bottom + 2,
    items: [
      { label: '所有设备', checked: source.value === null, run: () => { source.value = null } },
      ...sources.value.map((choice) => ({ label: choice.name, checked: source.value === choice.id, run: () => { source.value = choice.id } })),
    ],
  }
}

function openItemMenu(item: LibraryItem, e: MouseEvent) {
  e.preventDefault()
  if (!selection.value.has(item.id)) checked.value = new Set([item.id])
  selectedID.value = item.id
  menu.value = {
    x: e.clientX,
    y: e.clientY,
    items: itemMenu({
      item,
      title: item.title,
      meta: metaFor(item),
      copyEnabled: true,
      workflowsEnabled: props.snapshot?.workflows?.enabled !== false,
      onCopy: () => { void copy(item) },
      onEditTitle: () => { titleItem.value = item },
      onEditText: () => { void edit(item) },
      onDelete: () => requestDelete(selection.value.has(item.id) ? selectionItems() : [item]),
      fail: (message) => { error.value = message },
    }),
  }
}

function moveGrid(key: string) {
  const layout = masonryColumns(visibleItems.value, columnCount.value).map((column) => {
    let y = 0
    return column.map((item) => {
      const height = estimatedHeight(item)
      const center = y + height / 2
      y += height + 12
      return { item, center }
    })
  })
  let column = -1
  let row = -1
  for (let c = 0; c < layout.length; c++) {
    const index = layout[c].findIndex((entry) => entry.item.id === selectedID.value)
    if (index >= 0) { column = c; row = index; break }
  }
  if (column < 0) return
  const current = layout[column][row]
  if (key === 'ArrowLeft' || key === 'ArrowRight') {
    const nextColumn = layout[column + (key === 'ArrowRight' ? 1 : -1)]
    if (!nextColumn?.length) return
    const target = nextColumn.reduce((best, entry) =>
      Math.abs(entry.center - current.center) < Math.abs(best.center - current.center) ? entry : best)
    selectItem(target.item)
    return
  }
  const down = key === 'ArrowDown'
  const candidates = layout[column].filter((entry) => down ? entry.center > current.center : entry.center < current.center)
  if (!candidates.length) return
  const target = candidates.reduce((best, entry) =>
    Math.abs(entry.center - current.center) < Math.abs(best.center - current.center) ? entry : best)
  selectItem(target.item)
}

function moveList(key: string) {
  const list = visibleItems.value
  if (!list.length) return
  const index = list.findIndex((item) => item.id === selectedID.value)
  const next = Math.max(0, Math.min(list.length - 1, (index < 0 ? 0 : index) + (key === 'ArrowDown' ? 1 : -1)))
  selectItem(list[next])
}

function keydown(e: KeyboardEvent) {
  if (e.defaultPrevented) return
  if ((e.target as HTMLElement).matches?.('input,textarea,select,[contenteditable]')) return
  if (titleItem.value || editItem.value || pendingDelete.value) return
  if (menu.value) {
    if (e.key === 'Escape') { e.preventDefault(); menu.value = null }
    return
  }
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'f') { e.preventDefault(); searchInput.value?.focus(); return }
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a') {
    e.preventDefault()
    if (visibleItems.value.length) checked.value = new Set(visibleItems.value.map((item) => item.id))
    return
  }
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'c') {
    // 预览里的可选文字优先走浏览器原生复制。
    if (window.getSelection()?.toString()) return
    const item = selected.value
    if (item) { e.preventDefault(); void copy(item) }
    return
  }
  if (e.key === 'Escape') { clearSelection(); return }
  if (e.key === 'Backspace' || e.key === 'Delete') {
    if (selectionCount.value) { e.preventDefault(); requestDelete(selectionItems()) }
    return
  }
  if (e.key === 'ArrowUp' || e.key === 'ArrowDown' || e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
    e.preventDefault()
    if (mode.value === 'grid') moveGrid(e.key)
    else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') moveList(e.key)
  }
}

function loadMoreIfNeeded() {
  const el = body.value
  if (!el || !hasMore.value || loading.value || loadingMore.value || pageError.value) return
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 240) void refresh(false)
}

function onScroll() { loadMoreIfNeeded() }

function contentPoint(e: PointerEvent) {
  const el = body.value
  if (!el) return { x: 0, y: 0 }
  const rect = el.getBoundingClientRect()
  return { x: e.clientX - rect.left + el.scrollLeft, y: e.clientY - rect.top + el.scrollTop }
}

/** 拖拽框选：从条目外按下，拖动命中哪些记录就并选哪些（原生 ClipboardHistoryPane.interact）。 */
function onBodyPointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  const target = e.target as HTMLElement | null
  if (!target || target.closest('.cell, .row-wrap, [data-id], button, input, textarea, select')) return
  const el = body.value
  if (!el) return
  dragOrigin = contentPoint(e)
  dragBase = e.metaKey || e.ctrlKey || e.shiftKey ? new Set(selection.value) : new Set()
  const base = el.getBoundingClientRect()
  dragFrames = Array.from(el.querySelectorAll<HTMLElement>('[data-id]')).map((node) => {
    const rect = node.getBoundingClientRect()
    return {
      id: node.dataset.id ?? '',
      left: rect.left - base.left + el.scrollLeft,
      top: rect.top - base.top + el.scrollTop,
      right: rect.right - base.left + el.scrollLeft,
      bottom: rect.bottom - base.top + el.scrollTop,
    }
  })
  marqueeActive = false
}

function onPointerMove(e: PointerEvent) {
  if (!dragOrigin) return
  const point = contentPoint(e)
  const dx = point.x - dragOrigin.x
  const dy = point.y - dragOrigin.y
  if (!marqueeActive && Math.hypot(dx, dy) < 4) return
  marqueeActive = true
  const left = Math.min(dragOrigin.x, point.x)
  const top = Math.min(dragOrigin.y, point.y)
  const width = Math.abs(dx)
  const height = Math.abs(dy)
  marqueeRect.value = { left, top, width, height }
  const next = new Set(dragBase)
  for (const frame of dragFrames) {
    if (frame.left < left + width && frame.right > left && frame.top < top + height && frame.bottom > top) next.add(frame.id)
  }
  checked.value = next
  selectedID.value = visibleItems.value.find((item) => next.has(item.id))?.id ?? null
}

function onPointerUp() {
  if (!dragOrigin) return
  dragOrigin = null
  dragFrames = []
  dragBase = new Set()
  marqueeActive = false
  marqueeRect.value = null
}

function onSelectStart(e: Event) {
  if (dragOrigin) e.preventDefault()
}

function startsGroup(index: number) {
  void dayRevision.value
  return startsDayGroup(visibleItems.value, index)
}

function groupTitle(item: LibraryItem) {
  void dayRevision.value
  return dayTitle(item)
}
</script>

<template>
  <FeatureGate v-if="snapshot && snapshot.features?.clipboard === false" page="clipboard" title="剪贴板" />
  <div v-else class="clipboard-page" :class="mode === 'grid' ? 'grid-mode' : 'list-mode'">
    <WorkspaceSidebar v-model:selected-list-id="listID" @error="error=$event" @open-workflows="emit('navigate','settings:workflows')" />
    <main ref="historyPane" class="history">
      <header class="pane-head">
        <div class="title-row">
          <h2>历史记录</h2>
          <span class="grow" />
          <span v-if="headerTotal !== null" class="total">{{ headerTotal }} 条</span>
          <button type="button" class="head-button" aria-label="历史记录操作" title="历史记录操作" @click="openHeaderMenu">
            <Icon name="ellipsis" :size="14" />
          </button>
        </div>
        <div class="search" :class="{ focused: searchFocused }">
          <Icon name="magnifyingglass" :size="13" class="search-icon" />
          <input
            ref="searchInput" v-model="search" type="text" placeholder="搜索剪贴板" aria-label="搜索剪贴板"
            @focus="searchFocused = true" @blur="searchFocused = false"
            @keydown.escape.stop.prevent="search = ''; searchInput?.blur()"
          />
          <button v-if="search" type="button" class="clear" aria-label="清除搜索" title="清除搜索" @click="clearSearch">
            <Icon name="xmarkFill" :size="13" />
          </button>
          <span v-else class="hint">⌘ F</span>
        </div>
        <div class="filters">
          <button type="button" class="source" aria-label="来源设备" title="按来源设备筛选历史" @click="openSourceMenu">
            <Icon name="laptop" :size="13" />
            <span>{{ sourceName }}</span>
            <Icon name="chevronDown" :size="10" />
          </button>
          <div class="mode-switch" role="group" aria-label="显示方式">
            <button type="button" :class="{ on: mode === 'list' }" :aria-pressed="mode === 'list'" aria-label="列表" title="列表" @click="setMode('list')">
              <Icon name="list" :size="15" />
            </button>
            <button type="button" :class="{ on: mode === 'grid' }" :aria-pressed="mode === 'grid'" aria-label="瀑布流" title="瀑布流" @click="setMode('grid')">
              <Icon name="grid" :size="15" />
            </button>
          </div>
        </div>
        <div v-if="selectionCount > 1" class="selection-bar">
          <span>已选 {{ selectionCount }} 条</span>
          <button type="button" @click="requestDelete(selectionItems())">删除所选</button>
        </div>
      </header>

      <div
        ref="body" class="pane-body" role="listbox" aria-label="剪贴板历史"
        aria-multiselectable="true" @scroll="onScroll" @pointerdown="onBodyPointerDown"
      >
        <div v-if="loading && !visibleItems.length" class="pane-empty">
          <span class="spinner" />
          <strong>正在搜索…</strong>
        </div>
        <div v-else-if="!visibleItems.length && error" class="pane-empty">
          <Icon name="warning" :size="24" class="empty-glyph" />
          <strong>加载未完成</strong>
          <p>{{ error }}</p>
          <button type="button" class="button" @click="refresh()">重试</button>
        </div>
        <div v-else-if="!visibleItems.length" class="pane-empty">
          <Icon :name="hasFilters ? 'magnifyingglass' : 'clipboard'" :size="24" class="empty-glyph" />
          <strong>{{ hasFilters ? '没有匹配内容' : '还没有剪贴板记录' }}</strong>
          <p>{{ hasFilters ? '试试其他关键词，或清除筛选' : '复制文字、图片或文件后，会显示在这里' }}</p>
          <button v-if="hasFilters" type="button" class="button" @click="clearFilters">清除筛选</button>
        </div>
        <div v-else-if="mode === 'grid'" class="grid results">
          <div v-for="(column, index) in columns" :key="index" class="column">
            <div v-for="item in column" :key="item.id" class="cell" @click="select(item, $event)" @contextmenu="openItemMenu(item, $event)">
              <HistoryCard
                :item="item" :meta="metaFor(item)" :selected="selection.has(item.id)" :ocr-hit="ocrHit(item)"
                :copy-enabled="true" :copy="() => copy(item)"
                @edit-title="titleItem = item"
                @delete="requestDelete(selection.has(item.id) ? selectionItems() : [item])"
              />
            </div>
          </div>
        </div>
        <div v-else class="list results">
          <template v-for="(item, index) in visibleItems" :key="item.id">
            <div v-if="startsGroup(index)" class="day" :class="{ first: index === 0 }">{{ groupTitle(item) }}</div>
            <div class="row-wrap" @click="select(item, $event)" @contextmenu="openItemMenu(item, $event)">
              <HistoryRow
                :item="item" :meta="metaFor(item)" :selected="selection.has(item.id)"
                :focused="selectedID === item.id" :ocr-hit="ocrHit(item)"
              />
            </div>
          </template>
        </div>
        <div v-if="pageError" class="load-error">
          <span>加载未完成</span>
          <button type="button" class="button" @click="retryPagination">重试</button>
        </div>
        <div v-if="loadingMore" class="loading-more"><span class="spinner" />正在读取…</div>
        <div
          v-if="marqueeRect" class="marquee" aria-hidden="true"
          :style="{ left: marqueeRect.left + 'px', top: marqueeRect.top + 'px', width: marqueeRect.width + 'px', height: marqueeRect.height + 'px' }"
        />
      </div>
    </main>

    <PreviewPane
      :item="selected" :meta="selected ? metaFor(selected) : undefined" :notice="notice"
      :history-empty="historyEmpty" :workflow-runs="workflowRuns" :workflow-error="workflowError"
      :workflows-enabled="snapshot?.workflows?.enabled !== false"
      @edit="selected && edit(selected)" @error="error = $event"
    />
    <TitleEditorDialog v-if="titleItem" :title="titleItem.title" @save="saveTitle" @close="titleItem = null" />
    <ContentEditorDialog
      v-if="editItem" :text="editText" :runs="editRuns" :rtf-available="editItem.mime.includes('rtf')"
      @save="saveText" @close="editItem = null"
    />
    <ConfirmDialog
      v-if="pendingDelete" :title="`删除选中的 ${pendingDelete.length} 条内容？`"
      message="将删除这些内容的历史记录及所有手动列表中的归档，不只是从当前列表移除。此操作不可撤销。"
      :confirm-label="`删除 ${pendingDelete.length} 条内容`"
      @confirm="confirmPendingDelete" @cancel="pendingDelete = null"
    />
    <ContextMenu v-if="menu" v-bind="menu" @close="menu = null" />
  </div>
</template>

<style scoped>
.clipboard-page {
  height: 100%; min-height: 0; display: grid;
  grid-template-columns: minmax(164px, 196px) minmax(240px, 330px) minmax(340px, 1fr);
  background: var(--canvas); color: var(--text); outline: none;
}
.clipboard-page.grid-mode {
  grid-template-columns: minmax(164px, 196px) minmax(240px, 360px) minmax(340px, 1fr);
}
.history {
  min-width: 0; min-height: 0; display: flex; flex-direction: column; overflow: hidden;
  background: var(--surface); border-left: 0.5px solid var(--line);
}
.pane-head { flex: none; display: flex; flex-direction: column; gap: 12px; padding: 20px 14px 12px; }
.title-row { display: flex; align-items: center; gap: 8px; min-height: 28px; }
.title-row h2 { margin: 0; font-size: 14px; font-weight: 600; line-height: 1.3; }
.title-row .grow { flex: 1; }
.total { font-size: 11px; font-variant-numeric: tabular-nums; color: var(--secondary); white-space: nowrap; }
.head-button {
  width: 28px; height: 28px; flex: none; display: inline-flex; align-items: center; justify-content: center;
  border-radius: 8px; background: var(--control); color: var(--text);
}
.head-button:hover { background: color-mix(in srgb, var(--text) 9%, transparent); }
.search {
  display: flex; align-items: center; gap: 8px; height: 40px; padding: 0 12px;
  border-radius: 8px; background: var(--control);
}
.search.focused { box-shadow: inset 0 0 0 2px var(--accent); }
.search-icon { color: var(--secondary); }
.search input {
  flex: 1; min-width: 0; min-height: 24px; padding: 0; border: 0; background: transparent;
  font-size: 13px; color: var(--text); outline: none;
}
.search input::placeholder { color: var(--secondary); opacity: 0.9; }
.hint { font-size: 11px; color: var(--secondary); opacity: 0.6; white-space: nowrap; }
.clear { display: inline-flex; align-items: center; justify-content: center; padding: 0; color: var(--secondary); }
.clear:hover { color: var(--text); }
.filters { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.source {
  display: inline-flex; align-items: center; gap: 5px; min-width: 0; height: 30px; padding: 0;
  font-size: 12px; color: var(--accent);
}
.source span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mode-switch {
  flex: none; display: flex; align-items: center; gap: 2px; padding: 2px;
  border-radius: 8px; background: var(--control);
}
.mode-switch button {
  width: 34px; height: 30px; display: inline-flex; align-items: center; justify-content: center;
  border-radius: 8px; color: var(--secondary);
}
.mode-switch button:hover { color: var(--text); }
.mode-switch button.on { background: var(--surface); color: var(--text); }
.selection-bar { display: flex; align-items: center; gap: 8px; font-size: 11px; color: var(--secondary); }
.selection-bar button { margin-left: auto; font-size: 11px; color: var(--danger); }
.selection-bar button:hover { text-decoration: underline; }
.pane-body { position: relative; flex: 1; min-height: 0; overflow: auto; scrollbar-width: none; }
.pane-body::-webkit-scrollbar { display: none; }
.grid { display: flex; align-items: flex-start; gap: 12px; padding: 8px 12px; }
.column { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 12px; }
.cell { min-width: 0; }
.list { display: flex; flex-direction: column; padding-bottom: 8px; }
.day { margin: 14px 10px 2px; font-size: 13px; font-weight: 600; color: var(--secondary); }
.day.first { margin-top: 4px; }
.row-wrap { padding: 2px 10px; border-bottom: 0.5px solid var(--line); }
.row-wrap:last-child { border-bottom: 0; }
.pane-empty {
  display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 45px 15px;
  text-align: center; color: var(--secondary);
}
.pane-empty .empty-glyph { color: var(--secondary); opacity: 0.75; margin-bottom: 2px; }
.pane-empty strong { font-size: 13px; font-weight: 600; color: var(--text); }
.pane-empty p { margin: 0; font-size: 12px; line-height: 1.5; max-width: 280px; }
.pane-empty .button { margin-top: 2px; font-size: 12px; min-height: 28px; }
.load-error {
  display: flex; align-items: center; justify-content: center; gap: 10px; padding: 14px;
  font-size: 12px; color: var(--warning);
}
.load-error .button { min-height: 26px; padding: 0 10px; font-size: 12px; }
.loading-more { display: flex; align-items: center; justify-content: center; gap: 8px; padding: 18px; font-size: 12px; color: var(--secondary); }
.marquee {
  position: absolute; z-index: 2; pointer-events: none;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent) 80%, transparent);
}
.spinner {
  width: 14px; height: 14px; border-radius: 50%; flex: none;
  border: 2px solid color-mix(in srgb, var(--secondary) 30%, transparent);
  border-top-color: var(--secondary); animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
</style>
