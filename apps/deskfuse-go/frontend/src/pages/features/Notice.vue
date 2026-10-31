<script setup lang="ts">
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import { copyText, reportProblem } from './core'

// 对应 DFNotice；传入 raw 时额外提供“复制详情 / 反馈问题…”（对应窗口级错误浮层）。
const props = withDefaults(
  defineProps<{
    tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger'
    title: string
    message?: string
    raw?: string
    actionTitle?: string
    dismissible?: boolean
  }>(),
  { tone: 'neutral', dismissible: false },
)
const emit = defineEmits<{ (e: 'action'): void; (e: 'dismiss'): void }>()

const icon = computed(() => ({
  success: 'check-circle',
  warning: 'alert',
  danger: 'x-circle',
  accent: 'info',
  neutral: 'info',
})[props.tone])

const copied = ref(false)
async function copy() {
  copied.value = await copyText(props.raw ?? props.message ?? props.title)
  setTimeout(() => (copied.value = false), 1600)
}
</script>

<template>
  <div class="notice" :class="`tone-${tone}`" role="status">
    <Icon :name="icon" class="glyph" />
    <div class="body">
      <div class="title">{{ title }}</div>
      <div v-if="message" class="message">{{ message }}</div>
      <div v-if="raw" class="tools">
        <button type="button" class="df-link" @click="copy">{{ copied ? '已复制' : '复制详情' }}</button>
        <button type="button" class="df-link" title="自动附上错误原文、版本信息和诊断文件" @click="reportProblem(raw)">反馈问题…</button>
      </div>
    </div>
    <button v-if="actionTitle" type="button" class="button" @click="emit('action')">{{ actionTitle }}</button>
    <button v-if="dismissible" type="button" class="icon-button" aria-label="关闭提示" title="关闭提示" @click="emit('dismiss')">
      <Icon name="x" :size="14" />
    </button>
  </div>
</template>

<style scoped>
.notice {
  --tone: var(--secondary);
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 12px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--tone) 7%, transparent);
  border: 0.5px solid color-mix(in srgb, var(--tone) 24%, transparent);
}
.tone-accent { --tone: var(--accent); }
.tone-success { --tone: var(--success, #34c759); }
.tone-warning { --tone: var(--warning, #ff9500); }
.tone-danger { --tone: var(--danger, #ff3b30); }
.glyph { color: var(--tone); margin-top: 1px; }
.body { flex: 1 1 auto; min-width: 0; display: grid; gap: 2px; }
.title { font-weight: 500; }
.message { font-size: 12px; color: var(--secondary); user-select: text; overflow-wrap: anywhere; }
.tools { display: flex; gap: 12px; margin-top: 4px; }
</style>
