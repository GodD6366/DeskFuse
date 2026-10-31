<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import ConfirmDialog from './features/ConfirmDialog.vue'
import DeviceIcon from './features/DeviceIcon.vue'
import DeviceRoutesPanel from './features/DeviceRoutesPanel.vue'
import Icon from './features/Icon.vue'
import Notice from './features/Notice.vue'
import { camelize, enumName, reportProblem, transferPhaseTitle, useAction } from './features/core'

// 对应 DevicesView。showsApplications 为 true 时（远程应用页）右侧改为 #detail 插槽，
// 左侧列表仍是同一份设备列表。
const props = withDefaults(
  defineProps<{
    snapshot: Record<string, any> | null
    searchRequest?: number
    localSelectionRequest?: number
    showsApplications?: boolean
  }>(),
  { searchRequest: 0, localSelectionRequest: 0, showsApplications: false },
)
const emit = defineEmits<{
  (e: 'navigate', page: string): void
  (e: 'retry'): void
}>()

const { error, busy, run, retry, dismiss } = useAction()

const snap = computed<Record<string, any> | null>(() => (props.snapshot ? camelize(props.snapshot) : null))
const devices = computed<any[]>(() => snap.value?.devices ?? [])
const pairing = computed<any[]>(() => snap.value?.pairing ?? [])
const transfers = computed<any[]>(() => snap.value?.transfers ?? [])
const local = computed<Record<string, any>>(() => snap.value?.local ?? {})
const localId = computed<string>(() => local.value.id ?? '')
const localName = computed<string>(() => local.value.name ?? '这台电脑')
const input = computed<Record<string, any>>(() => snap.value?.input ?? {})

const selected = ref<string | null>(null)
const search = ref('')
const searchBox = ref<HTMLInputElement | null>(null)
const searchFocused = ref(false)

const query = computed(() => search.value.trim().toLowerCase())
const has = (text: unknown) => String(text ?? '').toLowerCase().includes(query.value)

const matches = computed(() =>
  devices.value
    .filter(
      (d) =>
        !query.value ||
        has(d.name) ||
        has(d.endpoint) ||
        has(d.detail) ||
        (d.routes ?? []).some((r: any) => has(r.endpoint)),
    )
    .sort((a, b) => {
      if (a.connected !== b.connected) return a.connected ? -1 : 1
      return String(a.name).localeCompare(String(b.name), 'zh-CN', { numeric: true })
    }),
)
const paired = computed(() => matches.value.filter((d) => d.trusted && !d.blocked))
const nearby = computed(() => matches.value.filter((d) => !d.trusted && !d.blocked))
const blocked = computed(() => matches.value.filter((d) => d.blocked))
const batchUpdateCandidates = computed(() => paired.value.filter((d) => d.connected && d.canTriggerUpdate))
const localMatches = computed(
  () => !query.value || has(localName.value) || '这台 mac'.includes(query.value) || '这台电脑'.includes(query.value),
)
const sections = computed(() => [
  { title: '已配对', items: paired.value },
  { title: '附近', items: nearby.value },
  { title: '已阻止', items: blocked.value },
])
const noRemote = computed(() => !paired.value.length && !nearby.value.length && !blocked.value.length)

// 原生 List 支持方向键在可见行之间移动；搜索后的可见顺序与渲染顺序一致。
const visibleDeviceIDs = computed(() => sections.value.flatMap((section) => section.items.map((item: any) => item.id)))
function moveSelection(step: number, current?: string) {
  const ids = visibleDeviceIDs.value
  if (!ids.length) return
  const index = current ? ids.indexOf(current) : ids.findIndex((id: string) => id === selected.value)
  const next = Math.max(0, Math.min(ids.length - 1, (index < 0 ? 0 : index) + step))
  selected.value = ids[next]
  requestAnimationFrame(() => {
    ;(document.querySelector(`.row[data-device-id="${ids[next]}"]`) as HTMLElement | null)?.focus()
  })
}
const selectedDevice = computed(() => devices.value.find((d) => d.id === selected.value) ?? null)
const canOperateRemote = computed(() => input.value.canOperateRemoteApplications !== false)

