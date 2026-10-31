<script setup lang="ts">
import { computed } from 'vue'
import Icon from './Icon.vue'
import { enumName } from './core'

// 对应 DeviceRoutesPanel：来源与健康状态由引擎提供，不按 IP 前缀猜测。
const props = defineProps<{
  routes: any[]
  inputPath: unknown
  seenPaths: unknown[]
  disabled?: boolean
}>()
const emit = defineEmits<{ (e: 'setInputPath', path: string): void; (e: 'refresh'): void }>()

const labels: Record<string, string> = { auto: '自动', thunderbolt: '雷电', ethernet: '有线', wifi: 'Wi-Fi' }
const options = ['thunderbolt', 'ethernet', 'wifi'] as const
const current = computed(() => enumName(props.inputPath) || 'auto')
const seen = computed(() => new Set((props.seenPaths ?? []).map(enumName)))
const label = computed(() => labels[current.value] ?? '自动')

const footnote = computed(() =>
  current.value === 'auto'
    ? '同一地址只显示一次。自动模式按实测握手延迟选择最快可达线路；刷新会立即重测候选地址，不会中断当前连接。'
    : `键鼠共享只走${label.value}，${label.value}断开时暂停，不切换到其他线路；剪贴板和文件传输不受影响。`,
)

function probeTime(ms: number) {
  return new Date(ms).toLocaleTimeString('zh-CN', { hour12: false })
}
function rtt(value: number) {
  return value === 0 ? '<1 ms' : `${value} ms`
}
function pick(event: Event) {
  emit('setInputPath', (event.target as HTMLSelectElement).value)
}
</script>

<template>
  <section class="routes">
    <header class="df-row head">
      <span class="df-section">连接与路由</span>
      <span class="df-secondary" title="候选地址来自已发现或手动配置的连接链路"><Icon name="info" :size="13" /></span>
      <span class="df-caption df-secondary count">{{ routes.length }}</span>
      <span class="df-spacer" />
      <button type="button" class="df-link refresh-link" @click="emit('refresh')"><Icon name="refresh" :size="12" /> 刷新</button>
    </header>

    <p v-if="routes.length === 0" class="df-detail df-secondary">暂未发现连接地址。请确认对方应用正在运行后刷新，或通过 IP 添加设备。</p>
    <template v-else>
      <div class="wide">
        <div class="tr th df-caption df-secondary">
          <span />
          <span>地址</span>
          <span>发现来源</span>
          <span>状态</span>
        </div>
        <div v-for="route in routes" :key="route.id" class="tr">
          <span class="radio" :class="{ on: route.active }"><i /></span>
          <span class="addr">
            <span class="df-mono df-ellipsis endpoint">{{ route.endpoint }}</span>
            <span v-if="route.probeError" class="df-caption df-tone-warning">探测失败</span>
            <span v-else-if="route.lastProbeUnixMs" class="df-caption df-secondary">最近探测 {{ probeTime(route.lastProbeUnixMs) }}</span>
          </span>
          <span class="df-caption df-secondary stack">
            <span>{{ route.source }}</span>
            <span v-if="route.path">{{ route.path }}</span>
          </span>
          <span class="df-caption stack">
            <span :class="route.active ? 'df-tone-success' : 'df-secondary'">{{ route.status }}</span>
            <span v-if="route.rttMs !== null && route.rttMs !== undefined" class="num">{{ rtt(route.rttMs) }}</span>
          </span>
        </div>
      </div>

      <div class="compact">
        <div v-for="route in routes" :key="route.id" class="route">
          <span class="radio" :class="{ on: route.active }"><i /></span>
          <div class="route-info">
            <span class="df-mono endpoint">{{ route.endpoint }}</span>
            <div class="route-meta">
              <span class="df-secondary">{{ route.source }}</span>
              <span :class="route.active ? 'df-tone-success' : 'df-secondary'">{{ route.status }}</span>
              <span v-if="route.rttMs !== null && route.rttMs !== undefined" class="num df-secondary">{{ rtt(route.rttMs) }}</span>
            </div>
            <span v-if="route.path" class="df-secondary">{{ route.path }}</span>
            <span v-if="route.probeError" class="df-tone-warning">探测失败</span>
            <span v-else-if="route.lastProbeUnixMs" class="df-secondary">最近探测 {{ probeTime(route.lastProbeUnixMs) }}</span>
          </div>
        </div>
      </div>

      <p v-if="routes.length === 1" class="df-caption df-secondary">目前仅发现这一条候选地址，不代表设备只有一个网络接口。</p>
    </template>

    <div class="df-row path-row">
      <span class="df-detail">键鼠线路</span>
      <span class="df-spacer" />
      <select class="path-select" aria-label="键鼠线路" :value="current" :disabled="disabled" @change="pick">
        <option value="auto">自动</option>
        <option
          v-for="path in options"
          :key="path"
          :value="path"
          :disabled="path !== current && !seen.has(path)"
        >
          仅{{ labels[path] }}
        </option>
      </select>
    </div>
    <p class="df-caption df-secondary footnote">{{ footnote }}</p>
  </section>
</template>

<style scoped>
.routes { padding: 16px; display: grid; gap: 12px; container-type: inline-size; }
.routes p { line-height: 1.2; }
.head { gap: 7px; min-height: 0; }
.count { font-variant-numeric: tabular-nums; }
.refresh-link { display: inline-flex; align-items: center; gap: 5px; }
.endpoint { user-select: text; }

.wide { display: grid; }
.compact { display: none; }
.tr {
  display: grid;
  grid-template-columns: 24px minmax(160px, 1fr) 128px 94px;
  gap: 12px;
  align-items: start;
  padding: 12px 14px;
}
.tr.th { padding: 10px 14px; border-bottom: 0.5px solid var(--line); }
.tr:not(.th):not(:last-child) { border-bottom: 0.5px solid var(--line); }
.addr, .stack { display: grid; gap: 4px; min-width: 0; }
.stack { gap: 3px; }
.num { font-variant-numeric: tabular-nums; }

.route { display: flex; align-items: flex-start; gap: 12px; padding: 12px 0; }
.route + .route { border-top: 0.5px solid var(--line); }
.route-info { display: grid; gap: 6px; min-width: 0; font-size: 12px; line-height: 1.2; }
.route-meta { display: flex; flex-wrap: wrap; align-items: baseline; gap: 8px; }

.radio {
  width: 24px;
  height: 24px;
  flex: none;
  position: relative;
}
.radio::before {
  content: '';
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  width: 13px;
  height: 13px;
  border-radius: 50%;
  border: 1.5px solid var(--secondary);
}
.radio.on::before { border-color: var(--accent); }
.radio i {
  display: none;
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
}
.radio.on i { display: block; }

.path-row { gap: 12px; min-height: 24px; }
.path-select {
  height: 24px;
  min-height: 24px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  color: var(--accent);
  font-size: 12px;
  flex: none;
}
.path-select:disabled { opacity: 0.45; }
.footnote { margin: 0; }

@container (max-width: 469px) {
  .wide { display: none; }
  .compact { display: grid; }
}
</style>
