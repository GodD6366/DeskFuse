<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { ItemMeta, LibraryItem } from '../../types/clipboard'
import Icon from './Icon.vue'
import { byteCount, cardKind, clipboardTitle, linkParts, timeText } from './format'
import { useThumbnail } from './useThumbnail'
import { useAppInfo } from './appInfo'

const props = defineProps<{
  item: LibraryItem
  meta?: ItemMeta
  selected: boolean
  ocrHit: boolean
  copyEnabled: boolean
  /** 返回是否真的复制成功；失败由调用方展示。 */
  copy: () => Promise<boolean>
}>()
const emit = defineEmits<{ editTitle: []; delete: [] }>()

const root = ref<HTMLElement | null>(null)
const { src, failed, loaded, onLoad, onError } = useThumbnail(() => props.item, 480)
const loading = computed(() => !!src.value && !loaded.value && !failed.value)
// 卡片右下角的来源应用水印（原生 ClipboardMasonryGrid.sourceAppWatermark）。
const appInfo = useAppInfo(() => props.item.source_app_id)
const appIcon = computed(() => appInfo.value?.icon ?? null)

const kind = computed(() => cardKind(props.item))
const customTitle = computed(() => (props.item.title ?? '').trim())
const link = computed(() => linkParts(props.item))
const remotePending = computed(() => props.meta?.remote === true && props.meta.available === false)
const unavailableText = computed(() => (remotePending.value ? '待获取图片' : '图片不可用'))

const hovering = ref(false)
const copied = ref(false)
let copiedTimer: number | undefined

async function onCopy() {
  if (copied.value) return
  if (await props.copy()) {
    copied.value = true
    window.clearTimeout(copiedTimer)
    copiedTimer = window.setTimeout(() => { copied.value = false }, 1200)
  }
}

// 悬停 500ms 后弹出图片大图。
const popup = ref<{ left: number; top: number } | null>(null)
let popupTimer: number | undefined
function enter() {
  hovering.value = true
  if (!props.item.has_image) return
  popupTimer = window.setTimeout(() => {
    const rect = root.value?.getBoundingClientRect()
    if (!rect || !src.value) return
    const left = Math.min(rect.right + 8, window.innerWidth - 624)
    popup.value = { left: Math.max(8, left), top: Math.max(8, Math.min(rect.top, window.innerHeight - 464)) }
  }, 500)
}
function leave() {
  hovering.value = false
  window.clearTimeout(popupTimer)
  popup.value = null
}
onBeforeUnmount(() => { window.clearTimeout(popupTimer); window.clearTimeout(copiedTimer) })
</script>