// 选中项被移除时回落：已配对 → 附近 → 已阻止 → 本机。
watch(
  () => [devices.value.map((d) => d.id).join('|'), localId.value],
  () => {
    if (selected.value === localId.value && localId.value) return
    if (selected.value && devices.value.some((d) => d.id === selected.value)) return
    selected.value = paired.value[0]?.id ?? nearby.value[0]?.id ?? blocked.value[0]?.id ?? localId.value ?? null
  },
  { immediate: true },
)
watch(() => props.searchRequest, async () => {
  await nextTick()
  searchBox.value?.focus()
})
watch(() => props.localSelectionRequest, () => {
  selected.value = localId.value
})

function statusLabel(d: any): string {
  if (d.connected) return '在线'
  if (d.blocked) return '已阻止'
  return d.linkStatus ? d.linkStatus : '离线'
}
const pathLabels: Record<string, string> = { auto: '自动', thunderbolt: '雷电', ethernet: '有线', wifi: 'Wi-Fi' }
function listDetail(d: any): string {
  const status = statusLabel(d)
  if (d.inputPathPaused) return `${status} · 等待${pathLabels[enumName(d.inputPath)] ?? '指定线路'}，键鼠已暂停`
  const endpoint = d.endpoint || d.detail
  return endpoint ? `${status} · ${endpoint}` : status
}

// --- 搜索 ---
function clearSearch() {
  search.value = ''
}
function onSearchEsc() {
  search.value = ''
  searchBox.value?.blur()
}

// --- 手动 IPv4 配对 ---
const showManual = ref(false)
const ip = ref('')
const manualRoot = ref<HTMLElement | null>(null)
function closeOutside(event: MouseEvent) {
  if (manualRoot.value && !manualRoot.value.contains(event.target as Node)) showManual.value = false
}
watch(showManual, (open) => {
  if (open) document.addEventListener('mousedown', closeOutside)
  else document.removeEventListener('mousedown', closeOutside)
})
onBeforeUnmount(() => document.removeEventListener('mousedown', closeOutside))
async function pairByIp() {
  const value = ip.value.trim()
  if (!value) return
  const result = await run('devices.pair', { ip: value })
  if (result.ok) {
    ip.value = ''
    showManual.value = false
  }
}

// --- 入站配对 ---
const deciding = ref<Set<string>>(new Set())
async function decide(request: any, allow: boolean) {
  deciding.value = new Set(deciding.value).add(request.id)
  const result = await run(allow ? 'devices.accept' : 'devices.reject', { request_id: request.id })
  if (!result.ok) {
    const next = new Set(deciding.value)
    next.delete(request.id)
    deciding.value = next
  }
}
watch(pairing, (requests) => {
  // 请求从快照消失即视为已处理。
  const alive = new Set(requests.map((r) => r.id))
  const next = new Set([...deciding.value].filter((id) => alive.has(id)))
  if (next.size !== deciding.value.size) deciding.value = next
})

