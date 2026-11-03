<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Icon from './Icon.vue'
import { backend, errorMessage } from '../../services/backend'

type AppEntry = { name?: string; display_name?: string; path?: string }
const props = defineProps<{ snapshot: Record<string, any> | null }>()
const registered = ref<AppEntry[]>([])
const installed = ref<AppEntry[]>([])
const host = ref<Record<string, any> | null>(null)
const available = ref(true)
const reason = ref('')
const loading = ref(false)
const status = ref('')
const name = ref('')
const displayName = ref('')
const path = ref('')
const selected = ref<AppEntry | null>(null)
const peer = ref<Record<string, any> | null>(null)
const peerApps = ref<AppEntry[]>([])
const peerLoading = ref(false)
const peerError = ref('')
const peerOpening = ref<string | null>(null)
const remotePeers = computed(() => (props.snapshot?.devices ?? []).filter((device: any) =>
  device.remote_app_rdp === true && device.trusted === true && device.blocked !== true && device.connected === true,
))

function label(app: AppEntry): string {
  return `${app.display_name || app.name || ''}${app.name && app.display_name && app.name !== app.display_name ? ` · ${app.name}` : ''}`
}
function selectInstalled(app: AppEntry) {
  name.value = String(app.name ?? '')
  displayName.value = String(app.display_name ?? app.name ?? '')
  path.value = String(app.path ?? '')
}
async function refresh() {
  loading.value = true
  status.value = ''
  try {
    const [catalog, programs] = await Promise.all([
      backend.invoke('remoteApps.hostCatalog'),
      backend.invoke('remoteApps.hostInstalled'),
    ])
    available.value = catalog?.available !== false
    reason.value = String(catalog?.reason ?? '')
    host.value = catalog?.host && typeof catalog.host === 'object' ? catalog.host : null
    registered.value = Array.isArray(catalog?.applications) ? catalog.applications : []
    installed.value = Array.isArray(programs?.applications) ? programs.applications : []
  } catch (cause) {
    status.value = errorMessage(cause)
  } finally {
    loading.value = false
  }
}
async function registerApp() {
  const appName = name.value.trim()
  const executable = path.value.trim()
  if (!appName || !executable) { status.value = '注册名和可执行文件路径不能为空。'; return }
  try {
    await backend.invoke('remoteApps.hostRegister', {
      name: appName,
      path: executable,
      display_name: displayName.value.trim() || appName,
    })
    status.value = '已提交注册，正在刷新实际状态。'
    await refresh()
  } catch (cause) { status.value = errorMessage(cause) }
}
async function unregisterApp() {
  const app = selected.value
  if (!app?.name) return
  try {
    await backend.invoke('remoteApps.hostUnregister', { name: app.name })
    selected.value = null
    status.value = '已提交移除，正在刷新实际状态。'
    await refresh()
  } catch (cause) { status.value = errorMessage(cause) }
}
async function loadPeer(device: Record<string, any>) {
  peer.value = device
  peerApps.value = []
  peerError.value = ''
  peerLoading.value = true
  try {
    const result = await backend.invoke('remoteApps.peerCatalog', { peer_id: device.id })
    if (result?.type === 'error') throw new Error(String(result.message ?? '远程应用目录不可用'))
    peerApps.value = Array.isArray(result?.apps) ? result.apps : []
  } catch (cause) { peerError.value = errorMessage(cause) }
  finally { peerLoading.value = false }
}
async function openPeerApp(app: AppEntry) {
  if (!peer.value?.id || !app.name) return
  peerOpening.value = app.name
  peerError.value = ''
  try { await backend.invoke('remoteApps.peerOpen', { peer_id: peer.value.id, name: app.name }) }
  catch (cause) { peerError.value = errorMessage(cause) }
  finally { peerOpening.value = null }
}
onMounted(() => { void refresh() })
</script>

