<script setup lang="ts">
import { computed } from 'vue'
import type { ItemMeta, LibraryItem } from '../../types/clipboard'
import Icon from './Icon.vue'
import { clipboardTitle, timeText } from './format'
import { useThumbnail } from './useThumbnail'
import { useAppInfo } from './appInfo'

// 菜单栏面板的瀑布流卡片，对应原生 MenuBarClipboardCard：
// 没有悬停操作按钮、没有图片放大，标题居中，右下角时间与 ⌘n 对齐。
const props = defineProps<{
  item: LibraryItem
  meta?: ItemMeta
  selected: boolean
  ocrHit: boolean
  index: number
  showShortcut: boolean
}>()
const emit = defineEmits<{ editTitle: [] }>()

const { src, failed, loaded, onLoad, onError } = useThumbnail(() => props.item, 320)
const loading = computed(() => !!src.value && !loaded.value && !failed.value)
const appInfo = useAppInfo(() => props.item.source_app_id)
const appIcon = computed(() => appInfo.value?.icon ?? null)

const customTitle = computed(() => (props.item.title ?? '').trim())
const remotePending = computed(() => props.meta?.remote === true && props.meta.available === false)
const unavailableText = computed(() => (remotePending.value ? '待获取图片' : '图片不可用'))
const hasMetadata = computed(() => props.item.copied_at_ms !== null || props.ocrHit || remotePending.value)
</script>

<template>
  <div class="panel-card" :class="{ selected }" :data-id="item.id" role="option" :aria-selected="selected">
    <img v-if="appIcon" class="watermark" :src="appIcon" alt="" draggable="false" aria-hidden="true" />

    <div v-if="customTitle" class="head" :class="{ selected }" title="双击编辑标题" @dblclick.stop="emit('editTitle')">
      <span>{{ customTitle }}</span>
    </div>

    <div class="inner">
      <div class="content">
        <template v-if="item.has_image">
          <img v-if="src && !failed" :src="src" alt="" class="image" draggable="false" @load="onLoad" @error="onError" />
          <div v-else class="placeholder">
            <span v-if="loading" class="spinner" aria-label="正在读取图片" />
            <span v-else class="missing"><Icon name="photoMissing" :size="12" />{{ unavailableText }}</span>
          </div>
        </template>
        <div v-else class="text">{{ clipboardTitle(item) }}</div>
      </div>

      <div v-if="hasMetadata || index < 9" class="meta">
        <span class="flags">
          <span v-if="ocrHit" title="由图片识别文字匹配"><Icon name="viewfinder" :size="11" /></span>
          <span v-if="remotePending" title="使用时从来源设备获取"><Icon name="downloadCircle" :size="11" /></span>
        </span>
        <span class="time">{{ timeText(item.copied_at_ms) }}</span>
        <span class="shortcut" :class="{ shown: showShortcut && index < 9 }">⌘ {{ index + 1 }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.panel-card {
  position: relative; overflow: hidden; border-radius: 12px; cursor: default;
  background: color-mix(in srgb, var(--text, #222) 5%, transparent);
  border: 1px solid color-mix(in srgb, var(--line, rgba(0, 0, 0, 0.09)) 60%, transparent);
  transition: background 0.16s ease;
}
.panel-card.selected {
  background: color-mix(in srgb, var(--accent, #3276f3) 18%, transparent);
  border-color: color-mix(in srgb, var(--accent, #3276f3) 25%, transparent);
}
.watermark {
  position: absolute; right: -22px; bottom: -22px; width: 88px; height: 88px;
  object-fit: contain; transform: rotate(-45deg); opacity: 0.05; pointer-events: none;
  user-select: none; z-index: 0;
}
.head {
  position: relative; z-index: 1; padding: 8px 10px; text-align: center;
  font-size: 13px; font-weight: 600; line-height: 1.3;
  background: color-mix(in srgb, var(--text, #222) 4%, transparent);
  border-bottom: 0.5px solid color-mix(in srgb, var(--line, rgba(0, 0, 0, 0.09)) 50%, transparent);
}
.head.selected { background: color-mix(in srgb, var(--accent, #3276f3) 8%, transparent); }
.head span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inner { position: relative; z-index: 1; padding: 10px; display: flex; flex-direction: column; gap: 10px; }
.content { min-width: 0; }
.image {
  display: block; margin: 0 auto; width: auto; height: auto;
  max-width: 100%; max-height: 160px; object-fit: contain; border-radius: 8px;
}
.placeholder {
  aspect-ratio: 16 / 10; max-height: 160px; border-radius: 8px; display: flex; align-items: center; justify-content: center;
  background: color-mix(in srgb, var(--text, #222) 3.5%, transparent); color: var(--secondary, #595959); font-size: 11px;
}
.missing { display: inline-flex; align-items: center; gap: 4px; }
.spinner {
  width: 14px; height: 14px; border-radius: 50%; border: 2px solid color-mix(in srgb, var(--secondary, #595959) 30%, transparent);
  border-top-color: var(--secondary, #595959); animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.text {
  font-size: 13px; line-height: 1.45; color: var(--text, #222);
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; word-break: break-word;
}
.meta { position: relative; min-height: 12px; display: flex; align-items: center; font-size: 11px; color: var(--secondary, #595959); }
.meta .flags { display: inline-flex; gap: 4px; min-width: 0; }
.meta .time { position: absolute; left: 0; right: 0; text-align: center; font-variant-numeric: tabular-nums; pointer-events: none; }
.meta .shortcut { margin-left: auto; font-variant-numeric: tabular-nums; opacity: 0; transition: opacity .12s ease; }
.meta .shortcut.shown { opacity: 1; }
</style>
