<script setup lang="ts">
import { computed } from 'vue'
import type { ItemMeta, LibraryItem } from '../../types/clipboard'
import Icon from './Icon.vue'
import { displayTitle, recordSymbol, timeText } from './format'
import { useThumbnail } from './useThumbnail'
import { useAppInfo } from './appInfo'

const props = defineProps<{
  item: LibraryItem
  meta?: ItemMeta
  selected: boolean
  focused: boolean
  ocrHit: boolean
  /** 面板行在时间下保留 ⌘n 提示位；主页面不传。 */
  shortcut?: number | null
  showShortcut?: boolean
  /** 面板行没有悬停高亮（原生 MenuBarClipboardRow 只在选中时着色）。 */
  hoverable?: boolean
  /** 面板把 data-id 放在外层行上，内层不再重复。 */
  identify?: boolean
}>()

const { src, failed, onLoad, onError } = useThumbnail(() => props.item, 96)
// 缩略图不可用时用来源应用图标顶替类型图标（原生 ClipboardRecordLabel.thumbnailView）。
const appInfo = useAppInfo(() => props.item.source_app_id)
const appIcon = computed(() => appInfo.value?.icon ?? null)

const symbol = computed(() => ({ files: 'doc', image: 'photo', text: 'text' }[recordSymbol(props.item)]))
const remotePending = computed(() => props.meta?.remote === true && props.meta.available === false)
const panelLayout = computed(() => props.shortcut !== null && props.shortcut !== undefined)
</script>

<template>
  <div
    class="row" role="option" :data-id="identify === false ? undefined : item.id" :aria-selected="selected"
    :class="{ selected, ring: selected && focused, 'no-hover': hoverable === false }"
    :title="item.preview"
  >
    <div class="thumb" :title="failed ? '图片不可用' : undefined">
      <img v-if="src && !failed" :src="src" alt="" draggable="false" @load="onLoad" @error="onError" />
      <img v-else-if="appIcon" class="app-icon" :src="appIcon" alt="" draggable="false" />
      <Icon v-else :name="symbol" :size="20" />
    </div>
    <div class="text">
      <div class="title">{{ displayTitle(item) }}</div>
      <div class="origin">来自 {{ meta?.originDevice ?? item.source }}</div>
    </div>
    <div v-if="panelLayout" class="trail panel">
      <div class="time-row">
        <span v-if="ocrHit" title="由图片识别文字匹配"><Icon name="viewfinder" :size="11" /></span>
        <span v-if="remotePending" title="使用时从来源设备获取"><Icon name="downloadCircle" :size="11" /></span>
        <span v-if="item.copied_at_ms !== null" class="time">{{ timeText(item.copied_at_ms) }}</span>
      </div>
      <span class="shortcut" :class="{ shown: showShortcut }">⌘ {{ shortcut }}</span>
    </div>
    <div v-else class="trail">
      <span v-if="item.copied_at_ms !== null" class="time">{{ timeText(item.copied_at_ms) }}</span>
      <span class="flags">
        <span v-if="ocrHit" title="由图片识别文字匹配"><Icon name="viewfinder" :size="11" /></span>
        <span v-if="remotePending" title="使用时从来源设备获取"><Icon name="downloadCircle" :size="11" /></span>
      </span>
    </div>
  </div>
</template>

<style scoped>
.row {
  display: flex; align-items: center; gap: 12px; min-height: 56px; padding: 6px 10px; border-radius: 8px;
  border: 1px solid transparent; box-sizing: border-box; cursor: default;
  transition: background 0.16s ease;
}
.row:hover { background: color-mix(in srgb, var(--accent, #3276f3) 6%, transparent); }
.row.no-hover:hover { background: transparent; }
.row.selected { background: color-mix(in srgb, var(--accent, #3276f3) 18%, transparent); }
.row.ring { border-color: color-mix(in srgb, var(--accent, #3276f3) 50%, transparent); }
.thumb {
  flex: none; width: 42px; height: 42px; border-radius: 8px; overflow: hidden;
  display: flex; align-items: center; justify-content: center;
  background: color-mix(in srgb, var(--text, #222) 3.5%, transparent); color: var(--secondary, #595959);
}
.thumb img { width: 100%; height: 100%; object-fit: cover; display: block; user-select: none; }
.thumb img.app-icon { width: 30px; height: 30px; object-fit: contain; }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
.title {
  font-size: 13px; font-weight: 500; line-height: 1.3; color: var(--text, #222);
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; word-break: break-word;
}
.origin { font-size: 11px; color: var(--secondary, #595959); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.trail { flex: none; display: flex; flex-direction: column; align-items: flex-end; gap: 4px; }
.time-row { display: inline-flex; align-items: center; gap: 4px; min-height: 11px; color: var(--secondary, #595959); }
.flags { display: inline-flex; gap: 4px; color: var(--secondary, #595959); min-height: 11px; }
.trail.panel { gap: 12px; }
.shortcut { font-size: 12px; font-variant-numeric: tabular-nums; color: var(--secondary, #595959); opacity: 0; transition: opacity .12s ease; }
.shortcut.shown { opacity: 1; }
.time { font-size: 12px; font-variant-numeric: tabular-nums; color: var(--secondary, #595959); }
</style>
