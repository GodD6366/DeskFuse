// 剪贴板库的线上格式，字段与 ClipboardLibraryStore.swift 的 Codable 结构一致。
// 引擎的 u64 ID 在前端一律是 string，避免超过 2^53 后丢精度。

export type ClipboardId = string

export type DisplayMode = 'list' | 'grid'

export interface LibraryRule {
  /** 仅编辑器使用，不持久化。 */
  key: string
  field: string
  op: string
  value: string
  value2: string
}

export interface LibraryList {
  id: string
  name: string
  kind: string
  match_mode: string
  position: number
  rules: LibraryRule[]
}

export interface LibraryChoice {
  id: string
  name: string
}

export interface LibraryApp {
  bundle_id: string
  name: string
}

export interface LibraryState {
  revision: number
  counts_revision?: number
  lists?: LibraryList[]
  devices?: LibraryChoice[]
  apps?: LibraryApp[]
}

export interface LibraryCursor {
  sort_at_ms: string | number
  id: string | number
}

export interface LibraryItem {
  id: ClipboardId
  related_ids: ClipboardId[]
  copied_at_ms: number | null
  mime: string
  length: number
  preview: string
  source: string
  has_image: boolean
  file_count: number
  memberships: string[]
  source_device_id: string | null
  source_app_id: string | null
  source_app_name: string | null
  title: string | null
  ocr_text: string | null
  ocr_version: number
  remote: boolean
  available: boolean
  sort_at_ms: number | null
}

export interface LibraryPage {
  items: LibraryItem[]
  total: number
  total_exact: boolean
  next_offset: number | null
  next_cursor: LibraryCursor | null
  revision: number | null
}

/** backend.content(id) 归一化后的完整内容。 */
export interface ClipboardContent {
  id: ClipboardId
  text: string
  truncated: boolean
  /** deskfuse://blob/... 之类的图片地址，前端不接触 Base64；没有图片时为空。 */
  imageSrc: string
  filePaths: string[]
  /** 文件不在本地时，宿主是否还能从来源设备取回。 */
  canRequestFiles: boolean
  /** 条目带有富文本（RTF）；字节不下发到前端。 */
  rtfAvailable: boolean
  html: string
  richRuns: Array<{text:string;bold?:boolean;italic?:boolean;underline?:boolean;strike?:boolean}>
  /** 宿主确认在本地存在的文件路径；宿主未提供时为 null，视为 filePaths 全部存在。 */
  localPaths: string[] | null
}

/** 历史行与卡片共用的展示元数据。 */
export interface ItemMeta {
  remote: boolean
  available: boolean
  originDevice: string | null
  ocrText: string | null
}

export interface PeerTarget {
  id: string
  name: string
  connected: boolean
}

export interface WorkflowTarget {
  id: string
  name: string
}

export type CardKind = 'image' | 'file' | 'link' | 'code' | 'text'
