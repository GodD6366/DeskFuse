<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import Icon from './Icon.vue'
import Notice from './Notice.vue'
import { backend } from '../../services/backend'
import { invokeAction, useAction } from './core'
import type { PageKey } from './core'

// 对应 FeatureOnboardingView + FeatureGateView：功能总开关关闭时唯一的引导页。
// 演示动画（FeatureDemos）未迁移，用图标徽标代替。
const props = defineProps<{ page: Exclude<PageKey, 'devices' | 'settings'>; title: string }>()
const emit = defineEmits<{ (e: 'enabled'): void; (e: 'openSettings', tab?: string): void }>()

type Permission = 'accessibility' | 'screenRecording' | 'inputMonitoring'

const content: Record<string, { icon: string; message: string; bullets: string[]; permission: Permission | null; permissionTitle?: string; opensSettings?: boolean }> = {
  input: {
    icon: 'keyboard',
    message: '用一套键鼠控制多台电脑：把鼠标移出屏幕边缘，就进入相邻的设备。',
    bullets: ['鼠标穿越屏幕边缘切换设备，键盘随之跟随', '开启后此 Mac 可被主控控制；在键鼠所在的电脑上设为主控'],
    permission: 'accessibility',
    permissionTitle: '辅助功能',
  },
  clipboard: {
    icon: 'copy',
    message: '复制过的内容都会留下，并在已配对的电脑之间同步。',
    bullets: ['在一台电脑复制，另一台直接粘贴', '历史记录随时搜索、再次使用'],
    permission: null,
  },
  files: {
    icon: 'arrow-up',
    message: '用手机或电脑的浏览器，把图片和数据直接发到这台电脑，不经过云端。',
    bullets: ['手机扫码，或在其他电脑的浏览器里打开访问地址', '电脑之间也能拖入文件，发给在线的电脑'],
    permission: null,
  },
  remoteApplications: {
    icon: 'window',
    message: '在这台 Mac 上打开另一台电脑的应用窗口。',
    bullets: ['应用窗口像本地窗口一样显示与操作', 'Windows 远端仍在完善中，可在“设置 › 高级”中开启'],
    permission: 'screenRecording',
    permissionTitle: '屏幕录制',
    opensSettings: true,
  },
  microphone: {
    icon: 'mic',
    message: '把另一台 Mac 的麦克风当作这台 Mac 的输入设备。',
    bullets: ['应用读取麦克风时才传输，音频不会保存', '两台 Mac 需先在“设备”中完成配对'],
    permission: null,
  },
  mouse: {
    icon: 'mouse',
    message: '给鼠标侧键分配常用动作。',
    bullets: ['按一下侧键即可切换桌面等系统动作', '未适配的鼠标型号自动使用通用映射'],
    permission: 'inputMonitoring',
    permissionTitle: '输入监控',
  },
}

const featureKey: Record<string, string> = {
  input: 'input',
  clipboard: 'clipboard',
  files: 'files',
  mouse: 'mouse',
  microphone: 'microphone',
  remoteApplications: 'remote_applications',
}
const info = computed(() => content[props.page])
const { error, busy, run, retry, dismiss } = useAction()
const granted = ref(false)
const confirming = ref(false)

async function refreshPermission() {
  const permission = info.value.permission
  if (!permission) return
  // 查询失败时保持“未授权”：宁可多显示一行授权入口，也不假装已授权。
  const id = permission.replace(/[A-Z]/g, letter => `_${letter.toLowerCase()}`)
  try { const snapshot = await backend.snapshot(); granted.value = snapshot.permissions?.items?.find((item: any) => item.id === id)?.granted === true }
  catch { granted.value = false }
}

async function authorize() {
  const permission = info.value.permission
  if (permission) await run('permissions.open', { permission: permission.replace(/[A-Z]/g, letter => `_${letter.toLowerCase()}`), mode: 'authorize' }, { key: 'authorize' })
}

async function enable() {
  confirming.value = true
  const result = await run('feature.setEnabled', { feature: featureKey[props.page] ?? props.page, enabled: true })
  if (result.ok) emit('enabled')
  else confirming.value = false
}

onMounted(() => {
  void refreshPermission()
  window.addEventListener('focus', refreshPermission)
})
onBeforeUnmount(() => window.removeEventListener('focus', refreshPermission))
</script>

<template>
  <div class="gate">
    <div class="inner">
      <div class="badge" aria-hidden="true"><Icon :name="info.icon" :size="22" /></div>
      <h2 class="df-title">{{ title }}</h2>
      <p class="message df-detail df-secondary">{{ info.message }}</p>
      <ul class="bullets df-detail df-secondary">
        <li v-for="bullet in info.bullets" :key="bullet"><Icon name="check" :size="12" />{{ bullet }}</li>
      </ul>
      <div v-if="info.permission && !granted" class="permission">
        <span class="df-detail">{{ info.permissionTitle }}</span>
        <span class="df-spacer" />
        <button type="button" class="button" :disabled="busy.authorize" @click="authorize">
          <Icon name="external" :size="12" /> 前往授权
        </button>
      </div>
      <Notice v-if="error" tone="danger" title="操作未完成" :message="error.message" :raw="error.raw" action-title="重试" dismissible @action="retry" @dismiss="dismiss" />
      <button type="button" class="button primary" :disabled="confirming" @click="enable">
        <template v-if="confirming"><Icon name="check" :size="12" /> 已启用</template>
        <template v-else>启用{{ title }}</template>
      </button>
      <button v-if="info.opensSettings" type="button" class="df-link" @click="emit('openSettings', 'general')">打开设置</button>
    </div>
  </div>
</template>

<style scoped>
.gate { flex: 1; display: grid; place-items: center; padding: 28px; min-height: 0; }
.inner { width: min(520px, 100%); display: grid; justify-items: center; gap: 20px; text-align: center; }
.badge {
  width: 56px; height: 56px; display: grid; place-items: center; border-radius: 16px;
  color: var(--accent); background: var(--surface); border: 0.5px solid var(--line);
}
.message { margin: 0; }
.bullets { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
.bullets li { display: flex; align-items: center; gap: 8px; justify-content: center; }
.permission {
  display: flex; align-items: center; gap: 12px; width: min(340px, 100%); padding: 10px 14px;
  border-radius: 8px; background: var(--control);
}
.inner > :deep(.notice) { width: 100%; text-align: left; }
</style>
