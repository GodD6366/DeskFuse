<script setup lang="ts">
import type { Component } from 'vue'

// 对应 DFSection：分区标签 + 16px 内边距卡片 + 可选说明。
defineProps<{ title?: string; footer?: string; symbol?: Component }>()
</script>

<template>
  <section class="s-section">
    <h3 v-if="title">
      <component :is="symbol" v-if="symbol" :size="12" class="s-symbol" />
      {{ title }}
    </h3>
    <div class="card s-body"><slot /></div>
    <p v-if="footer" class="s-footer">{{ footer }}</p>
  </section>
</template>

<style scoped>
.s-section { display: grid; gap: 8px; }
h3 {
  display: flex; align-items: center; gap: 6px; margin: 0; padding-left: 4px;
  font-size: 13px; font-weight: 600; color: var(--secondary);
}
.s-symbol { flex: none; }
/* 卡片内边距与原生 DFMetrics.cardPadding 一致；行间分隔线上下各留 8px，与原生 Divider 一致。
   原生行的内容最小高度是 32（再加减 2px 内边距）；带分隔线的行把上方留白放进 padding，
   所以最小高度要同步加回这段留白。 */
/* 原生卡片描边绘制在圆角内，不占布局；用 inset 阴影代替 1px 边框以保持与原生一致的高度。 */
.s-body { display: grid; gap: 8px; padding: 16px; border: 0; box-shadow: inset 0 0 0 0.5px var(--line); }
.s-body :deep(.s-row + .s-row) {
  min-height: 44px; padding-top: 10px; border-top: 0.5px solid var(--line);
}
.s-body :deep(.s-row + .s-row.compact) { min-height: 32px; padding-top: 8px; }
.s-body :deep(.toggle) { width: 40px; margin: 0; }
.s-body :deep(.toggle:checked)::after { transform: translateX(20px); }
.s-footer { margin: 0; padding: 0 4px; font-size: 12px; line-height: 15px; color: var(--secondary); }
/* 原生破坏性按钮为红底白字，而不是红字普通按钮。 */
.s-body :deep(.button.destructive) { background: var(--danger); color: #fff; border-color: transparent; }
.s-body :deep(.button.destructive:hover:not(:disabled)) { filter: brightness(1.06); }
.s-body :deep(.button.destructive:disabled) { opacity: 0.45; }
.s-body :deep(.button.primary:disabled) { background: var(--control); color: var(--secondary); border-color: transparent; opacity: 1; }
</style>
