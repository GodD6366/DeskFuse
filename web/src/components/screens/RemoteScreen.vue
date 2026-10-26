<script setup lang="ts">
import { computed, reactive, ref } from 'vue'

defineProps<{ active?: boolean }>()

type Kind = 'code' | 'timeline' | 'tracks' | 'files' | 'grid' | 'text'

interface App {
  id: string
  name: string
  kind: Kind
  color: string
}

interface Source {
  id: 'mac' | 'win'
  name: string
  tab: string
  apps: App[]
}

const sources: Source[] = [
  {
    id: 'mac',
    name: 'Mac mini',
    tab: 'Mac mini',
    apps: [
      { id: 'xcode', name: 'Xcode', kind: 'code', color: '#3d7bf2' },
      { id: 'fcp', name: 'Final Cut Pro', kind: 'timeline', color: '#2d2f35' },
      { id: 'logic', name: 'Logic Pro', kind: 'tracks', color: '#5b5f68' },
      { id: 'finder', name: '访达', kind: 'files', color: '#44a2ee' },
    ],
  },
  {
    id: 'win',
    name: 'Windows PC',
    tab: 'Windows',
    apps: [
      { id: 'excel', name: 'Excel', kind: 'grid', color: '#22834f' },
      { id: 'vs', name: 'Visual Studio', kind: 'code', color: '#7b4fc9' },
      { id: 'notepad', name: '记事本', kind: 'text', color: '#5d8fb8' },
      { id: 'explorer', name: '资源管理器', kind: 'files', color: '#e2a81c' },
    ],
  },
]

interface Win {
  key: string
  app: App
  source: string
  x: number
  y: number
  z: number
}

const sourceId = ref<Source['id']>('mac')
const source = computed(() => sources.find((s) => s.id === sourceId.value)!)
const windows = reactive<Win[]>([])
const area = ref<HTMLElement | null>(null)
let z = 1
let opened = 0

const MAX = 3

function open(app: App) {
  const key = `${sourceId.value}:${app.id}`
  const existing = windows.find((w) => w.key === key)
  if (existing) {
    existing.z = ++z
    return
  }
  if (windows.length >= MAX) {
    const oldest = windows.reduce((a, b) => (a.z < b.z ? a : b))
    windows.splice(windows.indexOf(oldest), 1)
  }
  const step = opened++ % 4
  windows.push({ key, app, source: source.value.name, x: 3 + step * 9, y: 4 + step * 11, z: ++z })
}

function close(w: Win) {
  windows.splice(windows.indexOf(w), 1)
}

function isOpen(app: App) {
  return windows.some((w) => w.key === `${sourceId.value}:${app.id}`)
}

// 拖动标题栏移动窗口，位置用百分比，屏幕缩放时不走样
let drag: { w: Win; sx: number; sy: number; ox: number; oy: number } | null = null

function onDown(event: PointerEvent, w: Win) {
  w.z = ++z
  if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return
  drag = { w, sx: event.clientX, sy: event.clientY, ox: w.x, oy: w.y }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}

function onMove(event: PointerEvent) {
  if (!drag || !area.value) return
  const r = area.value.getBoundingClientRect()
  drag.w.x = Math.min(Math.max(drag.ox + ((event.clientX - drag.sx) / r.width) * 100, -10), 50)
  drag.w.y = Math.min(Math.max(drag.oy + ((event.clientY - drag.sy) / r.height) * 100, 0), 46)
}

function onUp() {
  drag = null
}

open(sources[0].apps[1])
open(sources[0].apps[0])
</script>

<template>
  <div class="sc-host">
    <div class="sc">
      <div class="sc__copy">
        <span class="sc__beta">Beta</span>
        <h2>另一台的 App，开成这台的窗口</h2>
        <p>
          在 MacBook 上直接用 Mac mini 里的 Xcode、Final Cut Pro，或者 Windows 上的 Excel。不是把整块屏幕投过来：每个应用一个独立窗口，可以拖动、缩放、最小化。
        </p>
        <ul class="sc__list">
          <li>从应用列表里挑，正在运行的排在前面</li>
          <li>中文输入法的候选框跟着窗口一起过来</li>
          <li>Windows 应用通过 RemoteApp 打开，走 DeskFuse 的加密连接，不用对局域网开放远程桌面端口</li>
        </ul>
      </div>

      <div class="sc__demo">
        <div class="remote">
          <div class="picker">
            <div class="picker__sources" role="tablist" aria-label="选择电脑">
              <button
                v-for="s in sources"
                :key="s.id"
                type="button"
                role="tab"
                :aria-selected="sourceId === s.id"
                :class="{ 'is-on': sourceId === s.id }"
                @click="sourceId = s.id"
              >
                {{ s.tab }}
              </button>
            </div>
            <ul class="picker__apps">
              <li v-for="app in source.apps" :key="app.id">
                <button type="button" @click="open(app)">
                  <span class="tile" :style="{ background: app.color }">{{ app.name.slice(0, 1) }}</span>
                  <span class="picker__name">{{ app.name }}</span>
                  <span v-if="isOpen(app)" class="picker__open">已打开</span>
                </button>
              </li>
            </ul>
          </div>

          <div ref="area" class="remote__area" @pointermove="onMove" @pointerup="onUp" @pointercancel="onUp">
            <p v-if="!windows.length" class="remote__empty">点左边的应用，它的窗口会出现在这里</p>
            <TransitionGroup name="win">
              <div
                v-for="w in windows"
                :key="w.key"
                class="win"
                :style="{ left: `${w.x}%`, top: `${w.y}%`, zIndex: w.z }"
                @pointerdown="onDown($event, w)"
              >
                <div class="win__bar">
                  <button class="win__close" type="button" :aria-label="`关闭 ${w.app.name}`" @click="close(w)"></button>
                  <i></i><i></i>
                  <span>{{ w.app.name }} — {{ w.source }}</span>
                </div>
                <div class="win__body" :class="`k-${w.app.kind}`">
                  <template v-if="w.app.kind === 'code'">
                    <i v-for="n in 9" :key="n" :style="{ width: `${30 + ((n * 37) % 55)}%`, marginLeft: `${(n % 3) * 8}%` }"></i>
                  </template>
                  <template v-else-if="w.app.kind === 'timeline'">
                    <b class="preview"></b>
                    <i v-for="n in 3" :key="n"></i>
                  </template>
                  <template v-else-if="w.app.kind === 'tracks'">
                    <i v-for="n in 4" :key="n"></i>
                  </template>
                  <template v-else-if="w.app.kind === 'files'">
                    <i v-for="n in 6" :key="n"></i>
                  </template>
                  <template v-else-if="w.app.kind === 'grid'">
                    <i v-for="n in 32" :key="n"></i>
                  </template>
                  <template v-else>
                    <i v-for="n in 7" :key="n" :style="{ width: `${50 + ((n * 23) % 45)}%` }"></i>
                  </template>
                </div>
              </div>
            </TransitionGroup>
          </div>
        </div>
        <p class="remote__caption">这台 MacBook 上打开的是另一台电脑的应用</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.remote {
  display: grid;
  grid-template-columns: 176px minmax(0, 1fr);
  gap: 14px;
  height: clamp(320px, 52cqi, 430px);
  padding: 14px;
  border-radius: 16px;
  background: #e3e1dc;
}

