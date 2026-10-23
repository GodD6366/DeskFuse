<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'

type ClipKind = 'link' | 'image' | 'folder'

interface ClipItem {
  id: string
  kind: ClipKind
  title: string
  meta: string
  detail: string
}

const items: ClipItem[] = [
  {
    id: 'link',
    kind: 'link',
    title: 'https://deskfuse.godd.cc',
    meta: '链接 · 25 bytes',
    detail: '在另一台设备直接粘贴打开',
  },
  {
    id: 'image',
    kind: 'image',
    title: '设计稿 · 首页.png',
    meta: '图片 · 2.4 MB',
    detail: '原图保留，不压缩',
  },
  {
    id: 'folder',
    kind: 'folder',
    title: '项目资料',
    meta: '文件夹 · 12 项',
    detail: '保留文件夹结构',
  },
]

const selectedId = ref('link')
const phase = ref<'idle' | 'sending' | 'received'>('idle')
const pasted = ref(false)

const selected = computed(() => items.find((item) => item.id === selectedId.value) ?? items[0])
const received = computed(() => phase.value === 'received')
const sending = computed(() => phase.value === 'sending')
const liveMessage = computed(() => {
  if (sending.value) return `正在发送${selected.value.title}`
  if (pasted.value) return '内容已粘贴到对端剪贴板'
  if (received.value) return `已到达对端：${selected.value.title}`
  return ''
})

const reducedMotion =
  typeof window !== 'undefined' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches

let timer: ReturnType<typeof setTimeout> | undefined

function choose(id: string) {
  if (sending.value) return
  selectedId.value = id
  phase.value = 'idle'
  pasted.value = false
}

function send() {
  if (sending.value) return
  pasted.value = false
  phase.value = 'sending'
  clearTimeout(timer)
  timer = setTimeout(
    () => {
      phase.value = 'received'
    },
    reducedMotion ? 0 : 720,
  )
}

function paste() {
  if (!received.value) return
  pasted.value = true
}

onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <section id="demo" class="section demo" aria-labelledby="demo-title">
    <div class="container">
      <header class="section-head">
        <div class="section-head__main">
          <p class="section-index">01 / 现场</p>
          <h2 id="demo-title" v-reveal.title>
            复制一次，<br />
            另一台已经在等你。
          </h2>
        </div>
        <div class="section-head__aside" v-reveal="90">
          <p class="lead">
            不用同一个 Apple 账号，也不用先上传。局域网内直连，文本、图片和文件保留原来的样子。
          </p>
        </div>
      </header>

      <div class="demo__frame" v-reveal>
        <div class="demo__frame-top">
          <span>剪贴板同步</span>
          <span class="demo__hint">选择一项，然后复制</span>
          <span class="demo__demo-badge">交互示意</span>
        </div>

        <div class="demo__stage">
          <div class="demo__machine demo__machine--source">
            <div class="demo__machine-head">
              <span class="demo__device"><i aria-hidden="true"></i>这台 Mac</span>
              <span class="mono">SOURCE</span>
            </div>

            <div class="demo__list" role="group" aria-label="选择要复制的剪贴板内容">
              <button
                v-for="item in items"
                :key="item.id"
                class="demo__item"
                :class="{ 'is-active': selectedId === item.id }"
                type="button"
                :aria-pressed="selectedId === item.id"
                @click="choose(item.id)"
              >
                <span class="demo__item-icon" :class="`is-${item.kind}`" aria-hidden="true">
                  <svg v-if="item.kind === 'link'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
                    <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
                  </svg>
                  <svg v-else-if="item.kind === 'image'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="3" y="3" width="18" height="18" rx="2.5" />
                    <circle cx="9" cy="9" r="1.8" />
                    <path d="m21 15-4-4L6 20" />
                  </svg>
                  <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                </span>
                <span class="demo__item-text">
                  <b>{{ item.title }}</b>
                  <i>{{ item.meta }}</i>
                </span>
                <span class="demo__item-check" aria-hidden="true">
                  <i></i>
                </span>
              </button>
            </div>

            <div class="demo__machine-foot">
              <button
                class="demo__send"
                type="button"
                :disabled="sending"
                @click="send"
              >
                <span v-if="sending" class="demo__send-dot" aria-hidden="true"></span>
                {{ sending ? '正在传输' : received ? '再复制一次' : '复制到另一台' }}
              </button>
              <span class="demo__sync-note">双向同步 · 仅可信设备</span>
            </div>
          </div>

          <div class="demo__bridge" aria-hidden="true">
            <span class="demo__wire"></span>
            <span class="demo__port demo__port--left"></span>
            <span class="demo__port demo__port--right"></span>
            <span class="demo__packet" :class="{ 'is-flying': sending }"></span>
            <span class="demo__bridge-label">LOCAL / QUIC</span>
          </div>

          <div class="demo__machine demo__machine--target" :class="{ 'is-receiving': sending }">
            <div class="demo__machine-head">
              <span class="demo__device"><i aria-hidden="true"></i>MacBook Pro 14</span>
              <span class="mono">TARGET</span>
            </div>

            <div class="demo__receive">
              <div v-if="!received" class="demo__waiting">
                <span class="demo__waiting-line" aria-hidden="true"></span>
                <span class="demo__waiting-text">
                  {{ sending ? '正在接收…' : '等待接收' }}
                </span>
                <span class="demo__waiting-sub">
                  {{ sending ? '内容正在通过本地链路传输' : '在左侧选择一项并复制' }}
                </span>
              </div>

              <div v-else class="demo__result">
                <span class="demo__arrived">已到达 · 来自这台 Mac</span>

                <div v-if="selected.kind === 'link'" class="demo__result-link">
                  <span class="mono">TEXT · 25 BYTES</span>
                  <p>https://deskfuse.godd.cc</p>
                  <span class="demo__result-detail">{{ selected.detail }}</span>
                </div>

                <div v-else-if="selected.kind === 'image'" class="demo__result-image">
                  <span class="demo__thumb" aria-hidden="true"><i></i></span>
                  <div>
                    <p>{{ selected.title }}</p>
                    <span class="demo__result-detail">{{ selected.meta }} · {{ selected.detail }}</span>
                  </div>
                </div>

                <div v-else class="demo__result-folder">
                  <span class="demo__folder-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
                      <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                    </svg>
                  </span>
                  <div>
                    <p>{{ selected.title }} /</p>
                    <span class="demo__result-detail">12 项 · 文件夹结构保留</span>
                  </div>
                </div>

                <div class="demo__result-actions">
                  <button class="demo__paste" type="button" @click="paste">
                    {{ pasted ? '已粘贴 ✓' : '粘贴到对端' }}
                  </button>
                  <span class="demo__result-state" aria-live="polite">
                    {{ pasted ? '内容已写入对端剪贴板' : '已同步到对端剪贴板' }}
                  </span>
                </div>
              </div>
            </div>

            <div class="demo__machine-foot demo__machine-foot--target">
              <span class="demo__history">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true">
                  <circle cx="12" cy="12" r="8" />
                  <path d="M12 7v5l3 2" />
                </svg>
                历史同步开启
              </span>
              <span class="mono">02 DEVICES</span>
            </div>
          </div>
        </div>

        <p class="demo__disclaimer">
          页面内的交互示意，不会连接真实设备。实际传输行为与格式支持以应用内为准。
        </p>
        <span class="sr-only" aria-live="polite">{{ liveMessage }}</span>
      </div>

      <ul class="demo__facts" v-reveal="80">
        <li>
          <b>双向同步</b>
          <span>哪一台复制都可以，两台都在等</span>
        </li>
        <li>
          <b>历史可搜</b>
          <span>按类型整理，图片内容也能搜索</span>
        </li>
        <li>
          <b>保留原格式</b>
          <span>文本、图片与原生剪贴板类型</span>
        </li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.demo {
  background:
    radial-gradient(ellipse 55% 45% at 15% 28%, rgba(89, 243, 218, 0.06), transparent 70%),
    var(--bg);
}

.demo__frame {
  position: relative;
  border: 1px solid var(--line);
  background:
    linear-gradient(180deg, rgba(207, 255, 243, 0.025), transparent 35%),
    var(--surface);
  clip-path: polygon(0 0, calc(100% - 22px) 0, 100% 22px, 100% 100%, 0 100%);
}

