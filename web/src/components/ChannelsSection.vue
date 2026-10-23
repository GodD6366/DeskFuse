<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import ClipboardScene from './scenes/ClipboardScene.vue'
import TransferScene from './scenes/TransferScene.vue'
import ControlScene from './scenes/ControlScene.vue'
import SecurityScene from './scenes/SecurityScene.vue'

interface Channel {
  key: 'clipboard' | 'transfer' | 'control' | 'security'
  index: string
  title: string
  tag: string
  description: string
  specs: string[]
}

const channels: Channel[] = [
  {
    key: 'clipboard',
    index: '01',
    title: '剪贴板',
    tag: '同步',
    description: '复制即到达。文本、图片与原生格式保持原样，历史按类型整理，连图片里的内容也能搜。',
    specs: ['文本 / 图片 / 原生格式', '跨设备历史', '图片内容搜索'],
  },
  {
    key: 'transfer',
    index: '02',
    title: '文件',
    tag: '传输',
    description: '发送队列和 240 × 240 暂存抽屉。拖进来，整批拖出去；文件夹结构原样保留，缓存有上限。',
    specs: ['保留文件夹结构', '暂存抽屉', '缓存上限与清理回执'],
  },
  {
    key: 'control',
    index: '03',
    title: '屏幕与键鼠',
    tag: '控制',
    description: '把光标推过屏幕边缘，键鼠就跟着过去。真实显示器布局、独立开关和一键紧急收回。',
    specs: ['真实显示器布局', '键鼠共享独立开关', '紧急收回'],
  },
  {
    key: 'security',
    index: '04',
    title: '设备与安全',
    tag: '信任',
    description: '先确认，再传输。设备身份是唯一依据，局域网直连，公网通道默认关闭。',
    specs: ['对端确认配对', 'QUIC 设备间加密', '日志仅元数据与哈希'],
  },
]

const scenes = {
  clipboard: ClipboardScene,
  transfer: TransferScene,
  control: ControlScene,
  security: SecurityScene,
} as const

const activeIndex = ref(0)
const active = computed(() => channels[activeIndex.value] ?? channels[0])
const currentScene = computed(() => scenes[active.value.key])
const tabButtons = ref<HTMLButtonElement[]>([])

function select(index: number, focus = false) {
  const next = (index + channels.length) % channels.length
  activeIndex.value = next
  if (focus) {
    nextTick(() => tabButtons.value[next]?.focus())
  }
}

function onChannelRequest(event: Event) {
  const key = (event as CustomEvent<string>).detail
  const index = channels.findIndex((channel) => channel.key === key)
  if (index >= 0) select(index)
}

onMounted(() => window.addEventListener('deskfuse:select-channel', onChannelRequest))
onBeforeUnmount(() => window.removeEventListener('deskfuse:select-channel', onChannelRequest))

function onTabKeydown(event: KeyboardEvent, index: number) {
  if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
    event.preventDefault()
    select(index + 1, true)
  } else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
    event.preventDefault()
    select(index - 1, true)
  } else if (event.key === 'Home') {
    event.preventDefault()
    select(0, true)
  } else if (event.key === 'End') {
    event.preventDefault()
    select(channels.length - 1, true)
  }
}
</script>

<template>
  <section id="abilities" class="section channels" aria-labelledby="abilities-title">
    <div class="container">
      <header class="section-head">
        <div class="section-head__main">
          <p class="section-index">02 / 能力</p>
          <h2 id="abilities-title" v-reveal.title>
            一条链路，<br />
            四种用途。
          </h2>
        </div>
        <div class="section-head__aside" v-reveal="90">
          <p class="lead">
            从一次复制，到一整套键鼠。每一层都能单独开关，也可以只用你需要的部分。
          </p>
        </div>
      </header>

      <div class="channels__layout" v-reveal>
        <div
          class="channels__tabs"
          role="tablist"
          aria-label="DeskFuse 能力"
          aria-orientation="vertical"
        >
          <button
            v-for="(channel, index) in channels"
            :id="`channel-tab-${channel.key}`"
            :key="channel.key"
            :ref="(el) => { if (el) tabButtons[index] = el as HTMLButtonElement }"
            class="channels__tab"
            :class="{ 'is-active': activeIndex === index }"
            type="button"
            role="tab"
            :aria-selected="activeIndex === index"
            aria-controls="channel-panel"
            :tabindex="activeIndex === index ? 0 : -1"
            @click="select(index)"
            @keydown="onTabKeydown($event, index)"
          >
            <span class="channels__num">{{ channel.index }}</span>
            <span class="channels__name">{{ channel.title }}</span>
            <span class="channels__tag">{{ channel.tag }}</span>
          </button>
        </div>

        <div class="channels__panel">
          <div class="channels__panel-top">
            <span class="channels__panel-title">
              {{ active.index }} / {{ active.title }}
            </span>
            <span class="channels__panel-note">界面示意</span>
            <span class="channels__panel-signal" aria-hidden="true">
              <i></i><i></i><i></i><i></i><i></i>
            </span>
          </div>

          <div
            id="channel-panel"
            class="channels__screen"
            role="tabpanel"
            :aria-labelledby="`channel-tab-${active.key}`"
          >
            <Transition name="scene-swap" mode="out-in">
              <component :is="currentScene" :key="active.key" />
            </Transition>
          </div>

          <div class="channels__panel-foot">
            <p class="channels__desc">{{ active.description }}</p>
            <ul class="channels__specs" aria-label="特性">
              <li v-for="spec in active.specs" :key="spec">{{ spec }}</li>
            </ul>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.channels {
  background:
    radial-gradient(ellipse 45% 35% at 82% 42%, rgba(89, 243, 218, 0.055), transparent 70%),
    var(--bg);
}

