<script setup lang="ts">
import { computed, ref } from 'vue'
import DevicesPage from './DevicesPage.vue'
import RemoteCatalogPane from './features/RemoteCatalogPane.vue'
import Icon from './features/Icon.vue'
import { camelize } from './features/core'
import { backend, errorMessage } from '../services/backend'

// 对应 DevicesView(showsApplications: true)：左侧仍是设备列表，右侧是
// 原生 applicationPane 的同一组空状态与远程应用目录。
const props = defineProps<{ snapshot: Record<string, any> | null }>()

const input = computed<Record<string, any>>(() => camelize(props.snapshot?.input ?? {}))
const canOperate = computed(() => input.value.canOperateRemoteApplications !== false)
// Windows 远程应用总开关在原生「设置 › 高级」；关闭时给出同款引导。
const windowsEnabled = computed(() => props.snapshot?.remote_applications?.windows_enabled === true)
const error = ref('')
async function enableWindows() {
  try { await backend.invoke('feature.setEnabled', { feature: 'windows_remote_applications', enabled: true }) }
  catch (e) { error.value = errorMessage(e) }
}
const restriction = computed(() =>
  String(
    input.value.remoteApplicationRestriction ??
      '当前键鼠共享角色不允许操作远程应用，请在主控电脑操作，或结束被控并关闭键鼠共享。',
  ),
)
</script>

<template>
  <DevicesPage :snapshot="snapshot" :shows-applications="true">
    <template #detail="{ device }">
      <div v-if="!canOperate" class="empty-state remote-empty">
        <Icon name="keyboard" :size="40" />
        <h2>当前无法操作远程应用</h2>
        <p>{{ restriction }}</p>
      </div>
      <div v-else-if="!device" class="empty-state remote-empty">
        <Icon name="grid" :size="40" />
        <h2>选择远端设备</h2>
        <p>从左侧选择已连接的配对设备，在这里浏览并打开它的应用。</p>
      </div>
      <div v-else-if="device.blocked" class="empty-state remote-empty">
        <Icon name="hand" :size="40" />
        <h2>设备已阻止</h2>
        <p>在设备页解除阻止并配对后，即可查看应用列表。</p>
      </div>
      <div v-else-if="!device.trusted" class="empty-state remote-empty">
        <Icon name="lock" :size="40" />
        <h2>请先配对设备</h2>
        <p>在设备页与 {{ device.name }} 完成配对后，即可打开远程应用。</p>
      </div>
      <div v-else-if="!device.connected" class="empty-state remote-empty">
        <Icon name="wifi-off" :size="40" />
        <h2>设备未连接</h2>
        <p>等待 {{ device.name }} 连接后，将自动加载应用列表。</p>
      </div>
      <div v-else-if="device.remote_app_rdp && !windowsEnabled" class="empty-state remote-empty">
        <Icon name="display" :size="40" />
        <h2>Windows 远程应用未开启</h2>
        <p>开启后即可在这台 Windows 设备上浏览并打开远程应用。</p>
        <p v-if="error" class="remote-error">{{ error }}</p>
        <button type="button" class="button primary" @click="enableWindows">开启</button>
      </div>
      <RemoteCatalogPane v-else :key="device.id" :device="device" />
    </template>
  </DevicesPage>
</template>

<style scoped>
.remote-error { font-size: 12px; color: var(--warning); }
.remote-empty {
  flex: 1 1 auto;
  align-self: stretch;
  min-height: 0;
  gap: 10px;
  padding: 32px;
}
.remote-empty h2 { font-size: 15px; }
.remote-empty p { font-size: 12px; }
</style>
