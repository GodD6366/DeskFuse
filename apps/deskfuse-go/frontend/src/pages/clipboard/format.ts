// 展示用的纯函数：标题、类型、时间、字节数、代码识别。
// 逻辑一一对应 ClipboardHistoryPane.swift / ClipboardMasonryGrid.swift / ClipboardCodePreview.swift。

import type {
  CardKind, ClipboardContent, LibraryApp, LibraryChoice, LibraryCursor, LibraryItem,
  LibraryList, LibraryPage, LibraryRule, LibraryState,
} from '../../types/clipboard'

type Raw = Record<string, any>

export function newKey(): string {
  return globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

// MARK: 线上格式归一化

const str = (value: unknown, fallback = ''): string => (typeof value === 'string' ? value : fallback)
const num = (value: unknown, fallback = 0): number => {
  const parsed = typeof value === 'string' ? Number(value) : value
  return typeof parsed === 'number' && Number.isFinite(parsed) ? parsed : fallback
}
const optStr = (value: unknown): string | null => (typeof value === 'string' && value !== '' ? value : null)
const idOf = (value: unknown): string => (value === null || value === undefined ? '' : String(value))

export function normalizeRule(raw: Raw): LibraryRule {
  return { key: newKey(), field: str(raw.field, 'text'), op: str(raw.op, 'contains'), value: str(raw.value), value2: str(raw.value2) }
}

export function normalizeList(raw: Raw): LibraryList {
  return {
    id: idOf(raw.id),
    name: str(raw.name),
    kind: str(raw.kind, 'manual'),
    match_mode: str(raw.match_mode, 'all'),
    position: num(raw.position),
    rules: Array.isArray(raw.rules) ? raw.rules.map(normalizeRule) : [],
  }
}

export function serializeList(list: LibraryList): Record<string, unknown> {
  return {
    id: list.id,
    name: list.name,
    kind: list.kind,
    match_mode: list.match_mode,
    position: list.position,
    rules: list.rules.map(({ field, op, value, value2 }) => ({ field, op, value, value2 })),
  }
}

export function normalizeState(raw: Raw): LibraryState {
  const state: LibraryState = { revision: num(raw.revision) }
  if (raw.counts_revision !== undefined) state.counts_revision = num(raw.counts_revision)
  if (Array.isArray(raw.lists)) state.lists = raw.lists.map(normalizeList)
  if (Array.isArray(raw.devices)) state.devices = raw.devices.map((d: Raw): LibraryChoice => ({ id: idOf(d.id), name: str(d.name) }))
  if (Array.isArray(raw.apps)) state.apps = raw.apps.map((a: Raw): LibraryApp => ({ bundle_id: str(a.bundle_id), name: str(a.name) }))
  return state
}

export function normalizeItem(raw: Raw): LibraryItem {
  const id = idOf(raw.id)
  return {
    id,
    related_ids: Array.isArray(raw.related_ids) ? raw.related_ids.map(idOf) : [id],
    copied_at_ms: raw.copied_at_ms === null || raw.copied_at_ms === undefined ? null : num(raw.copied_at_ms),
    mime: str(raw.mime),
    length: num(raw.length),
    preview: str(raw.preview),
    source: str(raw.source),
    has_image: Boolean(raw.has_image),
    file_count: num(raw.file_count),
    memberships: Array.isArray(raw.memberships) ? raw.memberships.map(idOf) : [],
    source_device_id: optStr(raw.source_device_id),
    source_app_id: optStr(raw.source_app_id),
    source_app_name: optStr(raw.source_app_name),
    title: optStr(raw.title),
    ocr_text: optStr(raw.ocr_text),
    ocr_version: num(raw.ocr_version),
    remote: Boolean(raw.remote),
    available: raw.available === undefined || raw.available === null ? true : Boolean(raw.available),
    sort_at_ms: raw.sort_at_ms === undefined || raw.sort_at_ms === null ? null : num(raw.sort_at_ms),
  }
}

export function normalizePage(raw: Raw): LibraryPage {
  const cursor = raw.next_cursor as LibraryCursor | null | undefined
  return {
    items: Array.isArray(raw.items) ? raw.items.map(normalizeItem) : [],
    total: num(raw.total),
    total_exact: raw.total_exact !== false,
    next_offset: raw.next_offset === null || raw.next_offset === undefined ? null : num(raw.next_offset),
    next_cursor: cursor ?? null,
    revision: raw.revision === undefined || raw.revision === null ? null : num(raw.revision),
  }
}

/** backend.content：text、truncated、file_paths、image_url、rtf_available；字段名同时接受 camelCase。 */
export function normalizeContent(raw: Raw, fallbackId: string): ClipboardContent {
  const paths = raw.file_paths ?? raw.filePaths
  const local = raw.local_paths ?? raw.localPaths
  return {
    id: idOf(raw.id ?? fallbackId),
    text: str(raw.text),
    truncated: Boolean(raw.truncated),
    imageSrc: str(raw.image_url ?? raw.imageUrl),
    filePaths: Array.isArray(paths) ? paths.map(String) : [],
    canRequestFiles: Boolean(raw.can_request_files ?? raw.canRequestFiles),
    rtfAvailable: Boolean(raw.rtf_available ?? raw.rtfAvailable),
    html: str(raw.html),
    richRuns: Array.isArray(raw.rich_runs ?? raw.richRuns) ? (raw.rich_runs ?? raw.richRuns) : [],
    localPaths: Array.isArray(local) ? local.map(String) : null,
  }
}

// MARK: 文本与格式

/** ByteCountFormatter(.file) 的近似：十进制，字节 / KB / MB / GB。 */
export function byteCount(bytes: number): string {
  if (bytes < 1000) return `${bytes} 字节`
  const units: [string, number, number][] = [['KB', 1e3, 0], ['MB', 1e6, 1], ['GB', 1e9, 2], ['TB', 1e12, 2]]
  let chosen = units[0]
  for (const unit of units) if (bytes >= unit[1]) chosen = unit
  const value = bytes / chosen[1]
  const text = value.toFixed(chosen[2]).replace(/\.0+$/, '').replace(/(\.\d*?)0+$/, '$1')
  return `${text} ${chosen[0]}`
}

export function linkURL(text: string): URL | null {
  if (/\s/.test(text)) return null
  try {
    const url = new URL(text)
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) return null
    return url
  } catch {
    return null
  }
}

