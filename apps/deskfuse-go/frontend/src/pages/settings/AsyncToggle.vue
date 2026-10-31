<script setup lang="ts">
import { ref, watch } from 'vue'
import { backend } from '../../services/backend'
import { errorText } from './util'

// 开关立即提交后端；失败时回退并显示真实错误，不假装已保存。
const props = defineProps<{
  value: boolean
  action: string
  params: (next: boolean) => Record<string, unknown>
  label: string
  disabled?: boolean
}>()
const emit = defineEmits<{ applied: [value: boolean]; error: [message: string | null] }>()

const shown = ref(props.value)
const pending = ref(false)
const error = ref<string | null>(null)

watch(() => props.value, (value) => { if (!pending.value) shown.value = value })

async function change(event: Event) {
  const next = (event.target as HTMLInputElement).checked
  shown.value = next
  pending.value = true
  error.value = null
  emit('error', null)
  try {
    await backend.invoke(props.action, props.params(next))
    emit('applied', next)
  } catch (e) {
    shown.value = !next
    ;(event.target as HTMLInputElement).checked = !next
    error.value = errorText(e)
    emit('error', error.value)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <span class="async-toggle">
    <input class="toggle" type="checkbox" role="switch" :aria-label="label" :checked="shown" :disabled="disabled || pending" @change="change" />
    <small v-if="error" class="err" role="alert">{{ error }}</small>
  </span>
</template>

<style scoped>
.async-toggle { display: inline-flex; flex-direction: column; align-items: flex-end; gap: 4px; }
.err { max-width: 260px; font-size: 11px; text-align: right; color: var(--danger); }
</style>
