<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { asset } from './demos'

interface Shot {
  src: string
  alt: string
  caption: string
}

const shots: Record<string, Shot> = {
  clipboard: {
    src: 'assets/shot-clipboard.jpg',
    alt: 'DeskFuse 剪贴板界面',
    caption: '剪贴板历史：文本、图片、文件一目了然，支持图片内容搜索',
  },
  devices: {
    src: 'assets/shot-devices.jpg',
    alt: 'DeskFuse 设备界面',
    caption: '设备页：自动发现与手动配对，连接路由与传输活动全程可见',
  },
  screens: {
    src: 'assets/shot-screens.jpg',
    alt: 'DeskFuse 屏幕与键鼠界面',
    caption: '屏幕与键鼠：真实显示器布局自由摆放，共享键鼠一键开关',
  },
  remote: {
    src: 'assets/shot-remote.jpg',
    alt: 'DeskFuse 远程窗口界面',
    caption: '远程窗口：把另一台设备变成一块副屏，应用与文件随手可取',
  },
}

const tabs = [
  { key: 'clipboard', label: '剪贴板' },
  { key: 'devices', label: '设备' },
  { key: 'screens', label: '屏幕与键鼠' },
  { key: 'remote', label: '远程窗口' },
]

const activeKey = ref('clipboard')
const fading = ref(false)
const glider = ref<HTMLElement>()
const tabRefs = ref<Record<string, HTMLElement>>({})

const current = ref<Shot>({ ...shots.clipboard })

function setTabRef(key: string) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return (el: any) => {
    if (el instanceof HTMLElement) tabRefs.value[key] = el
  }
}

async function moveGlider() {
  await nextTick()
  const tab = tabRefs.value[activeKey.value]
  const g = glider.value
  if (!tab || !g) return
  g.style.left = `${tab.offsetLeft}px`
  g.style.width = `${tab.offsetWidth}px`
}

function select(key: string) {
  if (key === activeKey.value) return
  activeKey.value = key
  fading.value = true
  setTimeout(() => {
    current.value = { ...shots[key] }
    fading.value = false
  }, 180)
  moveGlider()
}

onMounted(() => {
  moveGlider()
  window.addEventListener('resize', moveGlider)
})
onBeforeUnmount(() => window.removeEventListener('resize', moveGlider))
</script>

<template>
  <section class="section" id="screenshots">
    <div class="section-inner">
      <div class="section-head" v-reveal>
        <p class="eyebrow">界面</p>
        <h2>原生 SwiftUI 打造，<br />为 macOS 而生。</h2>
        <p class="lead">每一个界面都是原生体验，与系统浑然一体。</p>
      </div>

      <div v-reveal>
        <div class="shot-tabs" role="tablist">
          <span ref="glider" class="tab-glider" aria-hidden="true"></span>
          <button
            v-for="t in tabs"
            :key="t.key"
            :ref="setTabRef(t.key)"
            class="shot-tab"
            :class="{ active: activeKey === t.key }"
            role="tab"
            @click="select(t.key)"
          >
            {{ t.label }}
          </button>
        </div>

        <div class="shot-stage">
          <div class="shot-frame shot-window-lg">
            <img :src="asset(current.src)" :alt="current.alt" :class="{ fading }" />
          </div>
          <p class="shot-caption">{{ current.caption }}</p>
        </div>
      </div>
    </div>
  </section>
</template>