export function clipboardTitle(item: LibraryItem): string {
  const title = item.preview.trim()
  if (item.has_image && ['', '图像', '图片'].includes(title)) return '剪贴板图片'
  const url = linkURL(title)
  if (url) {
    let path = url.pathname
    try { path = decodeURIComponent(path) } catch { /* 保留原样 */ }
    return (url.hostname + (path === '/' ? '' : path)).slice(0, 160)
  }
  if (title) return Array.from(title).slice(0, 160).join('')
  if (item.file_count > 0) return `${item.file_count} 个文件`
  return item.has_image ? '剪贴板图片' : '剪贴板文字'
}

export function displayTitle(item: LibraryItem): string {
  const custom = (item.title ?? '').trim()
  return custom || clipboardTitle(item)
}

/** 与原生 ClipboardPreviewPolicy 相同：图片总是可预览，文字只在列表里被截断时预览。 */
export function shouldPreview(item: LibraryItem, mode: 'list' | 'grid'): boolean {
  if (item.file_count > 0) return false
  if (item.has_image) return true
  return textIsTruncated(item.preview, mode)
}

export function textIsTruncated(preview: string, mode: 'list' | 'grid'): boolean {
  const text = preview.trim()
  if (!text) return false
  const length = Array.from(preview).length
  if (length >= 160) return true
  const lines = preview.split('\n').length
  return mode === 'list' ? lines > 1 || length > 28 : lines > 8
}

export function recordSymbol(item: LibraryItem): 'files' | 'image' | 'text' {
  return item.file_count > 0 ? 'files' : item.has_image ? 'image' : 'text'
}

export function looksLikeCode(text: string): boolean {
  const trimmed = text.trim()
  if (Array.from(trimmed).length < 24) return false
  const lines = trimmed.split('\n')
  if (lines.length < 2) return false
  if ((trimmed.startsWith('{') || trimmed.startsWith('[')) && trimmed.includes(':')) return true
  const first = (lines[0] ?? '').trim()
  const keywords = ['func ', 'def ', 'import ', 'class ', 'struct ', 'enum ', 'return ', 'const ',
    'let ', 'var ', 'function ', '#include', 'public ', 'private ', 'if let ', 'for (', 'await ']
  if (keywords.some((k) => first.startsWith(k))) return true
  const keyValue = lines.filter((l) => { const t = l.trim(); return t.startsWith('"') && t.includes('":') }).length
  if (keyValue >= 2) return true
  const indented = lines.filter((l) => l.startsWith('    ') || l.startsWith('\t')).length
  return indented >= 2 && trimmed.includes('=')
}

