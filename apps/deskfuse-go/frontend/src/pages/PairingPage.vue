<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { UserRoundPlus } from 'lucide-vue-next'
import { backend, errorMessage } from '../services/backend'
import { camelize } from './features/core'

// 设备配对确认：原生版是居中的浮动面板。窗口由 Go 进程创建和收起，
// 页面只显示待确认请求并把决定交给引擎。
type Pairing = { id: string; peerId: string; name: string; incoming: boolean; expiresAtMs: number }

const requests = ref<Pairing[]>([])
const pending = ref(new Set<string>())
const error = ref('')
let poll: ReturnType<typeof setTimeout> | undefined
let stopped = false

const visible = computed(() => requests.value.filter((request) => request.incoming))

async function refresh() {
  try {
    const snapshot = camelize(await backend.snapshot()) as { pairing?: Pairing[]; app?: { error?: string | null } }
    if (stopped) return
    requests.value = snapshot.pairing ?? []
    error.value = snapshot.app?.error ?? ''
    // 已被处理或过期的请求不再显示“处理中”。
    const live = new Set(visible.value.map((request) => request.id))
    for (const id of [...pending.value]) if (!live.has(id)) pending.value.delete(id)
  } catch (e) {
    if (!stopped) error.value = errorMessage(e)
  }
}

async function decide(id: string, approve: boolean) {
  pending.value = new Set(pending.value).add(id)
  try {
    await backend.invoke(approve ? 'pairing.approve' : 'pairing.reject', { id })
    await refresh()
  } catch (e) {
    error.value = errorMessage(e)
    pending.value.delete(id)
  }
}

// 关闭面板等同于放弃这些请求：与原生面板关闭时一致，逐条拒绝未决定的请求。
async function later() {
  const undecided = visible.value.filter((request) => !pending.value.has(request.id))
  await Promise.all(undecided.map((request) => backend.invoke('pairing.reject', { id: request.id }).catch(() => null)))
  await refresh()
}

async function loop() {
  if (!stopped && document.visibilityState !== 'hidden') await refresh()
  if (!stopped) poll = setTimeout(loop, 500)
}
function onVisibility() { if (document.visibilityState !== 'hidden') void refresh() }

onMounted(() => { void loop(); document.addEventListener('visibilitychange', onVisibility) })
onBeforeUnmount(() => {
  stopped = true
  if (poll) clearTimeout(poll)
  document.removeEventListener('visibilitychange', onVisibility)
})
</script>

<template>
  <div class="pairing">
    <header class="titlebar">
      <div>
        <h1>设备配对</h1>
        <p>确认设备加入网络</p>
      </div>
      <button type="button" class="button" @click="later">稍后处理</button>
    </header>
    <div class="list">
      <div v-if="error" class="notice" role="alert">{{ error }}</div>
      <div v-for="request in visible" :key="request.id" class="request">
        <span class="icon" aria-hidden="true"><UserRoundPlus :size="20" /></span>
        <div class="info">
          <strong :title="request.name">{{ request.name }}</strong>
          <span class="state"><i />{{ request.incoming ? '请求加入设备网络' : '等待对端确认' }}</span>
        </div>
        <div v-if="pending.has(request.id)" class="processing">处理中</div>
        <div v-else class="actions">
          <button type="button" class="button" @click="decide(request.id, false)">拒绝</button>
          <button type="button" class="button primary" @click="decide(request.id, true)">接受</button>
        </div>
      </div>
      <div v-if="!visible.length && !error" class="state-empty">没有待确认的配对请求</div>
    </div>
  </div>
</template>

<style scoped>
.pairing{display:flex;flex-direction:column;height:100vh;overflow:hidden;background:var(--canvas);color:var(--text)}
.titlebar{display:flex;align-items:flex-start;justify-content:space-between;gap:12px;padding:20px 24px 16px;flex:0 0 auto;--app-region:drag;-webkit-app-region:drag}
.titlebar h1{font-size:19px;font-weight:700}
.titlebar p{font-size:13px;color:var(--secondary);margin-top:4px}
.titlebar button{--app-region:no-drag;-webkit-app-region:no-drag}
.list{flex:1;min-height:0;overflow:auto;display:flex;flex-direction:column;gap:10px;padding:0 20px 20px}
.request{display:flex;align-items:center;gap:14px;padding:16px;border-radius:12px;background:var(--surface);border:1px solid var(--line);box-shadow:0 1px 2px #00000010}
.icon{display:grid;place-items:center;width:44px;height:44px;flex:0 0 44px;border-radius:50%;color:var(--accent);background:color-mix(in srgb,var(--accent) 12%,transparent)}
.info{display:flex;flex-direction:column;gap:5px;min-width:0;flex:1}
.info strong{font-size:14px;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.state{display:flex;align-items:center;gap:6px;font-size:12px;color:var(--secondary)}
.state i{width:6px;height:6px;border-radius:50%;background:var(--warning)}
.actions{display:flex;gap:8px;flex:0 0 auto}
.actions .button{min-height:30px;border-radius:15px;padding:0 16px}
.processing{font-size:12px;color:var(--secondary);padding:0 8px}
.notice{align-self:stretch;font-size:12px}
.state-empty{padding:32px 12px;text-align:center;color:var(--secondary);font-size:12px}
</style>