.channels__layout {
  display: grid;
  grid-template-columns: minmax(260px, 0.36fr) minmax(0, 0.64fr);
  gap: clamp(28px, 5vw, 84px);
  align-items: start;
}

.channels__tabs {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--line);
}

.channels__tab {
  position: relative;
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr) auto;
  gap: 12px;
  align-items: center;
  width: 100%;
  padding: 21px 0;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  text-align: left;
  cursor: pointer;
  transition: color 0.22s ease;
}

.channels__tab::before {
  content: "";
  position: absolute;
  top: 50%;
  left: -16px;
  width: 7px;
  height: 7px;
  border: 1px solid var(--faint);
  border-radius: 50%;
  transform: translateY(-50%);
  transition:
    background-color 0.22s ease,
    border-color 0.22s ease,
    box-shadow 0.22s ease;
}

.channels__tab:hover {
  color: var(--ink);
}

.channels__tab.is-active {
  color: var(--ink);
}

.channels__tab.is-active::before {
  border-color: var(--aqua);
  background: var(--aqua);
  box-shadow: 0 0 12px rgba(89, 243, 218, 0.7);
}

.channels__num {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.06em;
}

.channels__tab.is-active .channels__num {
  color: var(--orange);
}

.channels__name {
  font-family: var(--display);
  font-size: clamp(24px, 2.3vw, 34px);
  line-height: 1.1;
}

.channels__tag {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8.5px;
  letter-spacing: 0.06em;
}

.channels__tab.is-active .channels__tag {
  color: var(--aqua);
}

.channels__panel {
  min-width: 0;
  border: 1px solid var(--line);
  background: var(--surface);
  clip-path: polygon(0 0, calc(100% - 18px) 0, 100% 18px, 100% 100%, 0 100%);
}

.channels__panel-top {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 13px 18px;
  border-bottom: 1px solid var(--line);
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.06em;
}

.channels__panel-title {
  color: var(--muted);
}

.channels__panel-note {
  margin-left: auto;
  color: var(--faint);
  font-size: 8.5px;
}

.channels__panel-signal {
  display: inline-flex;
  align-items: flex-end;
  gap: 2px;
  height: 10px;
}

.channels__panel-signal i {
  display: block;
  width: 2px;
  background: var(--aqua);
  opacity: 0.55;
}

.channels__panel-signal i:nth-child(1) {
  height: 3px;
}

.channels__panel-signal i:nth-child(2) {
  height: 5px;
}

.channels__panel-signal i:nth-child(3) {
  height: 9px;
}

.channels__panel-signal i:nth-child(4) {
  height: 6px;
}

.channels__panel-signal i:nth-child(5) {
  height: 2px;
}

.channels__screen {
  position: relative;
  aspect-ratio: 16 / 10;
  overflow: hidden;
  background:
    linear-gradient(rgba(207, 255, 243, 0.04) 1px, transparent 1px),
    linear-gradient(90deg, rgba(207, 255, 243, 0.04) 1px, transparent 1px),
    var(--bg-deep);
  background-size: 34px 34px;
}

.channels__panel-foot {
  display: grid;
  grid-template-columns: minmax(0, 1.2fr) minmax(180px, 0.8fr);
  gap: 20px;
  align-items: start;
  padding: 20px 22px 22px;
  border-top: 1px solid var(--line);
}

.channels__desc {
  max-width: 46ch;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.65;
}

.channels__specs {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  list-style: none;
}

.channels__specs li {
  padding: 4px 8px;
  border: 1px solid var(--line);
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8.5px;
  letter-spacing: 0.02em;
}

.scene-swap-enter-active,
.scene-swap-leave-active {
  transition:
    opacity 0.2s ease,
    clip-path 0.28s var(--ease-out);
}

.scene-swap-enter-from {
  opacity: 0;
  clip-path: inset(0 0 100% 0);
}

.scene-swap-leave-to {
  opacity: 0;
  clip-path: inset(100% 0 0 0);
}

@media (max-width: 1000px) {
  .channels__layout {
    grid-template-columns: minmax(0, 1fr);
    gap: 26px;
  }

  .channels__tabs {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border-top: 0;
    border-bottom: 1px solid var(--line);
  }

  .channels__tab {
    grid-template-columns: auto minmax(0, 1fr);
    gap: 7px;
    padding: 13px 10px;
    border-bottom: 0;
    border-right: 1px solid var(--line);
  }

  .channels__tab:last-child {
    border-right: 0;
  }

  .channels__tab::before {
    display: none;
  }

  .channels__num {
    grid-row: 1;
  }

  .channels__name {
    grid-column: 1 / -1;
    font-size: clamp(18px, 3vw, 25px);
  }

  .channels__tag {
    display: none;
  }

  .channels__tab.is-active {
    background: rgba(89, 243, 218, 0.055);
    box-shadow: inset 0 -2px 0 var(--aqua);
  }
}

@media (max-width: 720px) {
  .channels__tabs {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .channels__tab:nth-child(2) {
    border-right: 0;
  }

  .channels__tab:nth-child(-n + 2) {
    border-bottom: 1px solid var(--line);
  }

  .channels__panel-foot {
    grid-template-columns: minmax(0, 1fr);
    gap: 14px;
  }

  .channels__screen {
    aspect-ratio: 4 / 3;
  }
}

@media (max-width: 480px) {
  .channels__panel-top {
    padding-inline: 13px;
  }

  .channels__panel-note {
    display: none;
  }

  .channels__panel-foot {
    padding: 16px;
  }

  .channels__desc {
    font-size: 12.5px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .scene-swap-enter-active,
  .scene-swap-leave-active {
    transition: none;
  }
}
</style>