// --- 确认对话框 ---
type Confirm = { kind: 'forget' | 'block' | 'leave' | 'batch'; ids: string[] }
const confirm = ref<Confirm | null>(null)
const confirmCopy = computed(() => {
  switch (confirm.value?.kind) {
    case 'forget':
      return { title: '忘记此设备并撤销信任？', message: '这会断开连接并移除配对关系。设备仍可被发现和重新配对，不会被拉黑。', action: '忘记设备', destructive: true }
    case 'block':
      return { title: '拉黑此设备？', message: '这会断开连接并阻止该设备配对，直到你允许重新配对。', action: '拉黑设备', destructive: true }
    case 'leave':
      return { title: '退出当前设备网络？', message: '这会解除本机与所有已配对设备的网络关系，不是断开某一台设备。', action: '退出网络', destructive: true }
    case 'batch':
      return {
        title: '批量触发设备更新？',
        message: '将请求当前在线且支持主动更新的配对设备开始各自的更新流程。每台设备仍按本地版本和安全条件执行。',
        action: `触发 ${confirm.value.ids.length} 台设备`,
        destructive: false,
      }
    default:
      return { title: '', message: '', action: '确定', destructive: false }
  }
})
async function confirmed() {
  const current = confirm.value
  confirm.value = null
  if (!current) return
  if (current.kind === 'forget') await run('devices.forget', { peer_id: current.ids[0], confirm: true })
  else if (current.kind === 'block') await run('devices.block', { peer_id: current.ids[0], confirm: true })
  else if (current.kind === 'leave') await run('devices.leaveNetwork', { confirm: true })
  else await run('devices.triggerUpdates', { peer_ids: current.ids })
}
function askBatchUpdate() {
  const ids = batchUpdateCandidates.value.map((d) => d.id)
  if (ids.length) confirm.value = { kind: 'batch', ids }
}

// --- 设备动作 ---
const openRemoteApps = (device: any) => run('remoteApps.openPicker', { device_id: device.id })
function rowActivate(device: any) {
  selected.value = device.id
  if (!props.showsApplications) return
}
function rowDouble(device: any) {
  if (!device.trusted || !device.connected || device.blocked) return
  selected.value = device.id
  if (!props.showsApplications) void openRemoteApps(device)
}
const refresh = () => run('devices.refresh')
const remoteHint = (device: any) =>
  !canOperateRemote.value
    ? (input.value.remoteApplicationRestriction ?? '当前无法操作远程应用')
    : device.connected
      ? '打开远程应用'
      : '连接设备后可打开远程应用'

const moreOpen = ref(false)

const phaseColor = (task: any) => (enumName(task.phase) === 'completed' ? 'df-tone-success' : 'df-secondary')
const percent = (task: any) => (task.total > 0 ? Math.min(100, (task.transferred / task.total) * 100) : 0)
const recent = computed(() => transfers.value.slice(0, 3))
</script>

