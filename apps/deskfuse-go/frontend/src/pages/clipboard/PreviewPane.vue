<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { ClipboardContent, ItemMeta, LibraryItem } from '../../types/clipboard'
import { errorMessage, library, loadContent, wantPayload } from './library'
import { baseName, byteCount, detectCode, formatName, fullDate, highlightCode } from './format'
import { addPathsToShelf, copyItem, revealItem, saveItem, stageItem } from './actions'
import { moreMenu } from './menus'
import type { MenuItem } from './menu'
import Icon from './Icon.vue'
import ContextMenu from './ContextMenu.vue'
import WorkflowRunStatus from './WorkflowRunStatus.vue'
import { useAppInfo } from './appInfo'
import type { WorkflowRun } from './workflowRuns'

const props = defineProps<{
  item: LibraryItem | null
  meta?: ItemMeta
  notice?: string
  historyEmpty?: boolean
  workflowRuns?: WorkflowRun[]
  workflowError?: string
  workflowsEnabled?: boolean
}>()
const emit = defineEmits<{ edit: []; error: [message: string] }>()

const content = ref<ClipboardContent | null>(null)
const loading = ref(false)
const loadError = ref('')
const actionError = ref('')
const copying = ref(false)
const copied = ref(false)
const staging = ref(false)
const savingImage = ref(false)
const savedImagePath = ref('')
const fillImage = ref(false)
const imageSize = ref<{ width: number; height: number } | null>(null)
const imageFailed = ref(false)
const more = ref<{ x: number; y: number; items: MenuItem[] } | null>(null)
const pane = ref<HTMLElement | null>(null)
const paneHeight = ref(0)

let generation = 0
let copiedTimer: number | undefined
let observer: ResizeObserver | undefined

onMounted(() => {
  if (!pane.value || typeof ResizeObserver === 'undefined') return
  observer = new ResizeObserver((entries) => {
    const entry = entries[0]
    if (entry) paneHeight.value = entry.contentRect.height
  })
  observer.observe(pane.value)
})
onBeforeUnmount(() => {
  observer?.disconnect()
  window.clearTimeout(copiedTimer)
})

watch(() => props.item?.id, (id) => { void start(id) }, { immediate: true })

async function start(id: string | null | undefined) {
  const current = ++generation
  content.value = null
  loadError.value = ''
  actionError.value = ''
  imageSize.value = null
  imageFailed.value = false
  copied.value = false
  copying.value = false
  staging.value = false
  savingImage.value = false
  fillImage.value = false
  more.value = null
  if (!id) { loading.value = false; return }
  loading.value = true
  const item = props.item
  try {
    const value = await loadContent(id)
    if (current !== generation) return
    content.value = value
  } catch (e) {
    if (current !== generation) return
    // 远端索引行还没有本地载荷：按原生行为请求并短暂轮询。
    if (item?.remote && item.available === false) {
      loadError.value = errorMessage(e)
      loading.value = false
      try { await wantPayload(id) } catch { /* 等待下一轮重试 */ }
      for (let attempt = 0; attempt < 20; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 500))
        if (current !== generation) return
        try {
          const retried = await loadContent(id)
          if (current !== generation) return
          content.value = retried
          loadError.value = ''
          loading.value = false
          return
        } catch { /* 继续等待载荷到达 */ }
      }
    }
    if (current === generation) loadError.value = errorMessage(e)
  } finally {
    if (current === generation) loading.value = false
  }
}

async function reload() {
  const item = props.item
  if (!item) return
  const current = ++generation
  try {
    const value = await loadContent(item.id)
    if (current !== generation) return
    content.value = value
    imageFailed.value = false
  } catch (e) {
    if (current === generation) actionError.value = errorMessage(e)
  }
}

const displayTitle = computed(() => {
  const custom = (props.item?.title ?? '').trim()
  if (custom) return custom
  const paths = content.value?.filePaths ?? []
  return paths.length === 1 ? baseName(paths[0]) : ''
})

const sourceLabel = computed(() => {
  const item = props.item
  if (!item) return ''
  const size = item.file_count > 0 ? `${item.file_count} 项` : byteCount(item.length)
  return `来自 ${item.source} · ${size}`
})

