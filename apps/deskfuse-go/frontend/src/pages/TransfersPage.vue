<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onFileDrop } from 'mygo-runtime'
import FeatureGate from './features/FeatureGate.vue'
import Icon from './features/Icon.vue'
import Notice from './features/Notice.vue'
import Toggle from './features/Toggle.vue'
import { camelize, copyText, enumName, invokeAction, reportProblem, transferPhaseTitle, useAction, usePolling } from './features/core'

// 对应 FileTransferView：左列手机扫码与发送，右列传输任务。
const props = defineProps<{ snapshot: Record<string, any> | null }>()
const emit = defineEmits<{
  (e: 'navigate', page: string): void
  (e: 'retry'): void
  (e: 'openSettings', tab?: string): void
}>()

const { error, busy, run, retry, dismiss } = useAction()

const snap = computed<Record<string, any> | null>(() => (props.snapshot ? camelize(props.snapshot) : null))
const devices = computed<any[]>(() => snap.value?.devices ?? [])
const transfers = computed<any[]>(() => snap.value?.transfers ?? [])
// 注意：布尔 prop 缺失时 Vue 会注入 false，不能把它当作“未传值”的兜底；直接读快照。
const enabled = computed(() => snap.value?.features?.files ?? snap.value?.files?.enabled)

// 与原生 ByteCountFormatter(.file) 接近：1000 进制，小于 10 时保留两位、其余按需。
function byteCount(bytes: number | null | undefined): string {
  const value = Math.max(0, Number(bytes) || 0)
  if (value < 1000) return `${value} 字节`
  const units = ['KB', 'MB', 'GB', 'TB']
  let size = value / 1000
  let unit = 0
  while (size >= 1000 && unit < units.length - 1) { size /= 1000; unit += 1 }
  const text = size >= 100 ? size.toFixed(0) : size >= 10 ? size.toFixed(1) : size.toFixed(2).replace(/0$/, '').replace(/\.0$/, '')
  return `${text} ${units[unit]}`
}

// --- 局域网网页分享（LanWebSharing）---
interface UploadTask { id: string; name: string; state: string; receivedBytes: number; totalBytes?: number | null; clipboardWritten?: boolean | null; message: string }
interface LanState { enabled: boolean; running: boolean; addresses: string[]; error?: string | null; tasks: UploadTask[] }
const lan = ref<LanState | null>(null)
const lanError = ref<string | null>(null)
const configuring = ref(false)
const selectedAddress = ref('')
const addressCopied = ref(false)
// 二维码由原生侧用 CoreImage 生成；地址变化时重新取图，多地址切换也跟随。
const qrImage = ref('')
const actionError = ref<string | null>(null)

async function refreshLan() {
  if (!enabled.value) return
  const result = await invokeAction<LanState>('lanWeb.state')
  if (result.ok) {
    lan.value = camelize(result.value)
    lanError.value = null
  } else {
    lanError.value = result.error.message
  }
}
usePolling(refreshLan, 1000)

const liveAddress = computed(() => {
  if (lanError.value || !lan.value?.running) return null
  const addresses = lan.value.addresses ?? []
  return addresses.includes(selectedAddress.value) ? selectedAddress.value : (addresses[0] ?? null)
})
watch(liveAddress, () => (addressCopied.value = false))
// 多地址时下拉框必须有当前值，否则会渲染成一个空的选择框。
watch(() => lan.value?.addresses, (addresses) => {
  if (!addresses?.length) { selectedAddress.value = ''; return }
  if (!addresses.includes(selectedAddress.value)) selectedAddress.value = addresses[0]
}, { immediate: true })
watch(liveAddress, async (address) => {
  qrImage.value = ''
  if (!address) return
  const result = await invokeAction<{ image: string }>('lanWeb.qr', { address })
  if (result.ok) qrImage.value = result.value?.image ?? ''
}, { immediate: true })