export function cardKind(item: LibraryItem): CardKind {
  if (item.has_image) return 'image'
  if (item.file_count > 0) return 'file'
  const text = item.preview.trim()
  if (linkURL(text)) return 'link'
  if (looksLikeCode(text)) return 'code'
  return 'text'
}

export function linkParts(item: LibraryItem): { host: string; path: string } | null {
  const url = linkURL(item.preview.trim())
  if (!url) return null
  let path = url.pathname
  try { path = decodeURIComponent(path) } catch { /* 保留原样 */ }
  return { host: url.hostname, path: path === '/' ? '' : path }
}

// MARK: 时间

const pad = (n: number) => String(n).padStart(2, '0')

export function timeText(ms: number | null): string {
  if (ms === null) return ''
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function fullDate(ms: number): string {
  return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(ms))
}

export function fullDateSeconds(ms: number): string {
  return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'medium' }).format(new Date(ms))
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
}

export function startsDayGroup(items: LibraryItem[], index: number): boolean {
  if (index === 0) return true
  const previous = items[index - 1].copied_at_ms
  const current = items[index].copied_at_ms
  if (previous === null || current === null) return previous === null && current !== null
  return !sameDay(new Date(previous), new Date(current))
}

export function dayTitle(item: LibraryItem): string {
  if (item.copied_at_ms === null) return '其他'
  const date = new Date(item.copied_at_ms)
  const now = new Date()
  if (sameDay(date, now)) return '今天'
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
  if (sameDay(date, yesterday)) return '昨天'
  return `${date.getMonth() + 1}月${date.getDate()}日`
}

// MARK: 预览元数据

const fileTypeNames: Record<string, string> = {
  pdf: 'PDF 文稿', png: 'PNG 图像', jpg: 'JPEG 图像', jpeg: 'JPEG 图像', gif: 'GIF 图像', webp: 'WebP 图像',
  heic: 'HEIC 图像', txt: '纯文本文稿', md: 'Markdown 文稿', json: 'JSON 文稿', zip: 'ZIP 归档',
  mp4: 'MPEG-4 影片', mov: 'QuickTime 影片', mp3: 'MP3 音频', wav: 'WAV 音频', doc: 'Word 文稿', docx: 'Word 文稿',
  xls: 'Excel 表格', xlsx: 'Excel 表格', ppt: 'PowerPoint 演示文稿', pptx: 'PowerPoint 演示文稿', csv: 'CSV 文稿',
}