const paths = computed(() => content.value?.filePaths ?? [])
const localPaths = computed(() => content.value?.localPaths ?? null)
const isLocal = (path: string) => localPaths.value === null || localPaths.value.includes(path)
const hasLocalFiles = computed(() => paths.value.length > 0 && (localPaths.value === null || paths.value.every(isLocal)))
const canStageFiles = computed(() => {
  const item = props.item
  if (!item || !content.value) return false
  return hasLocalFiles.value && (item.file_count === 0 || paths.value.length === item.file_count)
})
const hasMissingFiles = computed(() => {
  const item = props.item
  return !!item && item.file_count > 0 && !!content.value && !content.value.canRequestFiles && !canStageFiles.value
})

const availability = computed(() => {
  const item = props.item
  if (!item || item.file_count === 0 || !content.value) return null
  if (hasMissingFiles.value) return { label: '文件不可用', icon: 'warning', tone: 'warning' }
  if (canStageFiles.value) return { label: '本地可用', icon: 'check', tone: 'accent' }
  return { label: '使用时获取', icon: 'downloadCircle', tone: 'accent' }
})

const stagingHint = computed(() => {
  const item = props.item
  if (!item || !content.value) return ''
  if (savedImagePath.value) return '图片已保存，可加入暂存区。'
  if (!content.value.filePaths.length && item.has_image) return '这是剪贴板图片；先保存为文件，即可加入暂存区。'
  if (item.file_count > 0 && !canStageFiles.value) {
    return content.value.canRequestFiles
      ? '文件尚未在本地；先复制以获取文件，来源设备需保持连接。'
      : '来源文件已不可用，请重新复制文件。'
  }
  return ''
})

const codeDoc = computed(() => {
  const value = content.value
  if (!value || !value.text) return null
  return detectCode(value.text)
})
const highlighted = computed(() => (codeDoc.value ? highlightCode(codeDoc.value.text) : ''))
const codeHeight = computed(() => {
  if (!codeDoc.value) return 0
  return Math.min(300, codeDoc.value.text.split('\n').length * 22 + 80)
})

const imageHeight = computed(() => Math.max(240, Math.min(440, (paneHeight.value || 668) * 0.55)))
const showSaveImage = computed(() => !!props.item?.has_image && !savedImagePath.value && paths.value.length === 0 && !!content.value?.imageSrc)
const showImageSizing = computed(() => !!content.value?.imageSrc && !imageFailed.value)

/**
 * 来源应用：与原生 ClipboardAppInfoCache 同源，优先 NSWorkspace 解析出的显示名，
 * 再回退到条目自带名称、库状态映射和 bundle id。
 */
const appInfo = useAppInfo(() => props.item?.source_app_id)
const sourceAppIcon = computed(() => appInfo.value?.icon ?? null)
const sourceAppName = computed(() => {
  const item = props.item
  if (!item) return ''
  if (appInfo.value?.name) return appInfo.value.name
  if (item.source_app_name) return item.source_app_name
  const id = item.source_app_id
  if (!id) return ''
  return library.apps.find((app) => app.bundle_id === id)?.name ?? id
})

const metaRows = computed<Array<[string, string]>>(() => {
  const item = props.item
  const value = content.value
  if (!item || !value) return []
  const rows: Array<[string, string]> = [
    ['种类', formatName(item, value)],
    ['大小', item.file_count > 0 ? `${byteCount(item.length)}（${item.file_count} 项）` : byteCount(item.length)],
  ]
  if (imageSize.value) rows.push(['尺寸', `${imageSize.value.width} × ${imageSize.value.height}`])
  rows.push(['来源设备', item.source])
  if (sourceAppName.value) rows.push(['来源应用', sourceAppName.value])
  if (item.copied_at_ms !== null) rows.push(['添加时间', fullDate(item.copied_at_ms)])
  return rows
})

function onImageLoad(event: Event) {
  const image = event.target as HTMLImageElement
  if (image.naturalWidth > 1 && image.naturalHeight > 1) imageSize.value = { width: image.naturalWidth, height: image.naturalHeight }
}