<template>
  <div
    ref="root" class="card" role="option" :data-id="item.id" :aria-selected="selected"
    :class="{ selected, hover: hovering }" :title="item.preview"
    @mouseenter="enter" @mouseleave="leave"
  >
    <img v-if="appIcon" class="watermark" :src="appIcon" alt="" draggable="false" aria-hidden="true" />

    <div v-if="customTitle" class="head" :class="{ selected }" title="双击编辑标题" @dblclick.stop="emit('editTitle')">
      <span>{{ customTitle }}</span>
    </div>

    <div class="inner">
      <div class="content">
        <template v-if="kind === 'image'">
          <img v-if="src && !failed" :src="src" alt="" class="image" draggable="false" @load="onLoad" @error="onError" />
          <div v-else class="placeholder">
            <span v-if="loading" class="spinner" aria-label="正在读取图片" />
            <span v-else class="missing"><Icon name="photoMissing" :size="12" />{{ unavailableText }}</span>
          </div>
        </template>

        <div v-else-if="kind === 'file'" class="file">
          <Icon name="doc" :size="16" class="file-icon" />
          <div class="file-text">
            <div class="file-name">{{ clipboardTitle(item) }}</div>
            <div class="file-meta">{{ item.file_count > 1 ? `${item.file_count} 项 · ${byteCount(item.length)}` : byteCount(item.length) }}</div>
          </div>
        </div>

        <div v-else-if="kind === 'link'" class="link">
          <span class="link-icon"><Icon name="link" :size="13" /></span>
          <div class="link-text">
            <div class="host">{{ link?.host ?? clipboardTitle(item) }}</div>
            <div v-if="link?.path" class="path">{{ link.path }}</div>
          </div>
        </div>

        <div v-else-if="kind === 'code'" class="code">{{ clipboardTitle(item) }}</div>
        <div v-else class="plain">{{ clipboardTitle(item) }}</div>
      </div>

      <div class="foot">
        <span v-if="item.copied_at_ms !== null" class="time">{{ timeText(item.copied_at_ms) }}</span>
        <span class="device">{{ meta?.originDevice ?? item.source }}</span>
        <span class="flags">
          <span v-if="ocrHit" title="由图片识别文字匹配"><Icon name="viewfinder" :size="11" /></span>
          <span v-if="remotePending" title="使用时从来源设备获取"><Icon name="downloadCircle" :size="11" /></span>
        </span>
      </div>
    </div>

    <div v-if="hovering" class="actions">
      <button type="button" :disabled="!copyEnabled" :class="{ done: copied }" title="复制" aria-label="复制" @click.stop="onCopy" @mousedown.stop @dblclick.stop>
        <Icon :name="copied ? 'check' : 'copy'" :size="11" />
      </button>
      <button type="button" title="删除" aria-label="删除" @click.stop="emit('delete')" @mousedown.stop @dblclick.stop>
        <Icon name="trash" :size="11" />
      </button>
    </div>

    <Teleport to="body">
      <div v-if="popup && src" class="zoom" :style="{ left: popup.left + 'px', top: popup.top + 'px' }">
        <img :src="src" alt="图片放大预览" />
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.card {
  position: relative; overflow: hidden; border-radius: 12px; cursor: default;
  padding: 0;
  background: var(--surface, #fff);
  border: 0.5px solid color-mix(in srgb, var(--text, #222) 8%, transparent);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08);
  transition: transform 0.2s ease, box-shadow 0.2s ease, background 0.2s ease;
}
.card.hover { transform: translateY(-1px); box-shadow: 0 4px 12px rgba(0, 0, 0, 0.12); }
.card.selected {
  background: color-mix(in srgb, var(--accent, #3276f3) 18%, transparent);
  border-color: var(--accent, #3276f3);
  box-shadow: inset 0 0 0 1px var(--accent, #3276f3), 0 2px 8px rgba(0, 0, 0, 0.08);
}
.watermark {
  position: absolute; right: -24px; bottom: -24px; width: 96px; height: 96px;
  object-fit: contain; transform: rotate(-45deg); opacity: 0.07; pointer-events: none;
  user-select: none; z-index: 0;
}
.head {
  position: relative; z-index: 1;
  padding: 8px 10px; text-align: center; font-size: 13px; font-weight: 600; line-height: 1.3;
  background: color-mix(in srgb, var(--text, #222) 4%, transparent);
  border-bottom: 0.5px solid color-mix(in srgb, var(--line, rgba(0, 0, 0, 0.09)) 50%, transparent);
}
.head.selected { background: color-mix(in srgb, var(--accent, #3276f3) 8%, transparent); }
.head span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inner { position: relative; z-index: 1; padding: 10px; display: flex; flex-direction: column; gap: 6px; }
.content { min-width: 0; }
.image { display: block; margin: 0 auto; width: 100%; max-height: 200px; object-fit: contain; border-radius: 8px; }
.placeholder {
  aspect-ratio: 16 / 10; max-height: 200px; border-radius: 8px; display: flex; align-items: center; justify-content: center;
  background: color-mix(in srgb, var(--text, #222) 3.5%, transparent); color: var(--secondary, #595959); font-size: 11px;
}
.missing { display: inline-flex; align-items: center; gap: 4px; }
.spinner {
  width: 14px; height: 14px; border-radius: 50%; border: 2px solid color-mix(in srgb, var(--secondary, #595959) 30%, transparent);
  border-top-color: var(--secondary, #595959); animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.file { display: flex; align-items: center; gap: 8px; }
.file-icon { color: var(--accent, #3276f3); width: 24px; }
.file-text { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.file-name { font-size: 13px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.file-meta { font-size: 11px; color: var(--secondary, #595959); }
.link { display: flex; align-items: center; gap: 8px; }
.link-icon { width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; color: var(--secondary, #595959); }
.link-text { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.host { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.path { font-size: 11px; color: var(--secondary, #595959); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.code {
  font: 12px/1.5 ui-monospace, 'SF Mono', Menlo, Consolas, monospace; padding: 8px; border-radius: 8px;
  background: color-mix(in srgb, var(--text, #222) 4%, transparent);
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; white-space: pre-wrap; word-break: break-word;
}
.plain {
  font-size: 13px; line-height: 1.45;
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; word-break: break-word;
}
.foot { display: flex; align-items: center; gap: 6px; font-size: 11px; color: var(--secondary, #595959); min-width: 0; }
.foot .time { font-variant-numeric: tabular-nums; }
.foot .device { flex: 1 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.foot .flags { margin-left: auto; display: inline-flex; gap: 4px; }
.actions { position: absolute; top: 6px; right: 6px; display: flex; gap: 6px; z-index: 1; }
.actions button {
  width: 24px; height: 24px; padding: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
  background: var(--surface, #fff); color: var(--text, #222); border: 0.5px solid var(--line, rgba(0, 0, 0, 0.12)); cursor: default;
}
.actions button:disabled { color: var(--secondary, #595959); }
.actions button.done { color: var(--accent, #3276f3); }
.zoom {
  position: fixed; z-index: 800; padding: 12px; border-radius: 12px; pointer-events: none;
  background: var(--surface, #fff); border: 0.5px solid var(--line, rgba(0, 0, 0, 0.12)); box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
}
.zoom img { display: block; max-width: 600px; max-height: 440px; }
</style>
