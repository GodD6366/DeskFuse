<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import GlyphIcon from '../GlyphIcon.vue'
import clipboardShot from '../../assets/shots/clipboard.webp'
import inputShot from '../../assets/shots/input-wide.webp'

interface Capability {
  id: string
  name: string
  icon: string
  tagline: string
  badge?: string
  title: string
  body: string
  points: string[]
  note: string
  shot?: string
  shotAlt?: string
  flow?: [string, string, string, string]
  flowIcon?: string
}

const capabilities: Capability[] = [
  {
    id: 'input', name: '键鼠共享', icon: 'keyboard', tagline: '光标穿过屏幕边缘',
    title: '一套键鼠，划过所有屏幕',
    body: '光标移出这块屏幕，就出现在相邻那台电脑上；键盘输入也跟着过去。屏幕按桌上的真实位置摆放，不用记方向。',
    points: ['已配对、在线的电脑自动加入排布', '一个快捷键随时把控制收回本机', '光标只从两块屏幕相接的边过去'],
    note: '需要对应系统权限', shot: inputShot,
    shotAlt: 'DeskFuse 键鼠共享的实际屏幕排布界面',
  },
  {
    id: 'clipboard', name: '剪贴板', icon: 'clipboard', tagline: '复制一次，两边都能粘贴',
    title: '复制一次，两边都能粘贴',
    body: '文字、图片和原生剪贴板格式在电脑之间同步。历史按类型分组，图片里的文字也能搜。',
    points: ['保留原生格式，不只是纯文本', '剪贴板历史跨设备同步', '按类型分组，找得到以前复制的内容'],
    note: '剪贴板内容不会写进诊断文件', shot: clipboardShot,
    shotAlt: 'DeskFuse 剪贴板历史与预览的实际界面',
  },
  {
    id: 'mic', name: '麦克风', icon: 'mic', tagline: '把另一台的麦克风借过来', title: '让这台电脑多一个麦克风',
    body: '开会、录音或语音输入时，另一台 Mac 的麦克风变成这台电脑上的虚拟输入设备，不用把耳机或麦克风搬来搬去。',
    points: ['默认关闭，需要时再开', '这台电脑多出一个虚拟麦克风', '音频同样走局域网直连'],
    note: '每台设备可以单独开关', flow: ['另一台 Mac', '内置麦克风', '这台 Mac', '虚拟输入设备'], flowIcon: 'pulse',
  },
  {
    id: 'mouse', name: '鼠标按键', icon: 'mouse', tagline: '侧键和快捷键，跟着光标走', badge: 'Beta', title: '鼠标侧键，也跟着你换电脑',
    body: '把复制、切换桌面、Mission Control 这类快捷键绑到鼠标侧键上；换到另一台 Mac，按键映射也在。普通鼠标走系统按键，Logitech 扩展键走 HID++。',
    points: ['普通鼠标的系统按键与组合键', 'MX Master 3 / 3S 的手势键与模式键', 'G502 实体键可以附加 DeskFuse 快捷键'],
    note: 'Logitech 扩展键需要对应型号和输入监控授权', flow: ['你的鼠标', '侧键与快捷键', '当前 Mac', '映射跟着光标走'], flowIcon: 'layout',
  },
  {
    id: 'remote', name: '远程应用', icon: 'window', tagline: '高负载应用，留在另一台', badge: 'Beta', title: '高负载应用，跑在另一台 Mac 上',
    body: '把 Xcode、Final Cut、Excel 这类吃内存和显卡的应用放在另一台 Mac 运行，这台只负责窗口、输入和显示。默认开启独立虚拟屏，不会被那台电脑自己的桌面挡住。',
    points: ['每个应用一个独立窗口，不是整块屏幕投屏', '虚拟屏幕默认开启，可在设置里关闭', 'Windows 应用走 RemoteApp，不开放 RDP 端口'],
    note: 'macOS 窗口与 Windows RemoteApp；虚拟屏幕仍属实验性功能', flow: ['另一台 Mac', '应用在那里运行', '这台 Mac', '窗口在这里显示'], flowIcon: 'window',
  },
]

const active = ref('input')
const current = computed(() => capabilities.find((item) => item.id === active.value) ?? capabilities[0])

function selectTab(id: string) {
  active.value = id
  const url = new URL(window.location.href)
  if (id === 'input') url.searchParams.delete('capability')
  else url.searchParams.set('capability', id)
  window.history.replaceState(null, '', `${url.pathname}${url.search}${url.hash}`)
}

