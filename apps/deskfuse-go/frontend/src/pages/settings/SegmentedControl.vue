<script setup lang="ts" generic="T extends string">
import { ref } from 'vue'

const props = defineProps<{ options: { value: T; label: string }[]; label: string }>()
const model = defineModel<T>({ required: true })
const root = ref<HTMLElement | null>(null)

// radiogroup：方向键切换并把焦点带到新的选项。
function onKey(event: KeyboardEvent, index: number) {
  const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0
  if (!step) return
  event.preventDefault()
  const next = (index + step + props.options.length) % props.options.length
  model.value = props.options[next].value
  root.value?.querySelectorAll<HTMLElement>('[role=radio]')[next]?.focus()
}
</script>

<template>
  <div ref="root" class="seg" role="radiogroup" :aria-label="label">
    <button v-for="(option, index) in options" :key="option.value" type="button" role="radio"
      :aria-checked="option.value === model" :tabindex="option.value === model ? 0 : -1"
      :class="{ on: option.value === model }" @click="model = option.value" @keydown="onKey($event, index)">{{ option.label }}</button>
  </div>
</template>

<style scoped>
.seg { display: inline-flex; gap: 2px; padding: 2px; border-radius: 8px; background: var(--control); }
.seg button {
  min-height: 24px; padding: 0 10px; border-radius: 6px; font-size: 12px; color: var(--secondary); white-space: nowrap;
  transition: background-color var(--dur-fast) var(--ease-out), color var(--dur-fast) var(--ease-out);
}
.seg button:hover:not(.on) { color: var(--text); }
.seg button.on { background: var(--surface); color: var(--text); font-weight: 500; box-shadow: 0 0 0 0.5px var(--line), 0 1px 2px rgba(0, 0, 0, 0.06); }
</style>
