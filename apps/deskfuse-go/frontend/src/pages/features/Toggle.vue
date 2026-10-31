<script setup lang="ts">
// 开关：原生 checkbox 加 role="switch"，键盘与读屏行为沿用浏览器。
defineProps<{ modelValue: boolean; label: string; disabled?: boolean }>()
const emit = defineEmits<{ (e: 'update:modelValue', value: boolean): void }>()
</script>

<template>
  <input
    class="toggle df-switch"
    type="checkbox"
    role="switch"
    :checked="modelValue"
    :disabled="disabled"
    :aria-label="label"
    @change="emit('update:modelValue', ($event.target as HTMLInputElement).checked)"
  />
</template>

<style scoped>
.df-switch {
  appearance: none;
  -webkit-appearance: none;
  position: relative;
  flex: none;
  width: 34px;
  height: 20px;
  margin: 0;
  border-radius: 999px;
  background: color-mix(in srgb, var(--secondary) 35%, transparent);
  cursor: pointer;
  transition: background 0.18s ease;
}
.df-switch::after {
  content: '';
  position: absolute;
  top: 2px;
  left: 2px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
  transition: transform 0.18s ease;
}
.df-switch:checked { background: var(--accent); }
.df-switch:checked::after { transform: translateX(14px); }
.df-switch:disabled { opacity: 0.5; cursor: default; }
.df-switch:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