function onTabKey(event: KeyboardEvent, index: number) {
  const keys = ['ArrowRight', 'ArrowDown', 'ArrowLeft', 'ArrowUp', 'Home', 'End']
  if (!keys.includes(event.key)) return
  event.preventDefault()
  let next = index
  if (event.key === 'ArrowRight' || event.key === 'ArrowDown') next = (index + 1) % capabilities.length
  if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') next = (index - 1 + capabilities.length) % capabilities.length
  if (event.key === 'Home') next = 0
  if (event.key === 'End') next = capabilities.length - 1
  selectTab(capabilities[next].id)
  requestAnimationFrame(() => {
    document.querySelectorAll<HTMLButtonElement>('.cap__tab')[next]?.focus()
  })
}

onMounted(() => {
  const id = new URLSearchParams(window.location.search).get('capability')
  if (id && capabilities.some((item) => item.id === id)) active.value = id
})
</script>

<template>
  <section id="capabilities" class="cap section section--gray">
    <div class="page">
      <header class="section-heading">
        <h2>从光标开始，<br />连接整个桌面。</h2>
        <p>输入、内容、音频和应用，共用一条局域网连接。<br />每项能力都可独立开关。</p>
      </header>

      <div class="cap__tabs" role="tablist" aria-label="DeskFuse 能力">
        <button
          v-for="(item, i) in capabilities"
          :key="item.id"
          class="cap__tab"
          :class="{ 'is-active': active === item.id }"
          type="button"
          role="tab"
          :aria-selected="active === item.id"
          :aria-controls="active === item.id ? `cap-panel-${item.id}` : undefined"
          :tabindex="active === item.id ? 0 : -1"
          @click="selectTab(item.id)"
          @keydown="onTabKey($event, i)"
        >
          <GlyphIcon :name="item.icon" :size="17" />
          {{ item.name }}
        </button>
      </div>

      <Transition name="cap-fade" mode="out-in">
        <article
          :id="`cap-panel-${current.id}`"
          :key="current.id"
          class="cap__panel"
          role="tabpanel"
          :aria-label="current.name"
        >
          <div class="cap__copy">
            <p class="cap__tagline">
              {{ current.tagline }}<span v-if="current.badge" class="badge">{{ current.badge }}</span>
            </p>
            <h3>{{ current.title }}</h3>
            <p class="cap__body">{{ current.body }}</p>
            <ul class="cap__points">
              <li v-for="point in current.points" :key="point">
                <GlyphIcon name="check" :size="18" />{{ point }}
              </li>
            </ul>
            <p class="cap__note">{{ current.note }}</p>
          </div>

          <div class="cap__visual">
            <div v-if="current.shot" class="cap__shot" :class="`cap__shot--${current.id}`">
              <img :src="current.shot" :alt="current.shotAlt" loading="lazy" />
            </div>
            <div v-else-if="current.flow" class="cap__diagram" :aria-label="`${current.name}功能示意`">
              <div class="cap__flow">
                <div class="cap__flow-node">
                  <GlyphIcon :name="current.icon" :size="38" />
                  <strong>{{ current.flow[0] }}</strong>
                  <span>{{ current.flow[1] }}</span>
                </div>
                <div class="cap__flow-arrow" aria-hidden="true"><GlyphIcon name="arrow" :size="28" /></div>
                <div class="cap__flow-node">
                  <GlyphIcon :name="current.flowIcon || current.icon" :size="38" />
                  <strong>{{ current.flow[2] }}</strong>
                  <span>{{ current.flow[3] }}</span>
                </div>
              </div>
              <small>功能示意</small>
            </div>
          </div>
        </article>
      </Transition>
    </div>
  </section>
</template>

<style scoped>
.cap__tabs {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  width: fit-content;
  max-width: 100%;
  margin: 0 auto 44px;
  padding: 4px;
  border-radius: 980px;
  background: #e8e8ed;
}

.cap__tab {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  min-height: 42px;
  padding: 0 16px;
  border: 0;
  border-radius: 980px;
  background: transparent;
  color: var(--body);
  font: inherit;
  font-size: 0.875rem;
  white-space: nowrap;
  cursor: pointer;
  transition: background-color 0.18s var(--ease), color 0.18s var(--ease), box-shadow 0.18s var(--ease);
}

.cap__tab.is-active {
  background: #fff;
  box-shadow: 0 2px 8px rgb(0 0 0 / 9%);
  color: var(--ink);
  font-weight: 500;
}