<template>
  <div class="df-page devices">
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

    <template v-else>
      <aside class="list-pane">
        <div class="list-head">
          <div class="df-row head-row">
            <div class="head-text">
              <div class="df-title">设备列表</div>
              <div class="df-detail df-secondary">{{ showsApplications ? '选择设备，打开远端应用' : '发现并管理附近设备' }}</div>
            </div>
            <span class="df-spacer" />
            <div ref="manualRoot" class="manual">
              <button type="button" class="icon-button" title="手动输入 IPv4 地址" aria-label="手动输入 IPv4 地址" :aria-expanded="showManual" @click="showManual = !showManual">
                <Icon name="plus" :size="16" />
              </button>
              <div v-if="showManual" class="popover card" role="dialog" aria-label="手动输入 IPv4 地址" @keydown.esc="showManual = false">
                <div class="df-section">手动输入 IPv4 地址</div>
                <div class="df-detail df-secondary">配对仍需对方确认。</div>
                <div class="df-row">
                  <input v-model="ip" class="field" placeholder="192.168.1.10" inputmode="decimal" autofocus @keydown.enter="pairByIp" />
                  <button type="button" class="button primary" :disabled="!ip.trim() || busy['devices.pair']" @click="pairByIp">配对</button>
                </div>
              </div>
            </div>
            <button type="button" class="icon-button" title="刷新设备" aria-label="刷新设备" :disabled="busy['devices.refresh']" @click="refresh">
              <Icon name="refresh" :size="16" />
            </button>
            <button
              v-if="!showsApplications"
              type="button"
              class="icon-button"
              :disabled="batchUpdateCandidates.length === 0"
              :title="`批量触发 ${batchUpdateCandidates.length} 台设备更新`"
              aria-label="批量触发设备更新"
              @click="askBatchUpdate"
            >
              <Icon name="download-circle" :size="16" />
            </button>
          </div>

          <label class="search" :class="{ focused: searchFocused }">
            <Icon name="search" :size="14" class="df-secondary" />
            <input
              ref="searchBox"
              v-model="search"
              placeholder="搜索设备名称、IP 地址…"
              @focus="searchFocused = true"
              @blur="searchFocused = false"
              @keydown.esc.prevent="onSearchEsc"
            />
            <button v-if="search" type="button" class="df-hit df-secondary" aria-label="清除设备搜索" @click="clearSearch">
              <Icon name="x-circle" :size="14" />
            </button>
          </label>
        </div>

        <div v-if="pairing.length" class="pairing card">
          <div v-for="request in pairing" :key="request.id" class="pair-row">
            <span class="pair-icon"><Icon name="user-plus" :size="18" /></span>
            <div class="pair-text">
              <div class="df-ellipsis pair-name">{{ request.name }}</div>
              <div class="df-detail df-secondary pair-sub"><i class="warn-dot" />{{ request.incoming ? '请求加入设备网络' : '等待对端确认' }}</div>
            </div>
            <template v-if="request.incoming">
              <span v-if="deciding.has(request.id)" class="spinner" role="status" aria-label="处理中" />
              <span v-else class="df-row pair-actions">
                <button type="button" class="button" @click="decide(request, false)">拒绝</button>
                <button type="button" class="button primary" @click="decide(request, true)">允许</button>
              </span>
            </template>
          </div>
        </div>

        <div class="list df-scroll" role="listbox" aria-label="设备列表">
          <template v-if="localMatches && !showsApplications">
            <div class="group-title df-secondary">本机</div>
            <button
              type="button"
              class="row"
              role="option"
              :aria-selected="selected === localId"
              :class="{ selected: selected === localId }"
              @click="selected = localId"
            >
              <DeviceIcon :name="localName" :src="local.iconUrl" :size="42" />
              <span class="row-text">
                <span class="row-name df-ellipsis">{{ localName }}</span>
                <span class="df-caption df-secondary df-ellipsis">{{ local.runtimeStatus }}</span>
              </span>
            </button>
          </template>

          <template v-for="section in sections" :key="section.title">
            <template v-if="section.items.length">
              <div class="group-title df-secondary">{{ section.title }}（{{ section.items.length }}）</div>
              <div
                v-for="device in section.items"
                :key="device.id"
                class="row"
                role="option"
                tabindex="0"
                :data-device-id="device.id"
                :aria-selected="selected === device.id"
                :class="{ selected: selected === device.id }"
                @click="rowActivate(device)"
                @dblclick="rowDouble(device)"
                @keydown.enter.prevent="rowActivate(device)"
                @keydown.space.prevent="rowActivate(device)"
                @keydown.down.prevent="moveSelection(1, device.id)"
                @keydown.up.prevent="moveSelection(-1, device.id)"
              >
                <DeviceIcon :name="device.name" :src="device.iconUrl" :size="42" />
                <span class="row-text">
                  <span class="row-name df-ellipsis">{{ device.name }}</span>
                  <span class="df-caption df-secondary row-sub">
                    <i class="df-dot" :class="{ on: device.connected }" />
                    <span class="df-ellipsis">{{ listDetail(device) }}</span>
                  </span>
                </span>
                <button
                  v-if="device.trusted && !device.blocked"
                  type="button"
                  class="row-action"
                  :disabled="!device.connected || !canOperateRemote"
                  :title="remoteHint(device)"
                  :aria-label="`打开 ${device.name} 的远程应用`"
                  @click.stop="selected = device.id; if (!showsApplications) openRemoteApps(device)"
                >
                  <Icon name="grid" :size="15" />
                </button>
              </div>
            </template>
          </template>

          <p v-if="noRemote" class="empty-line df-detail df-secondary">{{ search ? '没有匹配的设备' : '尚未发现其他设备' }}</p>
        </div>
      </aside>

      <main class="detail df-scroll" :class="{ applications: showsApplications }">
        <Notice v-if="error" tone="danger" title="操作未完成" :message="error.message" :raw="error.raw" action-title="重试" dismissible @action="retry" @dismiss="dismiss" />

        <slot v-if="showsApplications" name="detail" :device="selectedDevice" :selected-id="selected" />

        <template v-else>
          <p v-if="snap.pairingMessage" class="df-caption df-secondary pairing-message">{{ snap.pairingMessage }}</p>

          <!-- 本机 -->
          <template v-if="selected === localId">
            <section class="card head-card">
              <DeviceIcon :name="localName" :src="local.iconUrl" :size="58" />
              <div class="head-info">
                <div class="df-title">{{ localName }}</div>
                <div class="df-detail" :class="local.error ? 'df-tone-warning' : 'df-tone-success'">
                  <i class="df-dot on" :class="{ warn: !!local.error }" /> {{ local.runtimeStatus }}
                </div>
              </div>
            </section>
            <section class="card info">
              <div class="info-row"><Icon name="power" :size="16" class="df-secondary" /><span class="label">运行状态</span><span class="df-secondary value">{{ local.runtimeStatus || '暂无信息' }}</span></div>
              <hr class="df-divider" />
              <div class="info-row"><Icon name="mouse" :size="16" class="df-secondary" /><span class="label">键鼠</span><span class="df-secondary value">{{ input.summary || '暂无信息' }}</span></div>
            </section>
            <section v-if="local.networkId" class="card leave">
              <button type="button" class="button destructive" @click="confirm = { kind: 'leave', ids: [] }">
                <Icon name="unlink" :size="14" /> 退出设备网络
              </button>
              <p class="df-caption df-secondary">退出后解除本机与所有已配对设备的网络关系，不是断开某一台设备。</p>
            </section>
          </template>

          <!-- 远端设备 -->
          <template v-else-if="selectedDevice">
            <section class="card head-card">
              <DeviceIcon :name="selectedDevice.name" :src="selectedDevice.iconUrl" :size="58" />
              <div class="head-info">
                <div class="df-title df-ellipsis">{{ selectedDevice.name }}</div>
                <div class="df-detail df-row status-line">
                  <i class="df-dot big" :class="{ on: selectedDevice.connected }" />
                  <span :class="selectedDevice.connected ? '' : 'df-secondary'">{{ statusLabel(selectedDevice) }}</span>
                  <span class="df-secondary df-ellipsis">· {{ selectedDevice.endpoint }}</span>
                </div>
                <div class="df-detail df-secondary df-ellipsis">{{ selectedDevice.detail }}</div>
              </div>
              <div class="more">
                <button type="button" class="more-button" aria-label="设备操作" :aria-expanded="moreOpen" @click="moreOpen = !moreOpen" @blur="moreOpen = false">
                  <Icon name="ellipsis" :size="14" />
                  <Icon name="chevron-down" :size="10" />
                </button>
                <div v-if="moreOpen" class="menu card" role="menu">
                  <button type="button" role="menu-item" class="menu-item" :disabled="!selectedDevice.connected || !canOperateRemote" :title="input.remoteApplicationRestriction" @mousedown.prevent="openRemoteApps(selectedDevice); moreOpen = false">
                    <Icon name="grid" :size="14" /> 打开远程应用
                  </button>
                  <hr class="df-divider" />
                  <button type="button" role="menu-item" class="menu-item" @mousedown.prevent="refresh(); moreOpen = false">
                    <Icon name="refresh" :size="14" /> 刷新连接
                  </button>
                </div>
              </div>
            </section>

            <section class="card routes-card">
              <DeviceRoutesPanel
                :routes="selectedDevice.routes ?? []"
                :input-path="selectedDevice.inputPath"
                :seen-paths="selectedDevice.inputPathsSeen ?? []"
                :disabled="busy['devices.setInputPath']"
                @set-input-path="(path) => run('devices.setInputPath', { device_id: selectedDevice.id, path })"
                @refresh="refresh"
              />
            </section>

            <section class="card block">
              <div class="df-row">
                <span class="df-section">传输活动</span>
                <span class="df-spacer" />
                <button type="button" class="df-link" @click="emit('navigate', 'files')">查看全部</button>
              </div>
              <p v-if="recent.length === 0" class="df-detail df-secondary pad-v">暂无传输任务</p>
              <div v-else class="activity">
                <template v-for="(task, index) in recent" :key="task.id">
                  <hr v-if="index" class="df-divider inset" />
                  <div class="task">
                    <Icon :name="task.outgoing ? 'arrow-up' : 'arrow-down'" :size="20" :class="task.outgoing ? 'df-tone-accent' : 'df-tone-success'" />
                    <span class="task-text">
                      <span class="df-detail task-name df-ellipsis">{{ task.name }}</span>
                      <span class="df-caption df-secondary df-ellipsis">{{ task.detail }}</span>
                    </span>
                    <progress v-if="task.total > 0" class="bar" :value="percent(task)" max="100" />
                    <span class="df-spacer" />
                    <span class="df-caption" :class="phaseColor(task)">{{ transferPhaseTitle(task) }}</span>
                    <Icon v-if="enumName(task.phase) === 'completed'" name="search" :size="14" class="df-secondary" />
                    <button v-else-if="task.canRetry" type="button" class="df-hit" aria-label="重试传输" @click="run('transfers.retry', { id: task.id })"><Icon name="refresh" :size="14" /></button>
                    <button v-else type="button" class="df-hit df-secondary" aria-label="取消传输" @click="run('transfers.cancel', { id: task.id })"><Icon name="x" :size="14" /></button>
                  </div>
                </template>
              </div>
            </section>

            <section class="card block">
              <div class="df-section">操作</div>
              <div class="df-row wrap">
                <template v-if="selectedDevice.blocked">
                  <button type="button" class="button" @click="run('devices.unblock', { peer_id: selectedDevice.id })"><Icon name="shield" :size="14" /> 允许重新配对</button>
                </template>
                <template v-else-if="!selectedDevice.trusted">
                  <button type="button" class="button primary" @click="run('devices.pair', { peer_id: selectedDevice.id })"><Icon name="user-plus" :size="14" /> 邀请配对</button>
                </template>
                <template v-else>
                  <button type="button" class="button" @click="run(selectedDevice.connected ? 'devices.disconnect' : 'devices.connect', { peer_id: selectedDevice.id })">
                    <Icon :name="selectedDevice.connected ? 'unlink' : 'refresh'" :size="14" /> {{ selectedDevice.connected ? '断开连接' : '重新连接' }}
                  </button>
                  <button type="button" class="button destructive" @click="confirm = { kind: 'forget', ids: [selectedDevice.id] }"><Icon name="trash" :size="14" /> 忘记设备</button>
                  <button
                    type="button"
                    class="button"
                    :disabled="!selectedDevice.connected || !selectedDevice.canTriggerUpdate"
                    :title="selectedDevice.canTriggerUpdate ? `请求 ${selectedDevice.name} 立即检查并安装可用更新` : '对端尚未声明支持主动更新'"
                    @click="run('devices.triggerUpdate', { peer_id: selectedDevice.id })"
                  >
                    <Icon name="download-circle" :size="14" /> 触发更新
                  </button>
                </template>
                <button v-if="!selectedDevice.blocked" type="button" class="button destructive" @click="confirm = { kind: 'block', ids: [selectedDevice.id] }">
                  <Icon name="hand" :size="14" /> 拉黑设备
                </button>
              </div>
            </section>
          </template>

          <div v-else class="empty-state">
            <Icon name="laptop" :size="40" />
            <h2 class="df-title">选择设备</h2>
            <p class="df-secondary">从左侧列表选择设备查看连接与传输状态。</p>
          </div>
        </template>
      </main>
    </template>

    <ConfirmDialog
      :open="!!confirm"
      :title="confirmCopy.title"
      :message="confirmCopy.message"
      :confirm-title="confirmCopy.action"
      :destructive="confirmCopy.destructive"
      @confirm="confirmed"
      @cancel="confirm = null"
    />
  </div>