async function configureLan(on: boolean) {
  configuring.value = true
  actionError.value = null
  const result = await invokeAction('lanWeb.configure', { enabled: on })
  configuring.value = false
  if (!result.ok) actionError.value = result.error.message
  else await refreshLan()
}
async function copyAddress(address: string) {
  addressCopied.value = await copyText(address)
  if (!addressCopied.value) actionError.value = '未能写入系统剪贴板，请选择地址后手动复制。'
}
// address 来自 lanWeb.state，是引擎生成的局域网地址（http://<ip>:<port>），宿主允许打开。
async function openAddress(address: string) {
  const result = await invokeAction('system.openUrl', { url: address })
  if (!result.ok) actionError.value = '无法打开浏览器，请复制地址后手动访问。'
}
const lanNotice = computed(() => actionError.value ?? lanError.value ?? lan.value?.error ?? null)

// --- 发送区 ---
interface Staged { path: string; name: string; size: number; isDirectory: boolean }
const staged = ref<Staged[]>([])
const target = ref('')
const dropTargeted = ref(false)
const dropError = ref<string | null>(null)

const connectedPeers = computed(() => devices.value.filter((d) => d.member && d.connected && d.trusted && !d.blocked))
watch(
  () => connectedPeers.value.map((d) => d.id).join('|'),
  () => {
    if (!connectedPeers.value.some((d) => d.id === target.value)) target.value = connectedPeers.value[0]?.id ?? ''
  },
  { immediate: true },
)
const totalSize = computed(() => staged.value.reduce((sum, f) => sum + f.size, 0))

function addStaged(items: Staged[]) {
  const known = new Set(staged.value.map((f) => f.path))
  staged.value = [...staged.value, ...items.filter((f) => !known.has(f.path))]
}
async function chooseFiles() {
  const result = await run<Staged[]>('transfers.chooseFiles')
  if (result.ok) addStaged(camelize<Staged[]>(result.value ?? []))
}
// MyGo 把 Finder 拖入的本地路径通过 mygo:file-drop 发给页面；DOM 的 File 对象
// 在 WKWebView 里没有 path。只处理落在发送区上的拖入。
let offFileDrop: (() => void) | null = null
onMounted(() => {
  offFileDrop = onFileDrop(async ({ paths, x, y }) => {
    const element = document.elementFromPoint(x, y)
    if (!element?.closest('.drop-wrap')) return
    dropTargeted.value = false
    dropError.value = null
    const result = await invokeAction<Staged[]>('transfers.inspectPaths', { paths })
    if (result.ok) addStaged(camelize<Staged[]>(result.value ?? []))
    else dropError.value = result.error.message
  })
})
onBeforeUnmount(() => { offFileDrop?.(); offFileDrop = null })
async function send() {
  if (!staged.value.length || !target.value) return
  const result = await run('transfers.send', { peer_id: target.value, paths: staged.value.map((f) => f.path) })
  if (result.ok) staged.value = []
}
async function addToShelf() {
  const result = await run('shelf.add', { paths: staged.value.map((f) => f.path) })
  if (result.ok) staged.value = []
}

// --- 传输队列 ---
const isActive = (t: any) => ['sending', 'receiving', 'verifying', 'delivering'].includes(enumName(t.phase))
const phaseOf = (t: any) => enumName(t.phase)
const progressBytes = (t: any) => (phaseOf(t) === 'delivering' ? t.saved : t.transferred)
const progressTotal = (t: any) => (phaseOf(t) === 'delivering' ? t.saveTotal : t.total)
const percent = (t: any) => (progressTotal(t) > 0 ? Math.floor(Math.min(1, progressBytes(t) / progressTotal(t)) * 100) : 0)
const tint = (t: any) => (phaseOf(t) === 'failed' ? 'df-tone-warning' : t.outgoing ? 'df-tone-accent' : 'df-tone-success')

const uploadTitle: Record<string, string> = { receiving: '正在接收', committing: '正在保存', done: '已保存', error: '接收失败', cancelled: '已取消' }
const uploadActive = (t: UploadTask) => t.state === 'receiving' || t.state === 'committing'
const uploads = computed(() => [...(lan.value?.tasks ?? [])].reverse())
</script>