<template>
  <section class="host-page">
    <header class="host-header">
      <div class="host-icon"><Icon name="display" :size="22" /></div>
      <div>
        <h2>共享的远程应用</h2>
        <p>管理这台 Windows 共享的应用，其他已配对设备可以连接使用。</p>
      </div>
      <button class="button" type="button" :disabled="loading" @click="refresh"><Icon name="refresh" :size="14" />刷新</button>
    </header>

    <section v-if="remotePeers.length" class="host-section">
      <h3>打开已配对 Windows 设备上的应用</h3>
      <div class="peer-list">
        <button v-for="device in remotePeers" :key="device.id" type="button" :class="['peer-row', { selected: peer?.id === device.id }]" @click="loadPeer(device)">
          <strong>{{ device.name || device.id }}</strong><span>{{ device.id }}</span>
        </button>
      </div>
      <p v-if="peerLoading" class="muted">正在读取 {{ peer?.name || '设备' }} 的应用目录…</p>
      <p v-else-if="peer && !peerApps.length" class="empty">这台设备没有可用的 RemoteApp。</p>
      <div v-else class="peer-apps">
        <button v-for="app in peerApps" :key="app.name" type="button" :disabled="peerOpening === app.name" @click="openPeerApp(app)">
          <strong>{{ label(app) }}</strong><span>{{ app.path }}</span>
        </button>
      </div>
      <p v-if="peerError" class="warning">{{ peerError }}</p>
    </section>

    <section class="host-section">
      <h3>服务端预检</h3>
      <p v-if="!host" class="muted">服务正在完成首次预检…</p>
      <template v-else>
        <p v-for="check in (host.checks ?? [])" :key="check.name" :class="['check', check.passed ? 'passed' : 'failed']">
          <Icon :name="check.passed ? 'check' : 'alert'" :size="13" />{{ check.message }}
        </p>
        <p v-if="host.port" class="muted">RDP 端口 {{ host.port }}<span v-if="host.certificate_sha256"> · 监听证书 {{ host.certificate_sha256 }}</span></p>
      </template>
      <p v-if="!available && reason" class="warning">{{ reason }}</p>
      <p v-else-if="available && host" class="muted">预检通过后，已配对设备可以连接。</p>
    </section>

    <section class="host-section">
      <h3>已注册 RemoteApp</h3>
      <div v-if="!registered.length" class="empty">还没有注册应用。注册后，已配对设备会在远程应用列表中看到它。</div>
      <div v-else class="app-list">
        <button v-for="app in registered" :key="app.name" type="button" :class="['app-row', { selected: selected?.name === app.name }]" @click="selected = app">
          <strong>{{ label(app) }}</strong><span>{{ app.path }}</span>
        </button>
      </div>
      <button class="button danger" type="button" :disabled="!selected" @click="unregisterApp">移除选中</button>
    </section>

    <section class="host-section form-section">
      <h3>注册应用</h3>
      <label>注册名（英文）<input v-model="name" autocomplete="off" /></label>
      <label>显示名称<input v-model="displayName" autocomplete="off" /></label>
      <label>可执行文件完整路径<input v-model="path" autocomplete="off" placeholder="C:\\Program Files\\…\\app.exe" /></label>
      <div class="installed-head"><span>已安装程序</span><span class="muted">点击填入表单</span></div>
      <div class="installed-list">
        <button v-for="app in installed" :key="`${app.name}:${app.path}`" type="button" @click="selectInstalled(app)">{{ label(app) }}<small>{{ app.path }}</small></button>
        <p v-if="!installed.length" class="empty">没有可选的已安装程序。</p>
      </div>
      <button class="button primary" type="button" :disabled="loading" @click="registerApp">注册 / 更新</button>
      <p v-if="status" class="status">{{ status }}</p>
    </section>
  </section>
</template>

<style scoped>
.host-page { width: min(760px, 100%); margin: 0 auto; padding: 24px; display: grid; gap: 18px; overflow: auto; }
.host-header { display: grid; grid-template-columns: auto 1fr auto; gap: 12px; align-items: center; }
.host-icon { width: 42px; height: 42px; border-radius: 10px; display: grid; place-items: center; color: var(--accent); background: var(--accent-subtle); }
h2, h3, p { margin: 0; } h2 { font-size: 17px; } h3 { font-size: 13px; margin-bottom: 10px; }
.host-header p, .muted, .empty, .status { color: var(--text-secondary); font-size: 12px; line-height: 1.5; }
.host-section { display: grid; gap: 8px; padding: 16px; border: 1px solid var(--line); border-radius: 12px; background: var(--surface); }
.check { display: flex; gap: 7px; align-items: center; font-size: 12px; } .check.passed { color: var(--success); } .check.failed, .warning { color: var(--warning); }
.app-list, .installed-list { display: grid; gap: 5px; max-height: 190px; overflow: auto; }
.peer-list, .peer-apps { display: grid; gap: 5px; }
.peer-row, .peer-apps button { display: grid; gap: 2px; text-align: left; padding: 9px 10px; border: 1px solid transparent; border-radius: 8px; color: inherit; background: transparent; cursor: pointer; }
.peer-row:hover, .peer-row.selected, .peer-apps button:hover { background: var(--control); border-color: var(--line); }
.peer-row span, .peer-apps span { color: var(--text-secondary); font-size: 11px; overflow-wrap: anywhere; }
.app-row, .installed-list button { display: grid; gap: 2px; text-align: left; padding: 9px 10px; border: 1px solid transparent; border-radius: 8px; color: inherit; background: transparent; cursor: pointer; }
.app-row:hover, .installed-list button:hover, .app-row.selected { background: var(--control); border-color: var(--line); }
.app-row span, .installed-list small { color: var(--text-secondary); font-size: 11px; overflow-wrap: anywhere; }
.danger { color: var(--danger); justify-self: start; } .form-section label { display: grid; gap: 4px; font-size: 12px; color: var(--text-secondary); }
input { width: 100%; box-sizing: border-box; padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; color: inherit; background: var(--control); }
.installed-head { display: flex; justify-content: space-between; font-size: 12px; margin-top: 4px; }
.status { color: var(--warning); }
</style>