</template>

<style scoped>
.devices { flex-direction: row; }
.boot { flex: 1; display: grid; place-items: center; }
.empty-state { display: grid; justify-items: center; gap: 10px; padding: 32px; text-align: center; color: var(--secondary); }
.empty-state .df-title { color: var(--text); margin: 0; }
.empty-state p { margin: 0; max-width: 360px; }

.list-pane { flex: 0 1 380px; width: 380px; min-width: 300px; margin-top: 8px; display: flex; flex-direction: column; min-height: 0; border-right: 0.5px solid var(--line); }
.list-head { padding: 18px 20px 14px; display: grid; gap: 14px; }
.head-row { align-items: flex-start; gap: 12px; }
.head-row .icon-button { width: 32px; height: 32px; min-height: 32px; padding: 0; border-color: transparent; background: var(--control); color: var(--accent); border-radius: 8px; }
.head-row .icon-button:hover:not(:disabled) { background: color-mix(in srgb, var(--text) 8%, var(--control)); }
.head-text { display: grid; gap: 5px; min-width: 0; }
.manual { position: relative; }
.popover { position: absolute; top: calc(100% + 6px); left: 0; z-index: 20; width: 300px; padding: 16px; display: grid; gap: 12px; background: var(--surface); box-shadow: 0 8px 28px rgba(0, 0, 0, 0.22); }
.search { display: flex; align-items: center; gap: 8px; height: 32px; padding: 0 12px; border-radius: 8px; background: var(--control); outline: 2px solid transparent; }
.search.focused { outline-color: color-mix(in srgb, var(--accent) 80%, transparent); }
.search input { flex: 1; min-width: 0; border: 0; outline: 0; background: transparent; color: var(--text); font: inherit; }

