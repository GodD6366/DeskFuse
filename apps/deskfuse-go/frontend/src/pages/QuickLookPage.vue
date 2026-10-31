<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { backend } from '../services/backend'
import { Preview } from '../mygo'
import { loadContent, wantPayload } from './clipboard/library'
import type { ClipboardContent } from '../types/clipboard'
import { detectCode, highlightCode } from './clipboard/format'
import Icon from './clipboard/Icon.vue'

// 菜单栏面板的只读快速预览：由 Go 进程放进一个贴住选中行的浮窗里，
// 尺寸和内容规则与原生 ClipboardQuickLook 相同（图片等比缩放、纯文本按需换行、
// 代码按语言着色、文件列表一行）。
const id = ref<string>('')
const status = ref('正在加载预览…')
const content = ref<ClipboardContent | null>(null)
const image = ref('')
const imageBox = ref<{ width: number; height: number } | null>(null)
const failed = ref(false)
let generation = 0
let disposed = false

const CODE_WIDTH = 640
const CODE_HEIGHT = 480
const IMAGE_MAX_WIDTH = 696
const IMAGE_MAX_HEIGHT = 536
const TEXT_MIN_WIDTH = 360
const TEXT_MAX_WIDTH = 600
const TEXT_MIN_HEIGHT = 160
const TEXT_MAX_HEIGHT = 460

function setSize(width: number, height: number) {
  const root = document.documentElement
  root.style.width = `${Math.round(width)}px`
  root.style.height = `${Math.round(height)}px`
}

/** 与原生 preferredContentSize 的文本测量一致：先量单行宽度，再量换行后的高度。 */
function measureText(text: string): { width: number; height: number } {
  const sample = Array.from(text).slice(0, 4000).join('')
  const width = Math.min(TEXT_MAX_WIDTH, Math.max(TEXT_MIN_WIDTH, naturalWidth(sample) + 36))
  const height = Math.min(TEXT_MAX_HEIGHT, Math.max(TEXT_MIN_HEIGHT, wrappedHeight(sample, width - 36) * 1.2 + 40))
  return { width, height }
}

function naturalWidth(text: string): number {
  const span = document.createElement('span')
  span.style.cssText = 'position:absolute;left:-9999px;top:0;visibility:hidden;white-space:pre;font:600 15px -apple-system,BlinkMacSystemFont,sans-serif'
  span.textContent = text
  document.body.append(span)
  const width = span.getBoundingClientRect().width
  span.remove()
  return width
}

function wrappedHeight(text: string, width: number): number {
  const box = document.createElement('div')
  box.style.cssText = `position:absolute;left:-9999px;top:0;visibility:hidden;width:${Math.max(1, width)}px;font:600 15px -apple-system,BlinkMacSystemFont,sans-serif;line-height:1.4;white-space:pre-wrap;overflow-wrap:break-word`
  box.textContent = text
  document.body.append(box)
  const height = box.getBoundingClientRect().height
  box.remove()
  return height
}

function applyImageSize(natural: { width: number; height: number }) {
  const sourceWidth = Math.max(natural.width, 1)
  const sourceHeight = Math.max(natural.height, 1)
  const scale = Math.min(1, IMAGE_MAX_WIDTH / sourceWidth, IMAGE_MAX_HEIGHT / sourceHeight)
  imageBox.value = { width: Math.round(sourceWidth * scale), height: Math.round(sourceHeight * scale) }
  setSize(
    Math.min(720, Math.max(300, sourceWidth * scale + 24)),
    Math.min(560, Math.max(220, sourceHeight * scale + 24)),
  )
}

function loadImage(src: string) {
  const probe = new Image()
  probe.onload = () => {
    if (disposed) return
    applyImageSize({ width: probe.naturalWidth, height: probe.naturalHeight })
  }
  probe.onerror = () => { if (!disposed) setSize(300, 160) }
  probe.src = src
}

const codeDoc = computed(() => (content.value ? detectCode(content.value.text) : null))
const highlighted = computed(() => (codeDoc.value ? highlightCode(codeDoc.value.text) : ''))

let waitForRemote = false

async function fetchContent(itemID: string): Promise<ClipboardContent | null> {
  const attempts = waitForRemote ? 11 : 1
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const value = await loadContent(itemID)
      if (value.text || value.imageSrc || value.filePaths.length) return value
    } catch {
      // 远端索引条目还没有本地内容：向来源设备请求后再重试，和原生一致。
    }
    if (disposed || id.value !== itemID || attempt === attempts - 1) return null
    if (attempt === 0) void wantPayload(itemID).catch(() => undefined)
    await new Promise((resolve) => setTimeout(resolve, 400))
  }
  return null
}

