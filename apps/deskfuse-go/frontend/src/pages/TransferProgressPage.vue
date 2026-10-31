<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { ArrowDown, ArrowUp, X } from 'lucide-vue-next'
import { backend, errorMessage } from '../services/backend'
import { byteCount, camelize, enumName, transferPhaseTitle } from './features/core'

// 全局传输进度：原生版是屏幕右上角的浮动面板。窗口归属与位置由 Go 进程决定，
// 这里只负责渲染、取消和关闭提示；快照仍是唯一数据来源。
type Transfer = {
  id: string
  name: string
  detail: string
  phase: unknown
  transferred: number
  total: number
  outgoing: boolean
  canRetry: boolean
  saved: number
  saveTotal: number
}

const transfers = ref<Transfer[]>([])
const dismissed = ref(new Set<string>())
const error = ref('')
const timers = new Map<string, ReturnType<typeof setTimeout>>()
let poll: ReturnType<typeof setTimeout> | undefined
let stopped = false

const visible = computed(() => transfers.value.filter((task) => enumName(task.phase) !== 'completed' && !dismissed.value.has(task.id)))

const phaseOf = (task: Transfer) => enumName(task.phase)
const finished = (task: Transfer) => ['failed', 'cancelled'].includes(phaseOf(task))
const fraction = (task: Transfer) => {
  const total = phaseOf(task) === 'delivering' ? task.saveTotal : task.total
  const bytes = phaseOf(task) === 'delivering' ? task.saved : task.transferred
  return total > 0 ? Math.min(1, bytes / total) : 0
}
const status = (task: Transfer) => {
  const percent = Math.round(fraction(task) * 100)
  return ['sending', 'receiving', 'delivering'].includes(phaseOf(task)) ? `${transferPhaseTitle({ phase: task.phase, outgoing: task.outgoing })} ${percent}%` : transferPhaseTitle({ phase: task.phase, outgoing: task.outgoing })
}
const detail = (task: Transfer) => {
  if (['sending', 'receiving'].includes(phaseOf(task)) && fraction(task) < 1) {
    return `${byteCount(task.transferred)} / ${byteCount(task.total)}`
  }
  return task.detail
}
const tint = (task: Transfer) => (phaseOf(task) === 'failed' ? 'failed' : phaseOf(task) === 'cancelled' ? 'muted' : task.outgoing ? 'outgoing' : 'incoming')

async function refresh() {
  try {
    const snapshot = camelize(await backend.snapshot()) as { transfers?: Transfer[] }
    if (stopped) return
    transfers.value = snapshot.transfers ?? []
    error.value = ''
    scheduleCancelledDismissals()
  } catch (e) {
    if (!stopped) error.value = errorMessage(e)
  }
}

// 已取消的提示 3 秒后自动关闭，与原生进度面板一致；计时器只在进入该状态时启动。
function scheduleCancelledDismissals() {
  const cancelled = new Set(visible.value.filter((task) => phaseOf(task) === 'cancelled').map((task) => task.id))
  for (const [id, timer] of timers) {
    if (!cancelled.has(id)) {
      clearTimeout(timer)
      timers.delete(id)
    }
  }
  for (const id of cancelled) {
    if (timers.has(id)) continue
    timers.set(id, setTimeout(() => {
      timers.delete(id)
      void dismiss(id)
    }, 3000))
  }
}

async function dismiss(id: string) {
  dismissed.value = new Set(dismissed.value).add(id)
  try { await backend.dismissTransfer(id) } catch { /* 窗口已经收起时忽略 */ }
}

async function close(task: Transfer) {
  if (finished(task)) { await dismiss(task.id); return }
  try { await backend.invoke('transfers.cancel', { id: task.id }) } catch (e) { error.value = errorMessage(e) }
}

async function loop() {
  if (!stopped && document.visibilityState !== 'hidden') await refresh()
  if (!stopped) poll = setTimeout(loop, 500)
}

function onVisibility() { if (document.visibilityState !== 'hidden') void refresh() }

