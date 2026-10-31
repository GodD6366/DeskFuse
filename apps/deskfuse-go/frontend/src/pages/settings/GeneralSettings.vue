<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, type Component } from 'vue'
import { ArrowUpDown, Clipboard, Keyboard, Mic, Mouse, PanelsTopLeft, Settings as SettingsIcon } from 'lucide-vue-next'
import { backend } from '../../services/backend'
import AsyncToggle from './AsyncToggle.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSection from './SettingsSection.vue'
import { errorText, get, preference, type Snapshot } from './util'

const props = defineProps<{ snapshot: Snapshot }>()

interface Feature {
  id: string
  title: string
  beta?: boolean
  icon: Component
  enabled: boolean | undefined
  action: string
  params: (next: boolean) => Record<string, unknown>
}

// 与 Page.primary + Page.extended 的顺序、图标和 Beta 标识一致。
const features = computed<Feature[]>(() => {
  const values = get<Record<string, boolean>>(props.snapshot, 'features') ?? {}
  return [
    { id: 'clipboard', title: '剪贴板', icon: Clipboard },
    { id: 'input', title: '键鼠共享', icon: Keyboard },
    { id: 'files', title: '局域网传输', icon: ArrowUpDown },
    { id: 'remote_applications', title: '远程应用', beta: true, icon: PanelsTopLeft },
    { id: 'microphone', title: '麦克风共享', beta: true, icon: Mic },
    { id: 'mouse', title: '鼠标自定义', beta: true, icon: Mouse },
  ]
    .filter((feature) => values[feature.id] !== undefined)
    .map((feature) => ({
      ...feature,
      enabled: values[feature.id],
      action: 'feature.setEnabled',
      params: (enabled: boolean) => ({ feature: feature.id, enabled }),
    }))
})

const ready = computed(() => props.snapshot !== null)

// ---- 开机自启：以系统登录项为准，不另存偏好 ----
const loginStatus = ref<string | null>(null)
const loginError = ref<string | null>(null)
const loginDetail = computed(() => {
  switch (loginStatus.value) {
    case 'enabled': return '已开启，登录 Mac 后自动启动 DeskFuse。'
    case 'requires_approval': return '等待系统批准，请在系统设置的登录项中允许 DeskFuse。'
    case 'not_registered': return '登录 Mac 后自动启动 DeskFuse；更改立即生效。'
    case 'not_found': return '系统未找到登录项，请将 DeskFuse 安装到“应用程序”后重试。'
    case null: return loginError.value ? '无法读取开机自启状态。' : '正在读取开机自启状态…'
    default: return '无法确认登录项状态，请在系统设置中检查。'
  }
})
const loginOn = computed(() => loginStatus.value === 'enabled' || loginStatus.value === 'requires_approval')

async function refreshLogin() {
  try {
    loginStatus.value = (await backend.invoke('login_item.status')) as string
    loginError.value = null
  } catch (e) {
    loginError.value = errorText(e)
  }
}
async function loginApplied() { await refreshLogin() }
const loginOpenError = ref<string | null>(null)
async function openLoginSettings() {
  loginOpenError.value = null
  try { await backend.invoke('login_item.open_settings') } catch (e) { loginOpenError.value = errorText(e) }
}

onMounted(() => {
  refreshLogin()
  window.addEventListener('focus', refreshLogin)
})
onBeforeUnmount(() => window.removeEventListener('focus', refreshLogin))
</script>

<template>
  <div class="page">
    <SettingsSection title="功能" footer="关闭后停止相关后台任务；在功能页可随时重新启用。">
      <SettingsRow v-for="f in features" :key="f.id" :title="f.title" :badge="f.beta ? 'Beta' : undefined" :icon="f.icon">
        <AsyncToggle :value="f.enabled === true" :action="f.action" :params="f.params" :label="f.title" :disabled="!ready || f.enabled === undefined" />
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title="启动">
      <SettingsRow title="开机自启" :detail="loginDetail">
        <AsyncToggle :value="loginOn" action="login_item.set" :params="(enabled) => ({ enabled })" label="开机自启"
          :disabled="loginStatus === null" @applied="loginApplied" />
        <template #extra>
          <div v-if="loginStatus === 'requires_approval'" class="extra">
            <button type="button" class="button" @click="openLoginSettings"><SettingsIcon :size="13" />打开登录项设置</button>
          </div>
          <p v-if="loginError" class="err">{{ loginError }}</p>
          <p v-if="loginOpenError" class="err">{{ loginOpenError }}</p>
        </template>
      </SettingsRow>
      <SettingsRow title="隐藏 Dock 图标" detail="隐藏后只保留菜单栏图标；仍可从菜单栏打开主窗口和设置。">
        <AsyncToggle :value="preference(snapshot, 'hide_dock_icon') === true" action="settings.set"
          :params="(value) => ({ key: 'hide_dock_icon', value })" label="隐藏 Dock 图标" />
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title="保持可用">
      <SettingsRow title="保持唤醒（避免作为被控端时休眠离线）" detail="阻止系统和显示器自动休眠，退出 DeskFuse 后自动恢复。">
        <AsyncToggle :value="preference(snapshot, 'keep_awake') === true" action="settings.set"
          :params="(value) => ({ key: 'keep_awake', value })" label="保持唤醒" />
      </SettingsRow>
    </SettingsSection>
  </div>
</template>

<style scoped>
.page { display: grid; align-content: start; gap: 24px; }
.extra { margin-top: 6px; }
.err { margin: 4px 0 0; font-size: 12px; line-height: 1.5; color: var(--danger); }
</style>
