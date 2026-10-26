<script setup lang="ts">
import { onBeforeUnmount, reactive, ref } from 'vue'

defineProps<{ active?: boolean }>()

interface Item {
  id: string
  name: string
  folder: boolean
  count?: number
}

interface Transfer {
  key: number
  item: Item
  progress: number
}

const items: Item[] = [
  { id: 'design', name: '设计稿', folder: true, count: 24 },
  { id: 'report', name: '季度报告.pdf', folder: false },
  { id: 'photos', name: '旅行照片', folder: true, count: 128 },
]

const transfers = reactive<Transfer[]>([])
const shelf = reactive<Item[]>([])
const selected = ref<Item | null>(null)
const over = ref<'send' | 'shelf' | null>(null)
const box = ref<HTMLElement | null>(null)
const ghost = reactive({ on: false, x: 0, y: 0, item: null as Item | null })
let key = 0
let raf = 0
let pending: { item: Item; sx: number; sy: number } | null = null

function send(item: Item) {
  const t = { key: key++, item, progress: 0 }
  transfers.unshift(t)
  if (transfers.length > 3) transfers.pop()
  const duration = item.folder ? 600 + (item.count ?? 1) * 12 : 900
  const start = performance.now()
  const tick = (now: number) => {
    const live = transfers.find((x) => x.key === t.key)
    if (!live) return
    live.progress = Math.min(1, (now - start) / duration)
    if (live.progress < 1) raf = requestAnimationFrame(tick)
  }
  raf = requestAnimationFrame(tick)
}

function stash(item: Item) {
  if (!shelf.some((s) => s.id === item.id)) shelf.push(item)
}

function sendShelf() {
  shelf.splice(0).forEach(send)
}

function act(target: 'send' | 'shelf', item: Item | null) {
  if (!item) return
  if (target === 'send') send(item)
  else stash(item)
  selected.value = null
}

// 拖动：位置相对演示框计算，屏幕处在缩放过的镜头里也不会错位
function zoneAt(x: number, y: number) {
  const hit = document.elementsFromPoint(x, y).find((el) => (el as HTMLElement).dataset?.drop)
  return ((hit as HTMLElement | undefined)?.dataset.drop as 'send' | 'shelf' | undefined) ?? null
}

function onDown(event: PointerEvent, item: Item) {
  if (event.button !== 0) return
  pending = { item, sx: event.clientX, sy: event.clientY }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp, { once: true })
}

function onMove(event: PointerEvent) {
  if (!pending || !box.value) return
  if (!ghost.on && Math.hypot(event.clientX - pending.sx, event.clientY - pending.sy) < 6) return
  const r = box.value.getBoundingClientRect()
  ghost.on = true
  ghost.item = pending.item
  ghost.x = event.clientX - r.left
  ghost.y = event.clientY - r.top
  over.value = zoneAt(event.clientX, event.clientY)
}

function onUp(event: PointerEvent) {
  window.removeEventListener('pointermove', onMove)
  if (ghost.on) {
    const zone = zoneAt(event.clientX, event.clientY)
    if (zone && ghost.item) act(zone, ghost.item)
  } else if (pending) {
    selected.value = selected.value?.id === pending.item.id ? null : pending.item
  }
  ghost.on = false
  ghost.item = null
  over.value = null
  pending = null
}

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  window.removeEventListener('pointermove', onMove)
})
</script>

<template>
  <div class="sc-host">
    <div class="sc">
      <div class="sc__copy">
        <h2>文件和整个文件夹，直接发过去</h2>
        <p>名称和目录结构原样保留。暂存抽屉先帮你接住文件，切完窗口再整批拖出去，或者一次发给另一台电脑。</p>
        <ul class="sc__list">
          <li>手机扫一下二维码，就能从浏览器把照片传到电脑上，不用装 App</li>
          <li>收到的文件默认放进「下载」，接收位置可以在设置里改</li>
        </ul>
      </div>

      <div class="sc__demo">
        <div ref="box" class="files">
          <div class="files__finder">
            <span class="files__device">MacBook Pro</span>
            <ul>
              <li v-for="item in items" :key="item.id">
                <button
                  type="button"
                  class="files__item"
                  :class="{ 'is-selected': selected?.id === item.id }"
                  :aria-pressed="selected?.id === item.id"
                  @pointerdown="onDown($event, item)"
                  @keydown.enter.prevent="selected = item"
                >
                  <span class="glyph" :class="{ 'is-folder': item.folder }" aria-hidden="true"></span>
                  <span>{{ item.name }}<small v-if="item.count">{{ item.count }} 个文件</small></span>
                </button>
              </li>
            </ul>
            <p class="files__hint">把文件拖到目标上，或者先点文件再点目标</p>
          </div>

          <div class="files__targets">
            <button
              type="button"
              class="target"
              data-drop="send"
              :class="{ 'is-over': over === 'send', 'is-ready': selected }"
              @click="act('send', selected)"
            >
              <span class="target__title">发送到 Mac mini</span>
              <ul v-if="transfers.length" class="transfers">
                <li v-for="t in transfers" :key="t.key">
                  <span>{{ t.item.name }}</span>
                  <span class="bar"><i :style="{ width: `${t.progress * 100}%` }"></i></span>
                  <small>{{ t.progress < 1 ? '传输中' : t.item.folder ? `已存到 下载/${t.item.name}，${t.item.count} 个文件` : '已存到 下载' }}</small>
                </li>
              </ul>
            </button>

            <div class="shelf">
              <button
                type="button"
                class="target target--shelf"
                data-drop="shelf"
                :class="{ 'is-over': over === 'shelf', 'is-ready': selected }"
                @click="act('shelf', selected)"
              >
                <span class="target__title">暂存抽屉</span>
                <span class="shelf__items">
                  <span v-for="s in shelf" :key="s.id" class="chip">{{ s.name }}</span>
                  <span v-if="!shelf.length" class="shelf__empty">先放在这里</span>
                </span>
              </button>
              <button v-if="shelf.length" class="btn btn--quiet btn--small" type="button" @click="sendShelf">
                整批发到 Mac mini
              </button>
            </div>

            <div class="phone">
              <span class="qr" aria-hidden="true"></span>
              <span>手机扫码，从浏览器传照片</span>
            </div>
          </div>

          <div v-if="ghost.on && ghost.item" class="ghost" :style="{ left: `${ghost.x}px`, top: `${ghost.y}px` }">
            <span class="glyph" :class="{ 'is-folder': ghost.item.folder }"></span>{{ ghost.item.name }}
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.files {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.1fr);
  gap: 14px;
  padding: 14px;
  border-radius: 16px;
  background: #e3e1dc;
  user-select: none;
  -webkit-user-select: none;
}