.pairing { margin: 0 18px 8px; padding: 12px; display: grid; gap: 10px; border-color: color-mix(in srgb, var(--warning, #ff9500) 18%, transparent); background: color-mix(in srgb, var(--warning, #ff9500) 4%, transparent); }
.pair-row { display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-radius: 12px; background: color-mix(in srgb, var(--warning, #ff9500) 6%, transparent); border: 1px solid color-mix(in srgb, var(--warning, #ff9500) 20%, transparent); }
.pair-icon { width: 40px; height: 40px; flex: none; display: grid; place-items: center; border-radius: 50%; color: var(--warning, #ff9500); background: color-mix(in srgb, var(--warning, #ff9500) 16%, transparent); }
.pair-text { flex: 1; min-width: 0; display: grid; gap: 4px; }
.pair-name { font-size: 14px; font-weight: 600; }
.pair-sub { display: flex; align-items: center; gap: 4px; }
.warn-dot { width: 5px; height: 5px; border-radius: 50%; background: var(--warning, #ff9500); }
.pair-actions { gap: 6px; }
.spinner { width: 16px; height: 16px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

.list { flex: 1; padding: 0 8px 12px; display: flex; flex-direction: column; }
.group-title { font-size: 13px; font-weight: 600; padding: 8px 11px 4px; margin-bottom: 2px; }
.group-title:first-child { position: relative; margin-bottom: 12px; }
.group-title:first-child::after { content: ''; position: absolute; left: -8px; right: -8px; bottom: -1px; height: 1px; background: var(--line); }
.row + .group-title { margin-top: 24px; }
.row { width: calc(100% - 32px); display: flex; align-items: center; gap: 12px; padding: 6px 10px; margin: 2px 16px; min-height: 52px; border: 0; border-radius: 8px; background: transparent; color: inherit; font: inherit; text-align: left; cursor: default; }
.row:hover { background: color-mix(in srgb, var(--accent) 6%, transparent); }
.row.selected { background: color-mix(in srgb, var(--accent) 18%, transparent); }
.row:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
.row-text { flex: 1; min-width: 0; display: grid; gap: 5px; }
.row-name { font-size: 14px; font-weight: 600; }
.row-sub { display: flex; align-items: center; gap: 6px; }
.row-action { width: 26px; height: 26px; flex: none; display: grid; place-items: center; border: 0; background: none; padding: 0; color: var(--accent); opacity: 0.5; border-radius: 6px; }
.row-action:hover:not(:disabled) { opacity: 0.9; background: color-mix(in srgb, var(--accent) 8%, transparent); }
.row-action:disabled { opacity: 0.25; }
.empty-line { padding: 20px 11px; margin: 0; }

.detail { flex: 1 1 0; min-width: 420px; margin-top: 8px; padding: 4px 24px 24px; display: grid; align-content: start; gap: 16px; }
.detail.applications { display: flex; flex-direction: column; align-content: normal; gap: 0; padding: 0; }
.detail.applications > .notice { margin: 12px 20px 0; }
.pairing-message { margin: 0; user-select: text; }
.head-card { display: flex; align-items: flex-start; gap: 16px; padding: 20px; }
.head-card .head-info { flex: 1; min-width: 0; display: grid; gap: 7px; }
.head-card .head-info .df-detail { line-height: 14px; }
.status-line { gap: 8px; min-height: 0; }
.df-dot.big { width: 10px; height: 10px; }
.df-dot.warn { background: var(--warning, #ff9500); }
.info { padding: 0 18px; }
.info-row { display: flex; align-items: flex-start; gap: 12px; padding: 15px 0; font-size: 12px; }
.info-row .label { width: 62px; flex: none; }
.info-row .value { flex: 1; min-width: 0; user-select: text; overflow-wrap: anywhere; }
.leave { padding: 18px; display: grid; gap: 10px; justify-items: start; }
.leave p { margin: 0; }
.block { padding: 16px; display: grid; gap: 12px; }
.block > .df-row:not(.wrap) { min-height: 0; }
.routes-card { padding: 0; }
.activity { display: grid; }
.task { display: flex; align-items: center; gap: 10px; padding: 11px 0; }
.task-text { display: grid; gap: 5px; min-width: 90px; max-width: 180px; }
.task-name { font-weight: 500; }
.bar { width: 110px; height: 4px; accent-color: var(--accent); }
.df-divider.inset { margin-left: 38px; }
.pad-v { padding: 8px 0; margin: 0; line-height: 1.2; }
.wrap { flex-wrap: wrap; gap: 12px; }
.destructive { color: #fff; background: var(--danger, #ff3b30); border-color: transparent; }
.destructive:hover:not(:disabled) { background: color-mix(in srgb, #000 6%, var(--danger, #ff3b30)); color: #fff; }
.more { position: relative; flex: none; }
.more-button { height: 24px; min-height: 24px; display: inline-flex; align-items: center; justify-content: center; gap: 3px; padding: 0 7px; border: 1px solid var(--line); border-radius: 8px; background: var(--control); color: var(--text); }
.more-button:hover { background: color-mix(in srgb, var(--text) 8%, var(--control)); }
.menu { position: absolute; right: 0; top: calc(100% + 4px); z-index: 20; min-width: 168px; padding: 4px; background: var(--surface); box-shadow: 0 8px 28px rgba(0, 0, 0, 0.22); display: grid; }
.menu-item { display: flex; align-items: center; gap: 8px; height: 28px; padding: 0 9px; border: 0; border-radius: 6px; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
.menu-item:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 12%, transparent); }
.menu-item:disabled { opacity: 0.45; cursor: default; }
</style>