.demo__frame::before,
.demo__frame::after {
  content: "";
  position: absolute;
  width: 7px;
  height: 7px;
  border: 1px solid var(--aqua);
  pointer-events: none;
}

.demo__frame::before {
  top: 11px;
  right: 11px;
  border-right: 0;
  border-bottom: 0;
}

.demo__frame::after {
  bottom: 11px;
  left: 11px;
  border-top: 0;
  border-right: 0;
}

.demo__frame-top {
  display: flex;
  align-items: center;
  gap: 18px;
  padding: 14px 22px;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  font-family: var(--mono);
  font-size: 10px;
  letter-spacing: 0.06em;
}

.demo__hint {
  margin-left: auto;
  color: var(--faint);
}

.demo__demo-badge {
  padding: 3px 7px;
  border: 1px solid rgba(255, 106, 60, 0.3);
  color: var(--orange-soft);
  font-size: 9px;
}

.demo__stage {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 138px minmax(0, 1fr);
  align-items: stretch;
  padding: clamp(18px, 2.8vw, 36px);
}

.demo__machine {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border: 1px solid var(--line);
  background: rgba(4, 12, 10, 0.46);
  transition:
    border-color 0.35s ease,
    background-color 0.35s ease;
}

.demo__machine--target {
  border-color: rgba(89, 243, 218, 0.22);
}

.demo__machine--target.is-receiving {
  border-color: rgba(89, 243, 218, 0.55);
}

.demo__machine-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 15px 18px;
  border-bottom: 1px solid var(--line);
}

.demo__device {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: var(--ink);
  font-size: 13px;
  font-weight: 550;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.demo__device i {
  flex: none;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--aqua);
  box-shadow: 0 0 7px rgba(89, 243, 218, 0.6);
}

.demo__machine-head .mono {
  color: var(--faint);
  font-size: 8.5px;
  white-space: nowrap;
}

.demo__list {
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding: 14px;
}

.demo__item {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-width: 0;
  padding: 12px;
  border: 1px solid transparent;
  background: rgba(207, 255, 243, 0.035);
  text-align: left;
  cursor: pointer;
  transition:
    border-color 0.2s ease,
    background-color 0.2s ease;
}

.demo__item:hover {
  border-color: var(--line-strong);
  background: rgba(207, 255, 243, 0.055);
}

.demo__item.is-active {
  border-color: rgba(89, 243, 218, 0.42);
  background: rgba(89, 243, 218, 0.07);
}

.demo__item-icon {
  display: grid;
  place-items: center;
  flex: none;
  width: 32px;
  height: 32px;
  border: 1px solid var(--line);
  color: var(--aqua);
}

.demo__item-icon.is-link {
  color: var(--aqua);
}

.demo__item-icon.is-image {
  color: #a8b7ff;
}

.demo__item-icon.is-folder {
  color: var(--orange-soft);
}

.demo__item-icon svg {
  width: 15px;
  height: 15px;
}

.demo__item-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.demo__item-text b {
  overflow: hidden;
  color: var(--ink);
  font-size: 12.5px;
  font-weight: 550;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.demo__item-text i {
  overflow: hidden;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  font-style: normal;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.demo__item-check {
  display: grid;
  place-items: center;
  flex: none;
  width: 14px;
  height: 14px;
  margin-left: auto;
  border: 1px solid var(--line-strong);
  border-radius: 50%;
  transition: border-color 0.2s ease;
}

.demo__item-check i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--aqua);
  opacity: 0;
  transform: scale(0.5);
  transition:
    opacity 0.2s ease,
    transform 0.2s var(--ease-out);
}

.demo__item.is-active .demo__item-check {
  border-color: var(--aqua);
}

.demo__item.is-active .demo__item-check i {
  opacity: 1;
  transform: scale(1);
}

.demo__machine-foot {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-top: auto;
  padding: 14px;
  border-top: 1px solid var(--line);
}

.demo__send {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-height: 36px;
  padding: 0 15px;
  background: var(--aqua);
  color: var(--bg-deep);
  font-size: 12px;
  font-weight: 650;
  cursor: pointer;
  clip-path: polygon(0 0, calc(100% - 8px) 0, 100% 8px, 100% 100%, 0 100%);
  transition:
    background-color 0.2s ease,
    opacity 0.2s ease;
}