async function load(itemID: string, wait = waitForRemote) {
  if (!itemID) return
  const request = ++generation
  waitForRemote = wait
  id.value = itemID
  content.value = null
  image.value = ''
  imageBox.value = null
  failed.value = false
  status.value = '正在加载预览…'
  setSize(360, 160)
  const value = await fetchContent(itemID)
  if (disposed || request !== generation) return
  if (!value) {
    failed.value = true
    status.value = '暂时无法预览此内容'
    setSize(360, 160)
    return
  }
  content.value = value
  if (value.imageSrc) {
    image.value = value.imageSrc
    loadImage(value.imageSrc)
    return
  }
  const doc = detectCode(value.text)
  if (doc) {
    setSize(CODE_WIDTH, CODE_HEIGHT)
    return
  }
  if (value.text) {
    const size = measureText(value.text)
    setSize(size.width, size.height)
    return
  }
  if (value.filePaths.length) {
    setSize(300, 116)
    return
  }
  failed.value = true
  status.value = '暂时无法预览此内容'
  setSize(360, 160)
}

// 系统 popover 可能持有键盘：把预览用的按键转回面板，保持空格关闭、
// 方向键换行与回车粘贴的行为。
function keydown(event: KeyboardEvent) {
  if (![' ', 'Escape', 'Enter', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(event.key)) return
  event.preventDefault()
  void Preview.key(event.key).catch(() => undefined)
}

function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

onMounted(() => {
  const query = new URLSearchParams(location.search)
  const initial = query.get('id') ?? ''
  ;(window as any).__deskfuseQuickLook = (next: string, wait = false) => { void load(next, wait) }
  window.addEventListener('keydown', keydown)
  if (initial) void load(initial, query.get('wait') === '1')
})

onBeforeUnmount(() => {
  disposed = true
  window.removeEventListener('keydown', keydown)
  delete (window as any).__deskfuseQuickLook
})
</script>

<template>
  <div class="quick-look">
    <p v-if="failed" class="status">{{ status }}</p>
    <img
      v-else-if="image" class="image" :src="image" alt="" draggable="false"
      :style="imageBox ? { width: imageBox.width + 'px', height: imageBox.height + 'px' } : undefined"
    />
    <div v-else-if="codeDoc" class="code">
      <div class="code-head"><Icon name="code" :size="12" />{{ codeDoc.language }}</div>
      <div class="code-body">
        <pre v-html="highlighted" />
      </div>
    </div>
    <div v-else-if="content?.text" class="text">
      <pre>{{ content.text }}</pre>
    </div>
    <div v-else-if="content?.filePaths.length" class="files">
      <Icon name="doc" :size="22" class="file-icon" />
      <div class="file-text">
        <div class="file-name">{{ content.filePaths.length === 1 ? fileName(content.filePaths[0]) : `${content.filePaths.length} 个文件` }}</div>
        <div v-if="content.filePaths.length > 1" class="file-more">{{ fileName(content.filePaths[0]) }}</div>
      </div>
    </div>
    <p v-else class="status">{{ status }}</p>
  </div>
</template>

<style>
html:root { background: var(--surface); }
html, body { margin: 0; height: 100%; overflow: hidden; background: var(--surface); }
#app { height: 100%; }
</style>

<style scoped>
.quick-look { width: 100%; height: 100%; overflow: hidden; background: var(--surface); }
.status { margin: 0; padding: 44px 18px; text-align: center; font-size: 13px; color: var(--secondary); }
.image { display: block; margin: 0 auto; padding: 12px; max-width: 100%; max-height: 100%; object-fit: contain; }
.code { height: 100%; display: flex; flex-direction: column; background: var(--surface); }
.code-head { display: flex; align-items: center; gap: 6px; padding: 10px 16px; font-size: 12px; font-weight: 500; color: var(--secondary); border-bottom: .5px solid var(--line); flex: none; }
.code-body { flex: 1; min-height: 0; overflow: auto; }
.code-body pre { margin: 0; padding: 18px; font: 12px/1.45 ui-monospace, 'SF Mono', Menlo, Consolas, monospace; white-space: pre; }
.code-body :deep(.tok-string) { color: var(--success); }
.code-body :deep(.tok-comment) { color: var(--secondary); }
.code-body :deep(.tok-keyword) { color: var(--accent); }
.code-body :deep(.tok-number) { color: var(--warning); }
.text { height: 100%; overflow: auto; padding: 14px 18px; }
.text pre { margin: 0; font: 600 15px/1.4 -apple-system, BlinkMacSystemFont, sans-serif; white-space: pre-wrap; overflow-wrap: break-word; color: var(--text); }
.files { display: flex; align-items: center; gap: 14px; padding: 18px; }
.file-icon { color: var(--secondary); }
.file-text { min-width: 0; display: flex; flex-direction: column; gap: 4px; }
.file-name { font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.file-more { font-size: 11px; color: var(--secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