<template>
  <div class="df-page transfers">
    <div v-if="!snap" class="boot">
      <div class="empty-state">
        <Icon name="alert" :size="40" />
        <h2 class="df-title">无法启动</h2>
        <p class="df-secondary">后台服务未就绪，请重新打开 DeskFuse。</p>
        <div class="df-row">
          <button type="button" class="button" @click="reportProblem('后台服务未就绪')">反馈问题…</button>
          <button type="button" class="button primary" @click="emit('retry')">重试</button>
        </div>
      </div>
    </div>

    <FeatureGate v-else-if="enabled === false" page="files" title="局域网传输" @open-settings="(tab) => emit('openSettings', tab)" />

    <template v-else>
      <div class="left df-scroll">
        <div class="stack">
          <Notice v-if="error" tone="danger" title="操作未完成" :message="error.message" :raw="error.raw" action-title="重试" dismissible @action="retry" @dismiss="dismiss" />
          <Notice v-if="lanNotice" tone="warning" title="局域网传输" :message="lanNotice" />

          <!-- 手机扫码传输 -->
          <section>
            <div class="df-section-label">手机扫码传输</div>
            <div class="card block">
              <div class="df-row">
                <div class="grow">
                  <div class="df-section">允许局域网访问</div>
                  <div class="df-detail df-secondary">同一局域网内的手机扫码即可发送照片或文字。</div>
                </div>
                <span v-if="configuring" class="spinner" role="status" aria-label="处理中" />
                <Toggle label="允许局域网访问" :model-value="lan?.enabled ?? false" :disabled="configuring || !lan" @update:model-value="configureLan" />
              </div>
              <hr class="df-divider" />
              <div v-if="liveAddress" class="df-row center">
                <img v-if="qrImage" class="qr" :src="qrImage" alt="用手机相机扫描二维码打开局域网传输网页" />
                <div v-else class="empty-state small qr-empty">
                  <Icon name="qr" :size="40" />
                  <div class="df-title">二维码不可用</div>
                  <div class="body-text df-secondary">仍可使用访问地址打开网页。</div>
                </div>
                <div class="grow address">
                  <div class="df-section">用手机相机扫码</div>
                  <div class="df-detail df-secondary">发送照片或文字，也可取用 Mac 列表内容。</div>
                  <select v-if="(lan?.addresses.length ?? 0) > 1" v-model="selectedAddress" class="df-select" aria-label="访问地址">
                    <option v-for="address in lan?.addresses" :key="address" :value="address">{{ address }}</option>
                  </select>
                  <span v-else class="df-mono sel">{{ liveAddress }}</span>
                  <div class="df-row">
                    <button type="button" class="button" @click="copyAddress(liveAddress)"><Icon name="copy" :size="14" /> {{ addressCopied ? '已复制地址' : '复制地址' }}</button>
                    <button type="button" class="button" @click="openAddress(liveAddress)"><Icon name="external" :size="14" /> 打开网页</button>
                  </div>
                </div>
              </div>
              <div v-else class="empty-state small">
                <Icon name="qr" :size="40" />
                <h3 class="df-title">{{ lan?.enabled ? '等待可访问地址' : '开启后扫码传输' }}</h3>
                <p class="body-text df-secondary">{{ lan?.enabled ? '服务成功监听局域网后，二维码和地址会显示在这里。' : '开启开关后，二维码和访问地址会显示在这里。' }}</p>
                <button v-if="!lan?.enabled" type="button" class="button primary" :disabled="configuring" @click="configureLan(true)">开启</button>
              </div>
            </div>
            <p class="df-detail df-secondary foot">离开此页仍保持可用；重启后自动恢复上次的开关。</p>
          </section>

          <!-- 发送文件 -->
          <section class="send">
            <div class="df-row label-row">
              <div class="df-section-label nomargin">发送文件</div>
              <span class="df-spacer" />
              <span class="df-caption df-secondary">也可拖到菜单栏图标，或按 ⇧⌘D 呼出暂存区</span>
            </div>
            <div class="card send-card">
              <div class="drop-wrap" @dragover.prevent="dropTargeted = true" @dragleave="dropTargeted = false">
                <button v-if="staged.length === 0" type="button" class="drop" :class="{ targeted: dropTargeted }" title="选择要发送的文件" @click="chooseFiles">
                  <Icon name="tray" :size="40" :class="dropTargeted ? 'df-tone-accent' : 'df-secondary'" />
                  <span class="df-section">{{ dropTargeted ? '松开即可添加' : '拖入文件或文件夹' }}</span>
                  <span class="df-detail df-secondary">或点击选择 · 只会发送给下方选中的在线电脑</span>
                </button>
                <div v-else class="staged" :class="{ targeted: dropTargeted }">
                  <div class="df-row">
                    <span class="df-section">{{ staged.length }} 个项目 · {{ byteCount(totalSize) }}</span>
                    <span class="df-spacer" />
                    <button type="button" class="df-link" @click="chooseFiles"><Icon name="plus" :size="12" /> 添加</button>
                    <button type="button" class="df-link" @click="staged = []">清空</button>
                  </div>
                  <ul class="files">
                    <li v-for="file in staged" :key="file.path" class="df-row file">
                      <Icon :name="file.isDirectory ? 'tray' : 'app'" :size="24" class="df-secondary" />
                      <span class="df-ellipsis grow" :title="file.path">{{ file.name }}</span>
                      <span class="df-detail df-secondary num">{{ file.isDirectory ? '文件夹' : byteCount(file.size) }}</span>
                      <button type="button" class="df-hit df-secondary" :aria-label="`移除 ${file.name}`" title="移除" @click="staged = staged.filter((f) => f.path !== file.path)"><Icon name="x-circle" :size="14" /></button>
                    </li>
                  </ul>
                </div>
                <p v-if="dropError" class="df-caption df-tone-warning drop-error">{{ dropError }}</p>
              </div>
              <hr class="df-divider" />
              <div class="df-row bar">
                <template v-if="connectedPeers.length === 0">
                  <span class="df-detail df-secondary">还没有已连接的电脑</span>
                  <button type="button" class="df-link" @click="emit('navigate', 'devices')">去添加</button>
                </template>
                <template v-else>
                  <span class="df-detail df-secondary">发送到</span>
                  <select v-model="target" class="df-select" aria-label="发送到">
                    <option v-for="device in connectedPeers" :key="device.id" :value="device.id">{{ device.name }}</option>
                  </select>
                </template>
                <span class="df-spacer" />
                <button v-if="staged.length" type="button" class="button" :disabled="busy['shelf.add']" @click="addToShelf">加入暂存区</button>
                <button type="button" class="button primary" :disabled="!staged.length || !target || busy['transfers.send']" @click="send">
                  {{ staged.length ? `发送 ${staged.length} 个项目` : '发送' }}
                </button>
              </div>
            </div>
          </section>
        </div>
      </div>

      <aside class="right df-scroll">
        <div class="stack wide">
          <section>
            <div class="df-section-label">传输任务</div>
            <div class="card block">
              <div v-if="transfers.length === 0" class="empty-state">
                <svg class="hero" width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <path d="m21 16-4 4-4-4" /><path d="M17 20V4" /><path d="m3 8 4-4 4 4" /><path d="M7 4v16" />
                </svg>
                <h3 class="df-title">还没有传输任务</h3>
                <p class="body-text df-secondary">选择文件并发送后，进度会显示在这里。</p>
                <button type="button" class="button primary" @click="chooseFiles">选择文件发送</button>
              </div>
              <template v-else>
                <template v-for="(task, index) in transfers" :key="task.id">
                  <hr v-if="index" class="df-divider" />
                  <div class="task">
                    <span class="tile" :class="tint(task)" :title="task.outgoing ? '发送' : '接收'"><Icon :name="task.outgoing ? 'arrow-up' : 'arrow-down'" :size="24" /></span>
                    <div class="task-body">
                      <div class="df-ellipsis2 name">{{ task.name }}</div>
                      <div class="df-detail df-secondary">{{ task.detail }}</div>

                      <div v-if="phaseOf(task) === 'verifying' || phaseOf(task) === 'queued'" class="df-row df-detail df-secondary"><span class="spinner" /> {{ transferPhaseTitle(task) }}</div>
                      <div v-else-if="isActive(task)" class="prog">
                        <template v-if="progressTotal(task) > 0">
                          <div class="df-row"><progress :value="percent(task)" max="100" /><span class="df-detail df-secondary num">{{ percent(task) }}%</span></div>
                          <div class="df-detail df-secondary num">{{ byteCount(progressBytes(task)) }} / {{ byteCount(progressTotal(task)) }}</div>
                        </template>
                        <div v-else class="df-row df-detail df-secondary"><span class="spinner" /> 已传输 {{ byteCount(progressBytes(task)) }}</div>
                      </div>
                      <div v-else-if="phaseOf(task) === 'completed'" class="df-detail df-secondary df-row"><Icon name="check-circle" :size="14" /> 已完成</div>
                      <div v-else-if="phaseOf(task) === 'cancelled'" class="df-detail df-secondary">已取消</div>
                      <div v-else-if="phaseOf(task) === 'failed'" class="df-detail df-tone-warning">失败</div>

                      <div>
                        <template v-if="phaseOf(task) === 'failed' || phaseOf(task) === 'cancelled'">
                          <button v-if="task.canRetry" type="button" class="df-link" @click="run('transfers.retry', { id: task.id })">重试</button>
                        </template>
                        <button v-else-if="phaseOf(task) !== 'completed'" type="button" class="df-link" @click="run('transfers.cancel', { id: task.id })">取消</button>
                      </div>
                    </div>
                  </div>
                </template>
              </template>
            </div>
            <p v-if="transfers.length === 0" class="df-detail df-secondary foot">在另一台电脑粘贴文件时会自动开始传输，也可以从暂存区立即发送。</p>
          </section>

          <section v-if="uploads.length">
            <div class="df-section-label">手机发送记录</div>
            <div class="card block">
              <template v-for="(task, index) in uploads" :key="task.id">
                <hr v-if="index" class="df-divider" />
                <div class="upload">
                  <div class="df-row top">
                    <span class="df-ellipsis2 name grow">{{ task.name }}</span>
                    <span class="df-detail" :class="task.state === 'error' ? 'df-tone-warning' : 'df-secondary'">{{ uploadTitle[task.state] ?? task.state }}</span>
                  </div>
                  <template v-if="uploadActive(task)">
                    <template v-if="task.state === 'receiving' && task.totalBytes && task.totalBytes > 0">
                      <progress :value="Math.min(task.receivedBytes, task.totalBytes)" :max="task.totalBytes" />
                      <div class="df-detail df-secondary num">{{ byteCount(task.receivedBytes) }} / {{ byteCount(task.totalBytes) }}</div>
                    </template>
                    <div v-else class="df-row df-detail df-secondary"><span class="spinner" /> {{ task.state === 'committing' ? '正在保存到列表…' : `已接收 ${byteCount(task.receivedBytes)}` }}</div>
                  </template>
                  <div v-if="task.message" class="df-detail df-secondary sel">{{ task.message }}</div>
                  <div v-if="task.clipboardWritten !== null && task.clipboardWritten !== undefined" class="df-detail df-row" :class="task.clipboardWritten ? 'df-tone-success' : 'df-secondary'">
                    <Icon :name="task.clipboardWritten ? 'check-circle' : 'copy'" :size="14" /> {{ task.clipboardWritten ? '已写入 Mac 剪贴板' : '未覆盖 Mac 当前剪贴板' }}
                  </div>
                </div>
              </template>
            </div>
          </section>
        </div>
      </aside>
    </template>
  </div>