.demo__send:hover {
  background: var(--aqua-soft);
}

.demo__send:disabled {
  cursor: wait;
  opacity: 0.65;
}

.demo__send-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--bg-deep);
  animation: send-pulse 0.8s ease-in-out infinite;
}

.demo__sync-note {
  color: var(--faint);
  font-size: 10px;
}

/* bridge */

.demo__bridge {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
}

.demo__wire {
  position: absolute;
  top: 50%;
  right: 8px;
  left: 8px;
  height: 1px;
  background-image: linear-gradient(90deg, var(--line-strong) 55%, transparent 45%);
  background-size: 9px 1px;
}

.demo__port {
  position: absolute;
  top: calc(50% - 3px);
  width: 6px;
  height: 6px;
  border: 1px solid var(--line-strong);
  border-radius: 50%;
  background: var(--surface);
}

.demo__port--left {
  left: 5px;
  border-color: var(--orange);
}

.demo__port--right {
  right: 5px;
  border-color: var(--aqua);
}

.demo__packet {
  position: absolute;
  top: calc(50% - 4px);
  left: 8px;
  width: 8px;
  height: 8px;
  background: var(--orange);
  box-shadow: 0 0 14px rgba(255, 106, 60, 0.8);
  opacity: 0;
}

.demo__packet.is-flying {
  opacity: 1;
  animation: packet-x 0.72s var(--ease-in-out) forwards;
}

.demo__bridge-label {
  position: absolute;
  top: calc(50% + 14px);
  color: var(--faint);
  font-family: var(--mono);
  font-size: 7.5px;
  letter-spacing: 0.08em;
  white-space: nowrap;
}

/* receive pane */

.demo__receive {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 238px;
  padding: 14px;
}

.demo__waiting {
  display: flex;
  flex: 1;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 5px;
  min-height: 200px;
  border: 1px dashed var(--line);
  color: var(--faint);
  text-align: center;
}

.demo__waiting-line {
  width: 44px;
  height: 1px;
  margin-bottom: 10px;
  background: var(--line-strong);
}

.demo__waiting-text {
  color: var(--muted);
  font-size: 12px;
}

.demo__waiting-sub {
  font-size: 10px;
}

.demo__result {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 16px;
  padding: 6px 2px 2px;
}

.demo__arrived {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--aqua);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.05em;
}

.demo__arrived::before {
  content: "";
  width: 5px;
  height: 5px;
  background: var(--aqua);
  box-shadow: 0 0 8px rgba(89, 243, 218, 0.7);
}

.demo__result-link {
  padding: 16px;
  border: 1px solid rgba(89, 243, 218, 0.22);
  background: rgba(89, 243, 218, 0.045);
}

.demo__result-link .mono {
  color: var(--faint);
  font-size: 8px;
}

.demo__result-link p {
  margin: 8px 0;
  color: var(--aqua);
  font-family: var(--mono);
  font-size: 14px;
  line-height: 1.5;
  word-break: break-all;
}

.demo__result-detail {
  color: var(--faint);
  font-size: 10px;
}

.demo__result-image {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px;
  border: 1px solid var(--line);
}

.demo__thumb {
  position: relative;
  flex: none;
  width: 64px;
  height: 48px;
  overflow: hidden;
  background:
    linear-gradient(135deg, rgba(89, 243, 218, 0.65), rgba(70, 105, 255, 0.5) 55%, rgba(255, 106, 60, 0.45));
}

.demo__thumb::after {
  content: "";
  position: absolute;
  inset: 0;
  background: repeating-linear-gradient(
    -35deg,
    transparent 0 7px,
    rgba(4, 12, 10, 0.14) 7px 8px
  );
}

.demo__result-image p,
.demo__result-folder p {
  color: var(--ink);
  font-size: 13px;
  font-weight: 550;
}

.demo__result-image .demo__result-detail,
.demo__result-folder .demo__result-detail {
  display: block;
  margin-top: 5px;
}

.demo__result-folder {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px;
  border: 1px solid var(--line);
}

.demo__folder-icon {
  display: grid;
  place-items: center;
  flex: none;
  width: 44px;
  height: 44px;
  border: 1px solid rgba(255, 106, 60, 0.28);
  color: var(--orange);
}

