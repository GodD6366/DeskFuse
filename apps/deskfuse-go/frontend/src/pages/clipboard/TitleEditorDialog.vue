<script setup lang="ts">
import { onMounted, ref } from 'vue'
import Dialog from './Dialog.vue'

const props = defineProps<{ title: string | null }>()
const emit = defineEmits<{ save: [title: string]; close: [] }>()

const draft = ref(props.title ?? '')
const input = ref<HTMLInputElement | null>(null)
onMounted(() => input.value?.focus())

function save() {
  const value = draft.value.trim()
  if (value !== (props.title ?? '')) emit('save', value)
  emit('close')
}
</script>

<template>
  <Dialog :width="360" label="编辑标题" @close="emit('close')">
    <form class="body" @submit.prevent="save">
      <h2>编辑标题</h2>
      <input ref="input" v-model="draft" class="text" type="text" placeholder="标题" />
      <p>留空并保存可移除自定义标题，不会修改剪贴板内容。</p>
      <div class="actions">
        <span class="spacer" />
        <button type="button" @click="emit('close')">取消</button>
        <button type="submit" class="primary">保存</button>
      </div>
    </form>
  </Dialog>
</template>

<style scoped>
.body { padding: 24px; display: flex; flex-direction: column; gap: 16px; }
h2 { margin: 0; font-size: 20px; font-weight: 700; }
.text {
  height: 28px;
  padding: 0 8px;
  border-radius: 6px;
  border: 0.5px solid var(--line, rgba(0, 0, 0, 0.18));
  background: var(--surface, #fff);
  color: inherit;
  font: inherit;
  font-size: 13px;
}
.text:focus { outline: 2px solid var(--accent, #3276f3); outline-offset: -1px; }
p { margin: 0; font-size: 11px; color: var(--secondary, #6e6e73); line-height: 1.4; }
.actions { display: flex; gap: 10px; align-items: center; }
.spacer { flex: 1; }
button {
  height: 28px;
  padding: 0 14px;
  border-radius: 7px;
  border: 0.5px solid var(--line, rgba(0, 0, 0, 0.14));
  background: var(--control, rgba(0, 0, 0, 0.04));
  color: var(--text, #1d1d1f);
  font: inherit;
  font-size: 13px;
  cursor: default;
}
button.primary { background: var(--accent, #3276f3); border-color: transparent; color: #fff; }
</style>
