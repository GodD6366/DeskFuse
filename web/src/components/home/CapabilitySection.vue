<script setup lang="ts">
import { computed, ref } from 'vue'

interface Capability {
  id: string
  name: string
  tagline: string
  badge: string
  tags: string[]
  title: string
  body: string
  points: string[]
  note: string
}

const capabilities: Capability[] = [
  {
    id: 'input',
    name: '键鼠共享',
    tagline: '光标穿过屏幕边缘',
    badge: '内置',
    tags: ['输入', '屏幕排布'],
    title: '一套键鼠，划过所有屏幕',
    body: '光标移出这块屏幕，就出现在相邻那台电脑上；键盘输入也跟着过去。屏幕按你桌上的真实位置摆放，不用记方向。',
    points: [
      '已配对、在线的电脑自动加入排布',
      '一个快捷键随时把控制收回本机',
      '光标只从两块屏幕相接的边过去',
    ],
    note: '需要辅助功能授权',
  },
  {
    id: 'clipboard',
    name: '剪贴板',
    tagline: '复制一次，两边都能粘贴',
    badge: '内置',
    tags: ['剪贴板', '历史'],
    title: '复制一次，两边都能粘贴',
    body: '文字、图片和原生剪贴板格式在电脑之间同步。历史按类型分组，图片里的文字也能搜。',
    points: ['保留原生格式，不只是纯文本', '剪贴板历史跨设备同步', '按类型分组，找得到以前复制的内容'],
    note: '剪贴板内容不会写进诊断文件',
  },
  {
    id: 'mic',
    name: '麦克风共享',
    tagline: '把另一台的麦克风借过来',
    badge: '内置',
    tags: ['音频', '虚拟设备'],
    title: '让这台电脑多一个麦克风',
    body: '开会、录音或语音输入时，另一台 Mac 的麦克风变成这台电脑上的虚拟输入设备，不用把耳机或麦克风搬来搬去。',
    points: ['默认关闭，需要时再开', '这台电脑多出一个虚拟麦克风', '音频同样走局域网直连'],
    note: '每台设备可以单独开关',
  },
  {
    id: 'mouse',
    name: '鼠标按键',
    tagline: '侧键和快捷键，跟着光标走',
    badge: 'Beta',
    tags: ['鼠标', '快捷键'],
    title: '鼠标侧键，也跟着你换电脑',
    body: '为了不用在两台 Mac 上重复记快捷键，把复制、切换桌面、Mission Control 这类键盘快捷键绑到鼠标侧键上；换到另一台 Mac，按键映射也在。普通鼠标走系统按键，Logitech 的扩展键走 HID++。',
    points: [
      '普通鼠标的系统按键与组合键',
      'MX Master 3 / 3S 的手势键与模式键',
      'G502 实体键可以附加 DeskFuse 快捷键',
    ],
    note: 'Beta：Logitech 扩展键需要对应型号和输入监控授权',
  },
  {
    id: 'remote',
    name: '远程应用',
    tagline: '高负载应用，留在另一台',
    badge: 'Beta',
    tags: ['远程', '虚拟屏幕'],
    title: '高负载应用，跑在另一台 Mac 上',
    body: '把 Xcode、Final Cut、Excel 这类吃内存和显卡的应用放在另一台 Mac 运行，这台只负责窗口、输入和显示。默认开启虚拟屏幕：每个应用一块独立屏，不会被那台电脑自己的桌面挡住。',
    points: [
      '不是整块屏幕投屏，每个应用一个独立窗口',
      '虚拟屏幕默认开启，可在设置里关闭',
      '中文输入法候选框跟着窗口',
      'Windows 应用走 RemoteApp，不开放 RDP 端口',
    ],
    note: 'Beta：macOS 窗口与 Windows RemoteApp；虚拟屏幕仍属实验性功能',
  },
]

const active = ref(capabilities[0].id)
const current = computed(() => capabilities.find((item) => item.id === active.value) ?? capabilities[0])
const number = (index: number) => String(index + 1).padStart(2, '0')

function selectTab(id: string) {
  active.value = id
}

