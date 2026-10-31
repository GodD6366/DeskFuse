<script setup lang="ts">
// 对应 SettingsSaveFooter：底部固定保存栏，状态在左、操作在右。
defineProps<{
  dirty: boolean
  saving: boolean
  canSave: boolean
  canRevert: boolean
  status?: string | null
  error?: string | null
  saveText?: string
}>()
defineEmits<{ revert: []; save: [] }>()
</script>

<template>
  <footer class="save-footer">
    <div class="bar">
      <span v-if="saving" class="note">保存中…</span>
      <span v-else-if="dirty" class="note">未保存</span>
      <span v-else-if="status" class="note">{{ status }}</span>
      <span class="spacer" />
      <button type="button" class="button" :disabled="!canRevert || saving" @click="$emit('revert')">撤销修改</button>
      <button type="button" class="button primary" :disabled="!canSave || saving" @click="$emit('save')">
        {{ saving ? '保存中…' : (saveText ?? '保存') }}
      </button>
    </div>
    <p v-if="error" class="err" role="alert">⚠ {{ error }}</p>
  </footer>
</template>

<style scoped>
.save-footer { display: grid; gap: 8px; padding: 0 0 12px; border-top: 0.5px solid var(--line); }
.bar { display: flex; align-items: center; gap: 8px; padding: 0 12px; }
.spacer { flex: 1; }
.note { font-size: 11px; color: var(--secondary); }
.bar :deep(.button.primary:disabled),
.bar .button.primary:disabled { background: var(--control); color: var(--secondary); border-color: transparent; opacity: 1; }
.err { margin: 0; padding: 0 12px; font-size: 11px; line-height: 1.5; color: var(--danger); }
</style>