.picker {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.82);
  box-shadow: 0 8px 24px -16px rgba(30, 30, 30, 0.5);
}

.picker__sources {
  display: grid;
  grid-template-columns: 1fr 1fr;
  padding: 2px;
  border-radius: 8px;
  background: var(--c-desk);
}

.picker__sources button {
  padding: 4px 0;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--c-muted);
  font: inherit;
  font-size: 0.75rem;
  font-weight: 600;
  cursor: pointer;
}

.picker__sources button.is-on {
  background: #fff;
  color: var(--c-ink);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
}

.picker__apps {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.picker__apps button {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 7px 8px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  font: inherit;
  font-size: 0.8125rem;
  text-align: left;
  cursor: pointer;
}

.picker__apps button:hover {
  background: var(--c-desk);
}

.tile {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 7px;
  color: #fff;
  font-size: 0.8125rem;
  font-weight: 700;
  flex: none;
}

.picker__name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.picker__open {
  color: var(--c-signal-button);
  font-size: 0.6875rem;
  font-weight: 600;
}

.remote__area {
  position: relative;
  overflow: hidden;
  border-radius: 10px;
}

.remote__empty {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 24px;
  color: var(--c-muted);
  font-size: var(--t-small);
  text-align: center;
}

.win {
  position: absolute;
  width: 72%;
  height: 64%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 10px;
  background: #fff;
  box-shadow:
    0 0 0 1px rgba(0, 0, 0, 0.08),
    0 18px 36px -18px rgba(30, 30, 30, 0.55);
  touch-action: none;
}

.win__bar {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 10px;
  flex: none;
  background: #f3f2ef;
  font-size: 0.6875rem;
  font-weight: 600;
  cursor: grab;
}

.win__bar span {
  margin-left: 8px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.win__bar i,
.win__close {
  width: 10px;
  height: 10px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: #d5d3ce;
  flex: none;
}

.win__close {
  background: #f26157;
  cursor: pointer;
}

.win__body {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding: 12px;
  min-height: 0;
}

.win__body i {
  display: block;
  height: 6px;
  border-radius: 3px;
  background: #e6e4df;
}

.k-code {
  background: #1f2228;
}

.k-code i {
  background: #3b4150;
}

.k-code i:nth-child(3n) {
  background: #7a5fd1;
}

.k-code i:nth-child(4n) {
  background: #3f8f6a;
}

.k-timeline {
  background: #26282d;
}

.k-timeline .preview {
  flex: 1;
  border-radius: 4px;
  background: linear-gradient(135deg, #4a5160, #2d2f35);
}

.k-timeline i {
  height: 12px;
  background: linear-gradient(90deg, #5a6ee0 0 30%, transparent 30% 34%, #d9893a 34% 62%, transparent 62% 66%, #5a6ee0 66%);
}

.k-tracks i {
  flex: 1;
  height: auto;
  background: repeating-linear-gradient(90deg, #c9c6bf 0 2px, transparent 2px 5px);
}

.k-files i {
  height: 14px;
  background: linear-gradient(90deg, #8fc1ee 0 14px, transparent 14px 22px, #e6e4df 22px);
}

.k-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 1px;
  padding: 0;
  background: #e6e4df;
}

.k-grid i {
  height: auto;
  border-radius: 0;
  background: #fff;
}

.k-grid i:nth-child(-n + 4) {
  background: #e6f2ea;
}

.remote__caption {
  margin-top: 12px;
  color: var(--c-muted);
  font-size: var(--t-small);
}

.win-enter-active {
  transition:
    opacity 0.25s var(--ease),
    transform 0.25s var(--ease);
}

.win-leave-active {
  transition: opacity 0.15s;
}

.win-enter-from {
  opacity: 0;
  transform: scale(0.92) translateX(-24px);
}

.win-leave-to {
  opacity: 0;
}

@container (max-width: 520px) {
  .remote {
    grid-template-columns: minmax(0, 1fr);
    height: auto;
  }

  .picker__apps {
    grid-template-columns: 1fr 1fr;
  }

  .remote__area {
    height: 240px;
  }

  .win {
    width: 82%;
  }
}
</style>
