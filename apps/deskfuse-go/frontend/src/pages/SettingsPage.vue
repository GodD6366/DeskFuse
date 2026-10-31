<script setup lang="ts">
import { computed } from 'vue'
import AboutSettings from './settings/AboutSettings.vue'
import AdvancedSettings from './settings/AdvancedSettings.vue'
import ClipboardSettings from './settings/ClipboardSettings.vue'
import FileSettings from './settings/FileSettings.vue'
import GeneralSettings from './settings/GeneralSettings.vue'
import PermissionsSettings from './settings/PermissionsSettings.vue'
import ShortcutSettings from './settings/ShortcutSettings.vue'
import WorkflowSettings from './settings/WorkflowSettings.vue'
import { preference } from './settings/util'

const props = defineProps<{ category: string; snapshot: Record<string, any> | null }>()
const emit = defineEmits<{ navigate: [target: string] }>()

const CATEGORIES = ['general', 'clipboard', 'workflows', 'shortcuts', 'files', 'permissions', 'about', 'advanced'] as const
const TITLES: Record<string, string> = {
  general: '通用', clipboard: '剪贴板', workflows: '工作流', shortcuts: '快捷键',
  files: '文件', permissions: '权限', about: '关于与更新', advanced: '高级',
}
// 旧版深链接使用的分类名，继续指向合并后的新分类。
const LEGACY: Record<string, string> = { storage: 'files', updates: 'about' }

const workflowsEnabled = computed(() => preference(props.snapshot, 'workflows_enabled') === true)

// 工作流关闭后仍停在该分类时，回到剪贴板设置，而不是显示一个已经不存在的页面。
const selected = computed(() => {
  const name = LEGACY[props.category] ?? props.category
  const known = (CATEGORIES as readonly string[]).includes(name) ? name : 'general'
  return known === 'workflows' && !workflowsEnabled.value ? 'clipboard' : known
})

const view = computed(() => ({
  general: GeneralSettings, clipboard: ClipboardSettings, workflows: WorkflowSettings, shortcuts: ShortcutSettings,
  files: FileSettings, permissions: PermissionsSettings, about: AboutSettings, advanced: AdvancedSettings,
} as Record<string, any>)[selected.value])

// 工作流自带左右分栏；剪贴板/文件有底部保存栏；快捷键有固定页头。它们自己管理滚动与留白。
const bare = computed(() => ['workflows', 'clipboard', 'files', 'shortcuts'].includes(selected.value))
</script>

<template>
  <section class="settings-page" :aria-label="TITLES[selected]">
    <p v-if="!snapshot" class="offline" role="status">后端未连接：下列设置暂时只读，本地草稿仍可编辑，但不会保存。</p>
    <div class="body" :class="{ bare }">
      <KeepAlive>
        <component :is="view" :key="selected" :snapshot="snapshot" @navigate="emit('navigate', $event)" />
      </KeepAlive>
    </div>
  </section>
</template>

<style scoped>
.settings-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.offline { margin: 0; padding: 8px 26px; font-size: 12px; color: var(--secondary); border-bottom: 1px solid var(--line); }
/* 原生 DFScrollPage：内容左对齐、最大 720px，顶部与左右留白 24/4；顶栏缺的 8px 在页面里补上。 */
.body { flex: 1; min-height: 0; padding: 12px 24px 24px; overflow: auto; }
.body > :deep(*) { width: 100%; max-width: 720px; }
.body.bare { padding: 0; overflow: hidden; }
.body.bare > :deep(*) { max-width: none; }
</style>