async function copy() {
  const item = props.item
  if (!item || copying.value) return
  copying.value = true
  actionError.value = ''
  try {
    await copyItem(item.id)
    copied.value = true
    window.clearTimeout(copiedTimer)
    copiedTimer = window.setTimeout(() => { copied.value = false }, 1200)
    await reload()
  } catch (e) {
    actionError.value = errorMessage(e)
    emit('error', actionError.value)
  } finally {
    copying.value = false
  }
}

async function stage() {
  const item = props.item
  if (!item || !canStageFiles.value || staging.value) return
  staging.value = true
  actionError.value = ''
  try { await stageItem(item.id) } catch (e) {
    actionError.value = errorMessage(e)
    emit('error', actionError.value)
  } finally {
    staging.value = false
  }
}

/** 剪贴板图片先落到用户选择的文件，再复用文件暂存区；原生 ClipboardPreviewPane.saveImage。 */
async function saveImage() {
  const item = props.item
  if (!item || !item.has_image || savingImage.value) return
  savingImage.value = true
  actionError.value = ''
  try {
    // clipboard.save 现在返回真实结果；只有保存成功才提供“加入暂存区”。
    const result = await saveItem(item)
    if (result?.saved === true && typeof result.path === 'string') savedImagePath.value = result.path
  } catch (e) {
    actionError.value = errorMessage(e)
    emit('error', actionError.value)
  } finally {
    savingImage.value = false
  }
}

async function stageSavedImage() {
  if (!savedImagePath.value || staging.value) return
  staging.value = true
  actionError.value = ''
  try { await addPathsToShelf([savedImagePath.value]) } catch (e) {
    actionError.value = errorMessage(e)
    emit('error', actionError.value)
  } finally {
    staging.value = false
  }
}

watch(() => props.item?.id, () => { savedImagePath.value = '' })

async function revealFiles() {
  const item = props.item
  if (!item || !paths.value.length || !canStageFiles.value) return
  actionError.value = ''
  try { await revealItem(item.id) } catch (e) {
    actionError.value = errorMessage(e)
    emit('error', actionError.value)
  }
}

function openMore(event: MouseEvent) {
  const item = props.item
  if (!item) return
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  more.value = {
    x: rect.right - 210,
    y: rect.bottom + 2,
    items: moreMenu({
      item,
      title: item.title,
      meta: props.meta,
      copying: copying.value,
      hasMissingFiles: hasMissingFiles.value,
      hasLocalFiles: hasLocalFiles.value,
      workflowsEnabled: props.workflowsEnabled,
      onCopy: () => { void copy() },
      onEditTitle: () => { /* 预览更多菜单没有标题项 */ },
      onEditText: () => emit('edit'),
      onDelete: () => { /* 预览更多菜单没有删除项 */ },
      fail: (message) => { actionError.value = message; emit('error', message) },
    }),
  }
}
</script>