.demo__folder-icon svg {
  width: 20px;
  height: 20px;
}

.demo__result-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px 14px;
  margin-top: auto;
}

.demo__paste {
  min-height: 32px;
  padding: 0 13px;
  border: 1px solid rgba(89, 243, 218, 0.38);
  color: var(--aqua);
  font-size: 11.5px;
  cursor: pointer;
  transition:
    background-color 0.2s ease,
    color 0.2s ease;
}

.demo__paste:hover {
  background: rgba(89, 243, 218, 0.09);
}

.demo__result-state {
  color: var(--faint);
  font-size: 9.5px;
}

.demo__machine-foot--target {
  justify-content: space-between;
}

.demo__history {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--muted);
  font-size: 10px;
}

.demo__history svg {
  width: 12px;
  height: 12px;
  color: var(--aqua);
}

.demo__machine-foot--target .mono {
  color: var(--faint);
  font-size: 8px;
}

.demo__disclaimer {
  padding: 0 22px 16px;
  color: var(--faint);
  font-size: 9px;
}

.demo__facts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0;
  margin-top: 28px;
  border-top: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
  list-style: none;
}

.demo__facts li {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 18px 24px 18px 0;
  border-right: 1px solid var(--line);
}

.demo__facts li:not(:first-child) {
  padding-left: 24px;
}

.demo__facts li:last-child {
  border-right: 0;
}

.demo__facts b {
  color: var(--ink);
  font-size: 13px;
  font-weight: 600;
}

.demo__facts span {
  color: var(--faint);
  font-size: 11.5px;
}

@keyframes packet-x {
  0% {
    left: 8px;
    opacity: 0;
  }
  12% {
    opacity: 1;
  }
  85% {
    opacity: 1;
  }
  100% {
    left: calc(100% - 16px);
    opacity: 0;
  }
}

@keyframes send-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.25;
  }
}

@media (max-width: 900px) {
  .demo__stage {
    grid-template-columns: minmax(0, 1fr);
    gap: 0;
  }

  .demo__bridge {
    height: 54px;
  }

  .demo__wire {
    top: 5px;
    bottom: 5px;
    left: 50%;
    width: 1px;
    height: auto;
    right: auto;
    background-image: linear-gradient(180deg, var(--line-strong) 55%, transparent 45%);
    background-size: 1px 9px;
  }

  .demo__port--left {
    top: 3px;
    left: calc(50% - 3px);
  }

  .demo__port--right {
    top: auto;
    right: auto;
    bottom: 3px;
    left: calc(50% - 3px);
  }

  .demo__packet {
    top: 5px;
    left: calc(50% - 4px);
  }

  .demo__packet.is-flying {
    animation-name: packet-y;
  }

  .demo__bridge-label {
    top: 50%;
    left: calc(50% + 14px);
    transform: translateY(-50%);
  }

  .demo__facts {
    grid-template-columns: minmax(0, 1fr);
  }

  .demo__facts li,
  .demo__facts li:not(:first-child) {
    padding: 14px 0;
    border-right: 0;
    border-bottom: 1px solid var(--line);
  }

  .demo__facts li:last-child {
    border-bottom: 0;
  }
}

@keyframes packet-y {
  0% {
    top: 5px;
    opacity: 0;
  }
  12% {
    opacity: 1;
  }
  85% {
    opacity: 1;
  }
  100% {
    top: calc(100% - 13px);
    opacity: 0;
  }
}

@media (max-width: 620px) {
  .demo__frame-top {
    padding-inline: 16px;
  }

  .demo__hint {
    display: none;
  }

  .demo__demo-badge {
    margin-left: auto;
  }

  .demo__stage {
    padding: 12px;
  }

  .demo__machine-head {
    padding-inline: 14px;
  }

  .demo__list {
    padding: 10px;
  }

  .demo__machine-foot {
    flex-wrap: wrap;
    padding: 12px;
  }

  .demo__sync-note {
    font-size: 9px;
  }

  .demo__disclaimer {
    padding-inline: 16px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .demo__packet.is-flying,
  .demo__send-dot {
    animation: none;
  }
}
</style>