function onTabKey(event: KeyboardEvent, index: number) {
  const keys = ['ArrowRight', 'ArrowDown', 'ArrowLeft', 'ArrowUp', 'Home', 'End']
  if (!keys.includes(event.key)) return
  event.preventDefault()
  const list = event.currentTarget instanceof HTMLElement ? event.currentTarget.parentElement : null
  if (!list) return
  let next = index
  if (event.key === 'ArrowRight' || event.key === 'ArrowDown') next = (index + 1) % capabilities.length
  if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') next = (index - 1 + capabilities.length) % capabilities.length
  if (event.key === 'Home') next = 0
  if (event.key === 'End') next = capabilities.length - 1
  active.value = capabilities[next].id
  requestAnimationFrame(() => {
    const buttons = list.querySelectorAll<HTMLButtonElement>('.cap__item')
    buttons[next]?.focus()
  })
}
</script>

<template>
  <section id="capabilities" class="cap section">
    <div class="page">
      <header class="cap__head">
        <h2>把几台电脑，连成一张桌子</h2>
        <p>
          DeskFuse 不是远程桌面，也不是一块共享屏幕。它把每台电脑的输入、剪贴板、文件、音频和桌面接进同一条局域网连接，让它们像同一台机器上的不同部分。下面按做出来的顺序介绍。
        </p>
      </header>

      <div class="cap__spread">
        <div class="cap__contents" role="tablist" aria-label="DeskFuse 能力">
          <span class="cap__contents-label" aria-hidden="true">BUILD ORDER</span>
          <button
            v-for="(item, i) in capabilities"
            :key="item.id"
            class="cap__item"
            :class="{ 'is-active': active === item.id }"
            type="button"
            role="tab"
            :aria-selected="active === item.id"
            :aria-controls="active === item.id ? `cap-panel-${item.id}` : undefined"
            :tabindex="active === item.id ? 0 : -1"
            @click="selectTab(item.id)"
            @keydown="onTabKey($event, i)"
          >
            <span class="cap__num">{{ number(i) }}</span>
            <span class="cap__name">{{ item.name }}</span>
            <small>{{ item.tagline }}</small>
          </button>
        </div>

        <Transition name="cap-fade" mode="out-in">
          <article
            :id="`cap-panel-${current.id}`"
            :key="current.id"
            class="cap__article"
            role="tabpanel"
            :aria-label="current.name"
          >
            <div class="cap__article-head">
              <span>{{ current.name }} / {{ current.badge }}</span>
              <span>{{ current.tags.join(' · ') }}</span>
            </div>
            <h3>{{ current.title }}</h3>
            <p class="cap__lead">{{ current.body }}</p>
            <ol class="cap__points">
              <li v-for="(point, i) in current.points" :key="point">
                <span>{{ number(i) }}</span>
                {{ point }}
              </li>
            </ol>
            <div class="cap__foot">
              <p>{{ current.note }}</p>
              <a href="#download">下载 DeskFuse <span aria-hidden="true">→</span></a>
            </div>
          </article>
        </Transition>

        <aside class="cap__margin" aria-label="边注">
          <span class="cap__pull">“光标从屏幕边缘过去，键盘跟着人走。”</span>
          <span class="cap__margin-note">本地直连<br />不经过云端<br />NO CLOUD</span>
        </aside>
      </div>
    </div>
  </section>
</template>

<style scoped>
.cap {
  border-top: 1px solid var(--rule-strong);
}

.cap__head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 340px);
  gap: 24px;
  align-items: start;
  margin-bottom: 44px;
  padding-top: 14px;
}

.cap__head h2 {
  grid-column: 1;
  font-size: var(--t-h2);
}

.cap__head p {
  grid-column: 2;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.8;
}

.cap__spread {
  display: grid;
  grid-template-columns: 230px minmax(0, 1fr) 190px;
  gap: clamp(24px, 4vw, 56px);
  align-items: start;
}

.cap__contents {
  display: grid;
  border-top: 1px solid var(--rule-strong);
}

.cap__contents-label {
  padding: 9px 0;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.08em;
}