<template>
  <aside ref="pane" class="preview" aria-label="内容预览">
    <div v-if="notice || actionError" class="notice" role="alert">
      <Icon name="warning" :size="12" />
      <span>{{ actionError || notice }}</span>
    </div>

    <div v-if="!item" class="empty">
      <Icon name="clipboard" :size="60" :stroke-width="0.8" class="empty-icon" />
      <div class="empty-text">
        <strong>{{ historyEmpty ? '复制内容，从这里接着用' : '选择一条历史' }}</strong>
        <p>{{ historyEmpty ? '在任意应用里复制，按 ⌥⌘V 就能从这里取回；内容会在可信设备之间同步。' : '文字、图片与文件会在可信设备之间同步。' }}</p>
      </div>
    </div>

    <div v-else-if="loading && !content" class="state">
      <span class="spinner" />正在读取预览…
    </div>

    <div v-else-if="loadError && !content" class="empty">
      <Icon name="warning" :size="40" :stroke-width="0.9" class="empty-icon" />
      <div class="empty-text">
        <strong>无法读取预览</strong>
        <p>{{ loadError }}</p>
      </div>
      <button type="button" class="button" @click="start(item.id)">重试</button>
    </div>

    <div v-else-if="content" class="scroll">
      <div class="head">
        <h2 v-if="displayTitle">{{ displayTitle }}</h2>
        <div class="source">
          <span class="source-text">{{ sourceLabel }}</span>
          <span v-if="availability" class="badge" :class="availability.tone">
            <Icon :name="availability.icon" :size="11" />{{ availability.label }}
          </span>
        </div>
      </div>

      <div class="canvas">
        <div v-if="content.imageSrc && !imageFailed" class="image-box" :class="{ fill: fillImage }" :style="{ height: imageHeight + 'px' }">
          <img :src="content.imageSrc" alt="剪贴板图片" @load="onImageLoad" @error="imageFailed = true" />
        </div>

        <div v-else-if="paths.length" class="files">
          <div v-for="path in paths" :key="path" class="file-row">
            <span class="file-icon"><Icon name="doc" :size="20" /></span>
            <div class="file-text">
              <div class="file-name" :title="path">{{ baseName(path) }}</div>
              <div class="file-state" :class="{ missing: !isLocal(path) }">{{ isLocal(path) ? '本地可用' : '文件已不可用，请重新获取' }}</div>
            </div>
          </div>
        </div>

        <div v-else-if="item.file_count > 0" class="unavailable">
          <Icon name="docMissing" :size="40" class="unavailable-icon" />
          <strong>{{ content.canRequestFiles ? '文件尚未在本地' : '文件不可用' }}</strong>
          <p>{{ content.canRequestFiles ? '使用时从来源设备获取；来源设备需要保持连接。' : '来源文件或缓存已不可用，请重新复制。' }}</p>
        </div>

        <div v-else-if="item.has_image" class="unavailable">
          <Icon name="photo" :size="40" class="unavailable-icon" />
          <strong>暂时无法显示图片</strong>
          <p>可以尝试复制原始内容，或从来源设备重新复制。</p>
        </div>

        <div v-else-if="codeDoc" class="code" :style="{ height: codeHeight + 'px' }">
          <div class="code-head"><Icon name="code" :size="12" />{{ codeDoc.language }}</div>
          <div class="code-body"><pre v-html="highlighted" /></div>
        </div>

        <div v-else class="text-preview">
          <pre class="text">{{ content.text }}</pre>
          <p v-if="content.truncated" class="truncated"><Icon name="info" :size="11" />预览已截断；复制会恢复完整内容。</p>
        </div>
      </div>

      <details v-if="item.ocr_text" class="ocr">
        <summary><Icon name="viewfinder" :size="13" /><span>识别文字</span></summary>
        <pre>{{ item.ocr_text }}</pre>
      </details>

      <WorkflowRunStatus :runs="workflowRuns ?? []" :error="workflowError ?? ''" compact />

      <div class="divider" />

      <div class="meta">
        <div v-for="[label, value] in metaRows" :key="label" class="meta-row">
          <span class="meta-label">{{ label }}</span>
          <span class="meta-value" :title="value">
            <img v-if="label === '来源应用' && sourceAppIcon" class="meta-app-icon" :src="sourceAppIcon" alt="" />
            <span class="meta-text">{{ value }}</span>
          </span>
        </div>
      </div>

      <div class="footer">
        <p v-if="stagingHint" class="staging-hint">{{ stagingHint }}</p>
        <div class="actions">
          <button
            v-if="showImageSizing" type="button" class="action sizing" :aria-pressed="fillImage"
            :title="fillImage ? '当前填满预览；点击切换为适合窗口' : '当前适合窗口；点击切换为填满预览'" @click="fillImage = !fillImage"
          >
            <Icon name="expand" :size="13" />{{ fillImage ? '填满预览' : '适合窗口' }}
          </button>
          <button type="button" class="action primary" :class="{ done: copied }" :disabled="copying || hasMissingFiles" @click="copy">
            <Icon :name="copied ? 'check' : 'copy'" :size="13" />{{ copying ? '正在准备…' : copied ? '已复制' : '复制' }}
          </button>
          <button
            v-if="showSaveImage" type="button" class="action" :disabled="savingImage"
            :title="stagingHint || '保存图片到文件'" @click="saveImage"
          >
            <Icon name="download" :size="13" />{{ savingImage ? '正在保存…' : '保存图片…' }}
          </button>
          <button
            v-else-if="savedImagePath" type="button" class="action"
            :disabled="staging" :title="stagingHint" @click="stageSavedImage"
          >
            <Icon name="tray" :size="13" />加入暂存区
          </button>
          <button
            v-else-if="item.file_count > 0 || paths.length" type="button" class="action"
            :disabled="!canStageFiles || staging" :title="stagingHint || '暂存本地文件，可从暂存区拖出或发送'" @click="stage"
          >
            <Icon name="tray" :size="13" />加入暂存区
          </button>
          <button
            v-if="paths.length" type="button" class="action" :disabled="!canStageFiles"
            :title="canStageFiles ? '在访达中显示' : '文件已不可用，请重新获取'" @click="revealFiles"
          >
            <Icon name="folder" :size="13" />在访达中显示
          </button>
          <button type="button" class="action icon" aria-label="更多操作" title="更多操作" @click="openMore">
            <Icon name="ellipsis" :size="14" />
          </button>
        </div>
      </div>
    </div>

    <ContextMenu v-if="more" :items="more.items" :x="more.x" :y="more.y" @close="more = null" />
  </aside>