.cap__tab:focus-visible {
  outline-offset: 1px;
}

.cap__panel {
  display: grid;
  grid-template-columns: minmax(0, 0.85fr) minmax(0, 1.15fr);
  align-items: center;
  gap: clamp(44px, 7vw, 88px);
  min-height: 480px;
  padding: clamp(32px, 5vw, 64px);
  border-radius: 12px;
  background: #fff;
}

.cap__tagline {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  color: var(--muted);
  font-size: 0.875rem;
  font-weight: 500;
  line-height: 1.6;
}

.cap__copy h3 {
  margin-top: 14px;
  font-size: var(--t-h3);
  line-height: 1.2;
}

.cap__body {
  margin-top: 20px;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.75;
}

.cap__points {
  display: grid;
  gap: 12px;
  margin: 24px 0 0;
  padding: 0;
  list-style: none;
}

.cap__points li {
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr);
  gap: 10px;
  color: var(--ink);
  font-size: 0.875rem;
  line-height: 1.65;
}

.cap__points svg {
  margin-top: 3px;
  color: var(--vermilion);
}

.cap__note {
  margin-top: 24px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.65;
}

.cap__visual,
.cap__shot,
.cap__diagram {
  min-width: 0;
}

.cap__shot {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 360px;
  padding: 20px;
  overflow: hidden;
  border-radius: 12px;
  background: var(--paper-2);
}

.cap__shot img {
  width: 100%;
  height: auto;
  border-radius: 8px;
  box-shadow: 0 14px 36px rgb(0 0 0 / 12%);
}

.cap__shot--input img {
  border-radius: 6px;
  box-shadow: 0 10px 28px rgb(0 0 0 / 9%);
}

.cap__diagram {
  display: grid;
  place-items: center;
  min-height: 360px;
  padding: 36px 24px 20px;
  border-radius: 12px;
  background: var(--paper-2);
}

.cap__flow {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 40px minmax(0, 1fr);
  align-items: center;
  gap: 16px;
  width: 100%;
}

.cap__flow-node {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  min-height: 180px;
  padding: 22px 14px;
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 10px 30px rgb(0 0 0 / 7%);
  text-align: center;
}

.cap__flow-node svg {
  margin-bottom: 18px;
  color: var(--ink);
}

.cap__flow-node strong {
  font-size: 1rem;
  font-weight: 600;
  line-height: 1.5;
}

.cap__flow-node span {
  margin-top: 5px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.6;
}

.cap__flow-arrow {
  color: var(--vermilion);
}

.cap__diagram > small {
  align-self: end;
  color: var(--muted);
  font-size: 0.75rem;
}

.cap-fade-enter-active,
.cap-fade-leave-active {
  transition: opacity 0.18s var(--ease), transform 0.18s var(--ease);
}

.cap-fade-enter-from {
  opacity: 0;
  transform: translateY(8px);
}

.cap-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

@media (hover: hover) and (pointer: fine) {
  .cap__tab:not(.is-active):hover {
    color: var(--ink);
    background: rgb(255 255 255 / 45%);
  }
}

@media (max-width: 900px) {
  .cap__tabs {
    justify-content: flex-start;
    width: 100%;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .cap__tabs::-webkit-scrollbar {
    display: none;
  }

  .cap__panel {
    grid-template-columns: 1fr;
    gap: 36px;
    min-height: 0;
  }

  .cap__copy {
    max-width: 620px;
  }
}

@media (max-width: 520px) {
  .cap__tabs {
    margin-bottom: 24px;
    border-radius: 14px;
  }

  .cap__tab {
    min-height: 44px;
    padding: 0 13px;
  }

  .cap__panel {
    gap: 28px;
    padding: 28px 22px;
  }

  .cap__shot,
  .cap__diagram {
    min-height: 260px;
  }

  .cap__shot {
    padding: 10px;
  }

  .cap__diagram {
    padding: 24px 12px 16px;
  }

  .cap__flow {
    grid-template-columns: minmax(0, 1fr) 24px minmax(0, 1fr);
    gap: 6px;
  }

  .cap__flow-node {
    min-height: 156px;
    padding: 18px 8px;
  }

  .cap__flow-node strong {
    font-size: 0.875rem;
  }

  .cap__flow-node span {
    font-size: 0.6875rem;
  }

  .cap__flow-arrow svg {
    width: 22px;
  }
}
</style>