.files__finder {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  border-radius: 12px;
  background: #fff;
}

.files__device {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--c-muted);
}

.files ul {
  display: grid;
  gap: 4px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.files__item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  font: inherit;
  font-size: 0.8125rem;
  text-align: left;
  cursor: grab;
  touch-action: none;
}

.files__item:hover {
  background: var(--c-paper);
}

.files__item.is-selected {
  background: var(--c-graphite);
  color: #fff;
}

.files__item small {
  display: block;
  font-size: 0.6875rem;
  opacity: 0.7;
}

.glyph {
  width: 26px;
  height: 30px;
  border-radius: 4px;
  background: linear-gradient(180deg, #fff 0 0) 6px 8px / 14px 2px no-repeat,
    linear-gradient(180deg, #fff 0 0) 6px 13px / 14px 2px no-repeat, #8b8f98;
  flex: none;
}

.glyph.is-folder {
  height: 22px;
  border-radius: 3px 5px 5px 5px;
  background: var(--c-graphite-2);
}

.files__hint {
  margin-top: auto;
  color: var(--c-muted);
  font-size: 0.75rem;
}

.files__targets {
  display: grid;
  gap: 10px;
  align-content: start;
}

.target {
  display: grid;
  gap: 8px;
  width: 100%;
  min-height: 64px;
  padding: 12px 14px;
  border: 0;
  border-radius: 12px;
  background: var(--c-graphite);
  color: #fff;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    background 0.15s,
    box-shadow 0.15s;
}

.target.is-ready,
.target.is-over {
  box-shadow: 0 0 0 3px var(--c-signal);
}

.target.is-over {
  background: var(--c-signal-button);
}

.target__title {
  font-size: 0.875rem;
  font-weight: 600;
}

.transfers {
  gap: 8px;
}

.transfers li {
  display: grid;
  gap: 3px;
  font-size: 0.75rem;
}

.transfers small {
  color: rgba(255, 255, 255, 0.7);
  font-size: 0.6875rem;
}

.bar {
  height: 4px;
  overflow: hidden;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.2);
}

.bar i {
  display: block;
  height: 100%;
  background: var(--c-signal);
}

.shelf {
  display: grid;
  gap: 8px;
}

.target--shelf {
  background: #fff;
  color: var(--c-ink);
}

.shelf__items {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chip {
  padding: 2px 8px;
  border-radius: 6px;
  background: var(--c-desk);
  font-size: 0.75rem;
}

.shelf__empty {
  color: var(--c-muted);
  font-size: 0.75rem;
}

.phone {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.6);
  font-size: 0.8125rem;
}

.qr {
  width: 34px;
  height: 34px;
  border-radius: 4px;
  background:
    conic-gradient(var(--c-graphite) 25%, transparent 0 50%, var(--c-graphite) 0 75%, transparent 0) 0 0 / 8px 8px,
    #fff;
  box-shadow: inset 0 0 0 3px #fff;
  flex: none;
}

.ghost {
  position: absolute;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-radius: 8px;
  background: #fff;
  box-shadow: 0 12px 28px -12px rgba(30, 30, 30, 0.6);
  font-size: 0.8125rem;
  pointer-events: none;
  transform: translate(-50%, -60%) rotate(-3deg);
  z-index: 5;
}

.ghost .glyph {
  width: 18px;
  height: 20px;
}

@container (max-width: 640px) {
  .files {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