</template>

<style scoped>
.preview {
  min-width: 0; min-height: 0; display: flex; flex-direction: column; overflow: hidden;
  background: var(--surface, #fff); border-left: 0.5px solid var(--line, rgba(0, 0, 0, 0.09));
}
.notice {
  flex: none; display: flex; align-items: center; gap: 6px; padding: 16px 16px 0;
  font-size: 12px; line-height: 1.4; color: var(--warning, #ed8b23);
}
.empty { flex: 1; min-height: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 24px; text-align: center; }
.empty-icon { color: var(--secondary, #595959); }
.empty-text { display: flex; flex-direction: column; gap: 4px; max-width: 360px; }
.empty-text strong { font-size: 15px; font-weight: 600; color: var(--text, #222); }
.empty-text p { margin: 0; font-size: 13px; line-height: 1.5; color: var(--secondary, #595959); }
.state { flex: 1; display: flex; align-items: center; justify-content: center; gap: 8px; font-size: 13px; color: var(--secondary, #595959); }
.spinner {
  width: 14px; height: 14px; border-radius: 50%;
  border: 2px solid color-mix(in srgb, var(--secondary, #595959) 30%, transparent);
  border-top-color: var(--secondary, #595959); animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.scroll { flex: 1; min-height: 0; overflow: auto; padding: 20px; display: flex; flex-direction: column; gap: 18px; scrollbar-width: none; }
.scroll::-webkit-scrollbar { display: none; }
.head { display: flex; flex-direction: column; gap: 10px; }
.head h2 { margin: 0; font-size: 15px; font-weight: 600; line-height: 1.35; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.source { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; min-width: 0; }
.source-text { font-size: 12px; color: var(--secondary, #595959); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.badge { display: inline-flex; align-items: center; gap: 4px; font-size: 11px; padding: 4px 8px; border-radius: 8px; white-space: nowrap; }
.badge.accent { color: var(--accent, #3276f3); background: color-mix(in srgb, var(--accent, #3276f3) 10%, transparent); }
.badge.warning { color: var(--warning, #ed8b23); background: color-mix(in srgb, var(--warning, #ed8b23) 10%, transparent); }
.canvas { min-width: 0; }
.image-box { display: flex; align-items: center; justify-content: center; overflow: hidden; border-radius: 12px; }
.image-box img { max-width: 100%; max-height: 100%; border-radius: 12px; object-fit: contain; }
.image-box.fill img { width: 100%; height: 100%; object-fit: cover; }
.files { display: flex; flex-direction: column; }
.file-row { display: flex; align-items: center; gap: 16px; padding: 18px 24px; border-bottom: 0.5px solid var(--line, rgba(0, 0, 0, 0.09)); }
.file-row:last-child { border-bottom: 0; }
.file-icon {
  width: 44px; height: 44px; flex: none; border-radius: 8px; display: inline-flex; align-items: center; justify-content: center;
  background: color-mix(in srgb, var(--text, #222) 4%, transparent); color: var(--accent, #3276f3);
}
.file-text { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
.file-name { font-size: 13px; font-weight: 500; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; word-break: break-word; }
.file-state { font-size: 11px; color: var(--secondary, #595959); }
.file-state.missing { color: var(--warning, #ed8b23); }
.unavailable { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 40px 20px; text-align: center; }
.unavailable-icon { color: var(--secondary, #595959); opacity: 0.7; }
.unavailable strong { font-size: 13px; font-weight: 600; }
.unavailable p { margin: 0; font-size: 12px; line-height: 1.5; color: var(--secondary, #595959); max-width: 320px; }
.code { display: flex; flex-direction: column; overflow: hidden; border-radius: 8px; border: 0.5px solid var(--line, rgba(0, 0, 0, 0.09)); background: var(--surface, #fff); }
.code-head { display: flex; align-items: center; gap: 6px; padding: 10px 16px; font-size: 12px; font-weight: 500; color: var(--secondary, #595959); border-bottom: 0.5px solid var(--line, rgba(0, 0, 0, 0.09)); }
.code-body { flex: 1; min-height: 0; overflow: auto; }
.code-body pre { margin: 0; padding: 18px; font: 12px/1.5 ui-monospace, 'SF Mono', Menlo, Consolas, monospace; white-space: pre; }
.code-body :deep(.tok-string) { color: var(--success, #28a745); }
.code-body :deep(.tok-comment) { color: var(--secondary, #595959); }
.code-body :deep(.tok-keyword) { color: var(--accent, #3276f3); }
.code-body :deep(.tok-number) { color: var(--warning, #ed8b23); }
.text-preview { display: flex; flex-direction: column; gap: 16px; padding: 12px 0; }
.text { margin: 0; font: 600 15px/1.6 -apple-system, BlinkMacSystemFont, 'PingFang SC', sans-serif; white-space: pre-wrap; overflow-wrap: anywhere; }
.truncated { display: flex; align-items: center; gap: 5px; margin: 0; font-size: 11px; color: var(--secondary, #595959); }
.ocr { border-radius: 8px; }
.ocr summary { display: flex; align-items: center; gap: 6px; padding: 12px; font-size: 12px; cursor: default; list-style: none; }
.ocr summary::-webkit-details-marker { display: none; }
.ocr pre { margin: 0 12px 12px; padding-top: 6px; max-height: 140px; overflow: auto; font-size: 12px; line-height: 1.5; white-space: pre-wrap; overflow-wrap: anywhere; }
.divider { height: 0.5px; background: var(--line, rgba(0, 0, 0, 0.09)); }
.meta { display: flex; flex-direction: column; gap: 10px; padding: 16px 0; }
.meta-row { display: flex; align-items: baseline; gap: 14px; font-size: 12px; min-width: 0; }
.meta-label { flex: none; width: 62px; color: var(--secondary, #595959); }
.meta-value { min-width: 0; display: inline-flex; align-items: center; gap: 6px; color: var(--text, #222); }
.meta-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.meta-app-icon { flex: none; width: 14px; height: 14px; object-fit: contain; }
.footer { display: flex; flex-direction: column; gap: 12px; }
.staging-hint { margin: 0; font-size: 11px; line-height: 1.4; color: var(--secondary, #595959); }
.actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: 10px; }
.action.sizing { margin-right: auto; }
.action {
  display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; border-radius: 8px;
  font-size: 13px; font-weight: 500; color: var(--text, #222); background: var(--control, rgba(0, 0, 0, 0.04)); cursor: default;
}
.action:hover:not(:disabled) { background: color-mix(in srgb, var(--text, #222) 9%, transparent); }
.action:disabled { opacity: 0.45; }
.action.primary { background: var(--accent, #3276f3); color: #fff; }
.action.primary:hover:not(:disabled) { background: color-mix(in srgb, var(--accent, #3276f3) 88%, #000); }
.action.primary.done { background: var(--control, rgba(0, 0, 0, 0.04)); color: var(--accent, #3276f3); }
.action.icon { width: 32px; padding: 0; justify-content: center; }
</style>