.cap__item {
  display: grid;
  grid-template-columns: 30px minmax(0, 1fr);
  gap: 2px 10px;
  padding: 14px 0;
  border: 0;
  border-top: 1px solid var(--rule);
  background: transparent;
  color: var(--ink);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.cap__item.is-active {
  background: var(--paper-2);
}

.cap__num {
  grid-row: span 2;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.04em;
  line-height: 1.7;
}

.cap__item.is-active .cap__num {
  color: var(--vermilion);
}

.cap__name {
  font-size: 0.9375rem;
  font-weight: 600;
  line-height: 1.5;
}

.cap__item small {
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.55;
}

.cap__item:hover .cap__name {
  color: var(--vermilion);
}

.cap__item:focus-visible {
  outline-offset: -2px;
}

.cap__article {
  min-width: 0;
}

.cap__article-head {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  padding: 9px 0;
  border-top: 1px solid var(--rule-strong);
  border-bottom: 1px solid var(--rule);
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.04em;
  line-height: 1.5;
  text-transform: uppercase;
}

.cap__article h3 {
  margin-top: 24px;
  font-size: clamp(2rem, 1.4rem + 2vw, 2.9rem);
  line-height: 1.18;
}

.cap__lead {
  max-width: 36em;
  margin-top: 18px;
  color: var(--body);
  font-size: 1.0625rem;
  line-height: 1.85;
}

.cap__points {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0;
  margin: 26px 0 0;
  padding: 0;
  border-top: 1px solid var(--rule);
  list-style: none;
}

.cap__points li {
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr);
  gap: 8px;
  padding: 12px 16px 12px 0;
  border-bottom: 1px solid var(--rule);
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.65;
}

.cap__points li:nth-child(2n) {
  padding-left: 16px;
  padding-right: 0;
  border-left: 1px solid var(--rule);
}

.cap__points li:last-child:nth-child(odd) {
  grid-column: 1 / -1;
  padding-left: 0;
  border-left: 0;
}

.cap__points span {
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  line-height: 1.8;
}

.cap__foot {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px 24px;
  margin-top: 20px;
}

.cap__foot p {
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.6;
}

.cap__foot a {
  color: var(--vermilion);
  font-size: 0.875rem;
  font-weight: 600;
  text-decoration: none;
  white-space: nowrap;
}

.cap__foot a:hover {
  text-decoration: underline;
  text-underline-offset: 4px;
}

.cap__margin {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 28px;
  padding-top: 9px;
  border-top: 1px solid var(--rule-strong);
}

.cap__pull {
  max-width: 12em;
  color: var(--vermilion);
  font-family: var(--font-display);
  font-size: 1.1rem;
  line-height: 1.6;
  text-align: right;
}

.cap__margin-note {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.08em;
  line-height: 1.8;
  text-align: right;
}

.cap-fade-enter-active,
.cap-fade-leave-active {
  transition:
    opacity 0.16s var(--ease),
    transform 0.16s var(--ease);
}

.cap-fade-enter-from {
  opacity: 0;
  transform: translateY(4px);
}

.cap-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

@media (max-width: 1080px) {
  .cap__spread {
    grid-template-columns: 210px minmax(0, 1fr);
  }

  .cap__margin {
    display: none;
  }
}

@media (max-width: 1080px) {
  .cap__head {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .cap__head h2,
  .cap__head p {
    grid-column: 1;
  }

  .cap__head p {
    max-width: 38em;
  }
}

@media (max-width: 760px) {
  .cap__spread {
    grid-template-columns: 1fr;
    gap: 24px;
  }

  .cap__contents {
    display: flex;
    overflow-x: auto;
    border-top: 1px solid var(--rule-strong);
    border-bottom: 1px solid var(--rule);
    scrollbar-width: none;
    -webkit-mask-image: linear-gradient(90deg, #000 calc(100% - 26px), transparent);
    mask-image: linear-gradient(90deg, #000 calc(100% - 26px), transparent);
  }

  .cap__contents::-webkit-scrollbar {
    display: none;
  }

  .cap__contents-label {
    display: none;
  }

  .cap__item {
    display: flex;
    align-items: baseline;
    gap: 7px;
    min-width: max-content;
    padding: 12px 16px 11px;
    border-top: 0;
    border-left: 0;
    border-bottom: 2px solid transparent;
  }

  .cap__item.is-active {
    background: transparent;
    border-bottom-color: var(--vermilion);
  }

  .cap__item small {
    display: none;
  }

  .cap__num {
    grid-row: auto;
  }

  .cap__points {
    grid-template-columns: 1fr;
  }

  .cap__points li:nth-child(2n) {
    padding-left: 0;
    border-left: 0;
  }
}

@media (max-width: 640px) {
  .cap__article h3 {
    margin-top: 20px;
  }

  .cap__lead {
    font-size: 1rem;
  }
}
</style>