const mimeTypeNames: Record<string, string> = {
  'text/plain': '纯文本文稿', 'text/html': 'HTML 文稿', 'text/rtf': '富文本格式 (RTF)', 'text/uri-list': 'URL',
  'image/png': 'PNG 图像', 'image/jpeg': 'JPEG 图像', 'image/gif': 'GIF 图像', 'image/tiff': 'TIFF 图像',
  'image/heic': 'HEIC 图像', 'image/webp': 'WebP 图像',
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

export function formatName(item: LibraryItem, content: ClipboardContent): string {
  if (content.filePaths.length === 1) {
    const ext = baseName(content.filePaths[0]).split('.').pop()?.toLowerCase() ?? ''
    if (ext && fileTypeNames[ext]) return fileTypeNames[ext]
    return '文件'
  }
  if (item.file_count > 0) return `${item.file_count} 个文件`
  const byMime = mimeTypeNames[item.mime.toLowerCase()]
  if (byMime) return byMime
  return item.has_image ? '图片' : '文字'
}

// MARK: 代码预览

export interface CodeDocument { language: string; text: string }

export function detectCode(source: string): CodeDocument | null {
  const sample = source.slice(0, 50000).trim()
  if (sample.startsWith('```')) {
    const newline = source.indexOf('\n')
    const closing = source.lastIndexOf('```')
    if (newline >= 0 && closing > newline) {
      const label = source.slice(0, newline).trim().replace(/```/g, '')
      return { language: label ? label.toUpperCase() : '代码', text: source.slice(newline + 1, closing) }
    }
  }
  if (sample.startsWith('{') || sample.startsWith('[')) {
    if (source.length <= 200000) {
      try { return { language: 'JSON', text: JSON.stringify(JSON.parse(source), null, 2) } } catch { /* 继续启发式 */ }
    }
    if (/[{,]\s*"[^"\n]+"\s*:/.test(sample)) return { language: 'JSON', text: source }
  }
  const signatures: [string, RegExp][] = [
    ['Python', /^\s*(?:def\s+\w+\([^\n]*\)\s*(?:->[^\n]+)?:|from\s+[\w.]+\s+import\s+\w+|class\s+\w+(?:\([^\n]*\))?:)/m],
    ['Swift', /^\s*(?:func\s+\w+\(|(?:struct|enum|class)\s+\w+[^\n]*\{|import\s+(?:SwiftUI|AppKit|Foundation)\b)/m],
    ['JavaScript / TypeScript', /^\s*(?:(?:export\s+)?(?:async\s+)?function\s+\w+\(|(?:const|let|var)\s+\w+\s*=|import\s+.+\s+from\s+['"])/m],
    ['Rust', /^\s*(?:(?:pub\s+)?(?:async\s+)?fn\s+\w+\(|use\s+\w+::|impl\s+\w+[^\n]*\{)/m],
    ['SQL', /^\s*(?:SELECT\s+[\s\S]+\s+FROM\s+|INSERT\s+INTO\s+|CREATE\s+TABLE\s+|UPDATE\s+\w+\s+SET\s+)/i],
    ['HTML / XML', /^\s*(?:<!DOCTYPE\s+html|<\?xml|<(?:html|div|span|body|svg)\b[^>]*>)/i],
    ['代码', /^\s*(?:#include\s*[<"]|(?:public|private|protected)\s+(?:static\s+)?(?:class|void|int|string)\s+\w+|#!\/(?:usr\/)?bin\/)/m],
  ]
  for (const [language, pattern] of signatures) if (pattern.test(sample)) return { language, text: source }
  return null
}

const escapeHTML = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

/** 返回已转义的 HTML；语法着色类名由 PreviewPane 的样式定义。 */
export function highlightCode(text: string): string {
  const sample = text.slice(0, 50000)
  const pattern = /("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')|(\/\/[^\n]*|\/\*[\s\S]*?\*\/|^\s*#[^\n]*)|\b(true|false|null|nil|None|True|False|const|let|var|func|fn|def|class|struct|enum|import|from|return|if|else|for|while|async|await|try|throw|pub|use|SELECT|FROM|WHERE|INSERT|INTO)\b|\b(\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b/gm
  let out = ''
  let last = 0
  for (const match of sample.matchAll(pattern)) {
    const start = match.index ?? 0
    out += escapeHTML(sample.slice(last, start))
    const cls = match[1] !== undefined ? 'tok-string' : match[2] !== undefined ? 'tok-comment' : match[3] !== undefined ? 'tok-keyword' : 'tok-number'
    out += `<span class="${cls}">${escapeHTML(match[0])}</span>`
    last = start + match[0].length
  }
  return out + escapeHTML(sample.slice(last)) + escapeHTML(text.slice(50000))
}

// MARK: 瀑布流

/** 卡片高度估计，与 ClipboardMasonryLayout.estimatedHeight 一致。 */
export function estimatedHeight(item: LibraryItem): number {
  if (item.has_image) return 128
  if (item.file_count > 0) return 96
  const lines = Math.min(4, Math.max(1, Math.floor(Array.from(item.preview).length / 36)))
  return 70 + lines * 18
}

/** 逐项放进当前最矮的一列，与 ClipboardMasonryLayout.blocks 一致。 */
export function masonryColumns(items: LibraryItem[], columnCount: number): LibraryItem[][] {
  const count = Math.max(1, columnCount)
  const columns: LibraryItem[][] = Array.from({ length: count }, () => [])
  const heights = new Array<number>(count).fill(0)
  for (const item of items) {
    let index = 0
    for (let i = 1; i < count; i++) if (heights[i] < heights[index]) index = i
    columns[index].push(item)
    heights[index] += estimatedHeight(item) + 12
  }
  return columns
}

export const defaultTypeGroupIDs = [
  '67f10b9a-9f3c-508d-8090-4192e3ea297f',
  '9bd6453f-f270-5d1e-b806-b7cab97ae197',
  '96479303-4816-53e6-b7b3-d2b67704d587',
]

export const isDefaultTypeGroup = (list: LibraryList) => defaultTypeGroupIDs.includes(list.id)

export const filtersOnlyImages = (list: LibraryList | undefined) =>
  !!list && list.kind === 'smart' && list.rules.length === 1
  && list.rules[0].field === 'kind' && list.rules[0].op === 'is' && list.rules[0].value === 'image'

export function listSymbol(list: LibraryList): 'photo' | 'folder' | 'text' | 'smart' | 'list' {
  switch (list.name) {
    case '图片': return 'photo'
    case '文件': return 'folder'
    case '纯文本': case '文字': return 'text'
    default: return list.kind === 'smart' ? 'smart' : 'list'
  }
}
