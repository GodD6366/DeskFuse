<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import Icon from './Icon.vue'
import type { MenuItem } from './menu'

const props = defineProps<{ items: MenuItem[]; x: number; y: number }>()
const emit = defineEmits<{ close: [] }>()

const root = ref<HTMLElement | null>(null)
const pos = reactive({ left: props.x, top: props.y })
const openIndex = ref(-1)
const subPos = reactive({ left: 0, top: 0 })
const loaded = reactive<Record<number, MenuItem[]>>({})
const loading = reactive<Record<number, boolean>>({})
const loadError = reactive<Record<number, string>>({})

function clamp() {
  const el = root.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  pos.left = Math.max(4, Math.min(props.x, window.innerWidth - rect.width - 4))
  pos.top = Math.max(4, Math.min(props.y, window.innerHeight - rect.height - 4))
}

function onPointerDown(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) emit('close')
}
function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); emit('close'); return }
  // 菜单打开期间，快捷键等同于点击对应菜单项（NSMenu 的 key equivalent）。
  if ((event.metaKey || event.ctrlKey) && event.key.length === 1) {
    const hit = props.items.find((item) => item.shortcut?.toUpperCase().endsWith(event.key.toUpperCase()))
    if (hit && !hit.disabled) { event.preventDefault(); choose(hit) }
  }
}
const close = () => emit('close')

onMounted(async () => {
  await nextTick()
  clamp()
  window.addEventListener('pointerdown', onPointerDown, true)
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('blur', close)
  window.addEventListener('resize', close)
  window.addEventListener('wheel', close, { passive: true, capture: true })
})
onBeforeUnmount(() => {
  window.removeEventListener('pointerdown', onPointerDown, true)
  window.removeEventListener('keydown', onKey, true)
  window.removeEventListener('blur', close)
  window.removeEventListener('resize', close)
  window.removeEventListener('wheel', close, true)
})
watch(() => [props.x, props.y], async () => { pos.left = props.x; pos.top = props.y; await nextTick(); clamp() })

async function hover(index: number, item: MenuItem, event: MouseEvent) {
  if (item.separator) return
  if (!item.children && !item.load) { openIndex.value = -1; return }
  if (item.disabled) return
  const row = event.currentTarget as HTMLElement
  const rect = row.getBoundingClientRect()
  openIndex.value = index
  subPos.left = rect.right - 4
  subPos.top = rect.top - 5
  if (item.load && !loaded[index] && !loading[index]) {
    loading[index] = true
    try {
      loaded[index] = await item.load()
      delete loadError[index]
    } catch (error) {
      loadError[index] = error instanceof Error ? error.message : String(error)
    } finally {
      loading[index] = false
    }
  }
}

function children(index: number, item: MenuItem): MenuItem[] {
  if (item.children) return item.children
  if (loaded[index]) return loaded[index]
  if (loading[index]) return [{ label: '正在读取…', disabled: true }]
  if (loadError[index]) return [{ label: `无法读取：${loadError[index]}`, disabled: true }]
  return []
}

function choose(item: MenuItem) {
  if (item.disabled || item.separator || item.children || item.load) return
  emit('close')
  item.run?.()
}
</script>

<template>
  <Teleport to="body">
    <div ref="root" class="df-menu" role="menu" :style="{ left: pos.left + 'px', top: pos.top + 'px' }" @contextmenu.prevent>
      <template v-for="(item, index) in items" :key="index">
        <div v-if="item.separator" class="sep" role="separator" />
        <div
          v-else
          class="row"
          role="menuitem"
          :class="{ disabled: item.disabled, danger: item.danger, open: openIndex === index }"
          :aria-disabled="item.disabled || undefined"
          @mouseenter="hover(index, item, $event)"
          @click="choose(item)"
        >
          <span class="mark"><Icon v-if="item.checked" name="check" :size="12" /></span>
          <Icon v-if="item.icon" :name="item.icon" :size="13" class="lead" />
          <span class="label">{{ item.label }}</span>
          <span v-if="item.shortcut" class="shortcut">{{ item.shortcut }}</span>
          <Icon v-if="item.children || item.load" name="chevronRight" :size="11" class="more" />
        </div>
        <div
          v-if="openIndex === index && (item.children || item.load)"
          class="df-menu sub"
          role="menu"
          :style="{ left: subPos.left + 'px', top: subPos.top + 'px' }"
        >
          <template v-for="(child, ci) in children(index, item)" :key="ci">
            <div v-if="child.separator" class="sep" role="separator" />
            <div
              v-else
              class="row"
              role="menuitem"
              :class="{ disabled: child.disabled, danger: child.danger }"
              @click="choose(child)"
            >
              <span class="mark"><Icon v-if="child.checked" name="check" :size="12" /></span>
              <span class="label">{{ child.label }}</span>
            </div>
          </template>
        </div>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.df-menu {
  position: fixed;
  z-index: 1000;
  min-width: 200px;
  max-width: 320px;
  padding: 5px;
  border-radius: 10px;
  background: var(--surface, #fff);
  border: 0.5px solid var(--line, rgba(0, 0, 0, 0.12));
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.18), 0 2px 6px rgba(0, 0, 0, 0.08);
  color: var(--text, #1d1d1f);
  font-size: 13px;
  user-select: none;
}
.sub { z-index: 1001; }
.row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 26px;
  padding: 0 8px 0 4px;
  border-radius: 6px;
  cursor: default;
  white-space: nowrap;
}
.row:hover:not(.disabled), .row.open { background: var(--accent, #3276f3); color: #fff; }
.row.danger:not(.disabled):not(:hover) { color: var(--danger, #ff3b30); }
.row.disabled { color: var(--secondary, #6e6e73); opacity: 0.7; }
.mark { width: 14px; display: inline-flex; justify-content: center; flex: none; }
.lead { opacity: 0.85; }
.label { flex: 1; overflow: hidden; text-overflow: ellipsis; }
.shortcut { color: inherit; opacity: 0.55; font-size: 12px; }
.more { opacity: 0.7; }
.sep { height: 1px; margin: 5px 8px; background: var(--line, rgba(0, 0, 0, 0.1)); }
</style>