</template>

<style scoped>
.transfers { flex-direction: row; }
.boot { flex: 1; display: grid; place-items: center; }
.left { flex: 1 1 0; min-width: 0; display: flex; }
.right { flex: 0 0 min(380px, 38%); border-left: 0.5px solid var(--line); }
.stack { padding: 24px 16px 16px; display: flex; flex-direction: column; gap: 16px; width: 100%; min-height: 100%; }
.stack.wide { gap: 24px; min-height: 0; }
.left .stack > .send { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.block { padding: 16px; display: grid; gap: 8px; }
.card .df-section { font-weight: 500; line-height: 16px; }
.transfers .df-detail { line-height: 14px; }
.transfers .df-caption { line-height: 13px; }
.transfers .df-section-label { line-height: 16px; }
.transfers .card { border: 0; box-shadow: inset 0 0 0 0.5px var(--line); }
.transfers .button.primary:disabled { background: var(--control); color: var(--secondary); border-color: transparent; opacity: 1; }
.grow { flex: 1; min-width: 0; }
.top { align-items: flex-start; }
.center { align-items: center; }
.body-text { font-size: 13px; line-height: 16px; }
.foot { margin: 8px 4px 0; }
.label-row { align-items: baseline; min-height: 0; margin-bottom: 8px; gap: 8px; }
.nomargin { margin-bottom: 0; }
.num { font-variant-numeric: tabular-nums; }
.sel { user-select: text; overflow-wrap: anywhere; }
.empty-state { display: grid; justify-items: center; gap: 10px; padding: 32px; text-align: center; color: var(--secondary); }
.right .empty-state { min-height: 0; padding: 24px 16px; gap: 12px; }
.empty-state.small { min-height: 0; padding: 16px 8px; }
.empty-state .df-title { color: var(--text); margin: 0; }
.empty-state p { margin: 0; max-width: 360px; }
.qr { width: 144px; height: 144px; padding: 8px; border-radius: 12px; background: #fff; image-rendering: pixelated; }
.empty-state.qr-empty { width: 160px; height: 160px; min-height: 0; padding: 0; gap: 6px; align-content: center; }
.address { display: grid; gap: 8px; justify-items: start; }
.spinner { display: inline-block; width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
:deep(.df-switch) { width: 40px; }
:deep(.df-switch:checked)::after { transform: translateX(20px); }
@keyframes spin { to { transform: rotate(360deg); } }

.send-card { flex: 1; display: flex; flex-direction: column; min-height: 280px; }
.drop-wrap { flex: 1; padding: 12px; display: flex; flex-direction: column; min-height: 180px; gap: 6px; }
.drop {
  flex: 1; display: grid; place-content: center; justify-items: center; gap: 8px; padding: 16px;
  border: 1px dashed var(--line); border-radius: 12px; background: var(--control); color: inherit; font: inherit; cursor: pointer;
}
.drop:hover { background: color-mix(in srgb, var(--text) 3%, var(--control)); }
.drop.targeted, .staged.targeted { border: 1.5px dashed var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); }
.staged { flex: 1; display: flex; flex-direction: column; gap: 8px; border-radius: 8px; min-height: 0; }
.files { list-style: none; margin: 0; padding: 0; overflow: auto; flex: 1; }
.file { padding: 6px 0; border-top: 0.5px solid var(--line); gap: 8px; }
.file:first-child { border-top: 0; }
.drop-error { margin: 0; }
.bar { padding: 12px 16px; }

.task { display: flex; gap: 16px; align-items: flex-start; padding: 12px 0; }
.tile { width: 40px; height: 40px; flex: none; display: grid; place-items: center; border-radius: 8px; background: color-mix(in srgb, currentColor 12%, transparent); }
.task-body { flex: 1; min-width: 0; display: grid; gap: 4px; justify-items: start; }
.task-body > * { max-width: 100%; }
.prog { display: grid; gap: 4px; width: 100%; }
.name { font-weight: 500; }
.df-ellipsis2 { overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; word-break: break-all; }
progress { width: 100%; height: 4px; accent-color: var(--accent); }
.upload { padding: 4px 0; display: grid; gap: 4px; }
</style>