onMounted(() => {
  // 透明窗口：页面不能铺底色，否则浮动面板会变成整块方框。
  document.documentElement.classList.add('transparent-window')
  void loop()
  document.addEventListener('visibilitychange', onVisibility)
})
onBeforeUnmount(() => {
  stopped = true
  if (poll) clearTimeout(poll)
  for (const timer of timers.values()) clearTimeout(timer)
  timers.clear()
  document.removeEventListener('visibilitychange', onVisibility)
  document.documentElement.classList.remove('transparent-window')
})
</script>

<template>
  <div class="stack" role="list" aria-label="传输进度">
    <div v-for="task in visible" :key="task.id" class="pill" :class="tint(task)" role="listitem">
      <span class="icon" :aria-label="task.outgoing ? '发送' : '接收'">
        <ArrowUp v-if="task.outgoing" :size="14" />
        <ArrowDown v-else :size="14" />
      </span>
      <div class="body">
        <div class="line">
          <span class="name" :title="task.name">{{ task.name }}</span>
          <span class="status">{{ status(task) }}</span>
          <button type="button" class="close" :aria-label="finished(task) ? '关闭提示' : '取消传输'" :title="finished(task) ? '关闭提示' : '取消传输'" @click="close(task)">
            <X :size="12" />
          </button>
        </div>
        <div class="track" :class="{ sweeping: ['queued', 'verifying'].includes(phaseOf(task)) }">
          <span :style="{ width: ['queued', 'verifying'].includes(phaseOf(task)) ? '38%' : `${Math.max(2, fraction(task) * 100)}%` }" />
        </div>
        <div class="detail" :title="task.detail">{{ error || detail(task) }}</div>
      </div>
    </div>
  </div>
</template>

<style>
html.transparent-window,
html.transparent-window body { background: transparent !important; }
</style>

<style scoped>
.stack{display:flex;flex-direction:column;gap:8px;padding:8px;height:100vh;overflow:auto;box-sizing:border-box}
.pill{display:flex;gap:12px;width:300px;height:76px;flex:0 0 76px;padding:12px 14px;border-radius:16px;box-sizing:border-box;background:color-mix(in srgb,var(--surface) 94%,transparent);box-shadow:0 1px 3px #0002,0 8px 24px #0001;color:var(--text)}
.icon{display:grid;place-items:center;width:30px;height:30px;border-radius:50%;flex:0 0 30px}
.pill.outgoing .icon{color:var(--accent);background:color-mix(in srgb,var(--accent) 14%,transparent)}
.pill.incoming .icon{color:var(--success);background:color-mix(in srgb,var(--success) 14%,transparent)}
.pill.failed .icon{color:var(--warning);background:color-mix(in srgb,var(--warning) 14%,transparent)}
.pill.muted .icon{color:var(--secondary);background:var(--control)}
.body{display:flex;flex-direction:column;gap:6px;min-width:0;flex:1;padding-top:1px}
.line{display:flex;align-items:baseline;gap:8px;min-width:0}
.name{font-size:13px;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;min-width:0;flex:1}
.status{font-size:11px;font-weight:500;color:var(--secondary);font-variant-numeric:tabular-nums;white-space:nowrap}
.pill.failed .status{color:var(--warning)}
.close{display:grid;place-items:center;width:20px;height:20px;color:var(--secondary);border-radius:5px;flex:0 0 20px;margin-top:-2px}
.close:hover{background:var(--control);color:var(--text)}
.track{height:4px;border-radius:2px;background:color-mix(in srgb,var(--text) 10%,transparent);overflow:hidden;position:relative}
.track>span{display:block;height:100%;border-radius:2px;transition:width .25s ease}
.pill.outgoing .track>span{background:var(--accent)}
.pill.incoming .track>span{background:var(--success)}
.pill.failed .track>span{background:var(--warning)}
.pill.muted .track>span{background:var(--secondary)}
.track.sweeping>span{animation:sweep 1.4s linear infinite}
@keyframes sweep{from{transform:translateX(-100%)}to{transform:translateX(264%)}}
.detail{font-size:11px;color:var(--secondary);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-variant-numeric:tabular-nums}
</style>
