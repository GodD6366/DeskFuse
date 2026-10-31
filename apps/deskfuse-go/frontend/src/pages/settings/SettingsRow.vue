<script setup lang="ts">
import type { Component } from 'vue'

// 对应 DFSettingRow：左侧可带图标，标题与说明在左，控件在右。
// compact 用于 DFValueRow 形式的只读信息行（无说明、24px 高）。
defineProps<{ title: string; detail?: string; badge?: string; icon?: Component; subdued?: boolean; compact?: boolean }>()
</script>

<template>
  <div class="s-row" :class="{ compact }">
    <div class="s-text">
      <component :is="icon" v-if="icon" :size="14" class="s-icon" />
      <div class="s-copy">
        <div class="s-title" :class="{ subdued }">
          <span>{{ title }}</span>
          <span v-if="badge" class="s-badge">{{ badge }}</span>
        </div>
        <div v-if="detail" class="s-detail">{{ detail }}</div>
        <slot name="extra" />
      </div>
    </div>
    <div class="s-control"><slot /></div>
  </div>
</template>

<style scoped>
.s-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 36px; padding: 2px 0; }
.s-row.compact { min-height: 24px; padding: 0; }
.s-text { display: flex; align-items: flex-start; gap: 12px; flex: 1; min-width: 0; }
.s-icon { flex: none; width: 18px; margin-top: 1px; color: var(--secondary); }
.s-copy { display: grid; gap: 2px; min-width: 0; }
.s-title { display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 400; line-height: 15px; }
.s-title.subdued { color: var(--secondary); }
.s-detail { font-size: 12px; line-height: 14px; color: var(--secondary); }
.s-control { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.s-badge {
  padding: 1px 6px; border-radius: 999px; font-size: 11px; font-weight: 600;
  color: var(--warning); background: color-mix(in srgb, var(--warning) 12%, transparent);
}
</style>
