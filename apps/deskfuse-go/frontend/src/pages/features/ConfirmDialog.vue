<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'

// 对应 confirmationDialog / sheet：Esc 取消，点遮罩取消，打开时焦点进入对话框。
const props = withDefaults(
  defineProps<{
    open: boolean
    title: string
    message?: string
    confirmTitle?: string
    cancelTitle?: string
    destructive?: boolean
    hideConfirm?: boolean
    width?: number
  }>(),
  { confirmTitle: '确定', cancelTitle: '取消', destructive: false, hideConfirm: false, width: 420 },
)
const emit = defineEmits<{ (e: 'confirm'): void; (e: 'cancel'): void }>()

const panel = ref<HTMLElement | null>(null)
watch(
  () => props.open,
  async (open) => {
    if (!open) return
    await nextTick()
    panel.value?.focus()
  },
)
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="scrim" @mousedown.self="emit('cancel')">
      <div
        ref="panel"
        class="panel card"
        role="dialog"
        aria-modal="true"
        :aria-label="title"
        :style="{ width: `${width}px` }"
        tabindex="-1"
        @keydown.esc.stop="emit('cancel')"
      >
        <h2 class="head">{{ title }}</h2>
        <p v-if="message" class="message">{{ message }}</p>
        <slot />
        <div class="actions">
          <button type="button" class="button" @click="emit('cancel')">{{ cancelTitle }}</button>
          <button
            v-if="!hideConfirm"
            type="button"
            class="button"
            :class="destructive ? 'destructive' : 'primary'"
            @click="emit('confirm')"
          >
            {{ confirmTitle }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  background: rgba(0, 0, 0, 0.32);
}
.panel {
  max-width: calc(100vw - 48px);
  max-height: calc(100vh - 48px);
  overflow: auto;
  padding: 20px;
  background: var(--surface);
  color: var(--text);
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.28);
  outline: none;
  display: grid;
  gap: 12px;
}
.head { margin: 0; font-size: 15px; font-weight: 600; }
.message { margin: 0; color: var(--secondary); font-size: 12px; overflow-wrap: anywhere; }
.actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
.destructive { color: #fff; background: var(--danger, #ff3b30); border-color: transparent; }
</style>
