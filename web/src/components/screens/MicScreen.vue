<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = defineProps<{ active?: boolean }>()

/*
 * 波形默认由示例声音驱动；访客点按钮后改用自己的麦克风。
 * 麦克风数据只在本页换算成音量，不录制、不上传，离开这块屏幕就停止。
 */

type MicState = 'demo' | 'live' | 'denied' | 'unsupported'

const state = ref<MicState>('demo')
const level = ref(0)
const canvas = ref<HTMLCanvasElement | null>(null)

const HISTORY = 72
const history: number[] = new Array(HISTORY).fill(0)
let raf = 0
let visible = false
let stream: MediaStream | null = null
let audio: AudioContext | null = null
let analyser: AnalyserNode | null = null
let samples: Float32Array<ArrayBuffer> | null = null
let observer: IntersectionObserver | undefined
let lastPush = 0

/** 示例声音：音节起伏，句子之间只有短暂停顿，像在说话 */
function demoLevel(now: number) {
  const t = now / 1000
  const phrase = 0.3 + 0.7 * (0.5 + 0.5 * Math.sin(t * 1.3))
  const syllable = Math.abs(Math.sin(t * 7.7 + Math.sin(t * 2.1) * 2))
  return Math.min(1, phrase * syllable * 0.85 + Math.random() * 0.05)
}

function liveLevel() {
  if (!analyser || !samples) return 0
  analyser.getFloatTimeDomainData(samples)
  let sum = 0
  for (const s of samples) sum += s * s
  return Math.min(1, Math.sqrt(sum / samples.length) * 7)
}

function draw(now: number) {
  raf = requestAnimationFrame(draw)
  if (!visible) return
  const target = state.value === 'live' ? liveLevel() : demoLevel(now)
  level.value += (target - level.value) * 0.35
  if (now - lastPush > 45) {
    history.push(level.value)
    history.shift()
    lastPush = now
  }

  const c = canvas.value
  if (!c) return
  const ratio = window.devicePixelRatio || 1
  const w = c.clientWidth
  const h = c.clientHeight
  if (c.width !== Math.round(w * ratio) || c.height !== Math.round(h * ratio)) {
    c.width = Math.round(w * ratio)
    c.height = Math.round(h * ratio)
  }
  const ctx = c.getContext('2d')!
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
  ctx.clearRect(0, 0, w, h)
  const step = w / HISTORY
  const bar = Math.max(2, step * 0.5)
  history.forEach((v, i) => {
    const bh = Math.max(3, v * (h - 8))
    const x = i * step + (step - bar) / 2
    ctx.fillStyle = i > HISTORY - 10 ? '#f05a24' : `rgba(45, 47, 53, ${0.25 + (i / HISTORY) * 0.6})`
    ctx.beginPath()
    ctx.roundRect(x, (h - bh) / 2, bar, bh, bar / 2)
    ctx.fill()
  })
}

async function startMic() {
  if (!navigator.mediaDevices?.getUserMedia) {
    state.value = 'unsupported'
    return
  }
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } })
    audio = new AudioContext()
    analyser = audio.createAnalyser()
    analyser.fftSize = 1024
    samples = new Float32Array(analyser.fftSize)
    audio.createMediaStreamSource(stream).connect(analyser)
    state.value = 'live'
  } catch {
    stopMic()
    state.value = 'denied'
  }
}

function stopMic() {
  stream?.getTracks().forEach((t) => t.stop())
  audio?.close()
  stream = null
  audio = null
  analyser = null
  samples = null
  if (state.value === 'live') state.value = 'demo'
}

function toggle() {
  if (state.value === 'live') stopMic()
  else startMic()
}

watch(
  () => props.active,
  (on) => {
    if (!on) stopMic()
  },
)

onMounted(() => {
  // 先铺满一段示例波形，进入画面时不是一条直线
  const now = performance.now()
  for (let i = 0; i < HISTORY; i++) history[i] = demoLevel(now - (HISTORY - i) * 45)
  observer = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting
  })
  if (canvas.value) observer.observe(canvas.value)
  raf = requestAnimationFrame(draw)
})

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  observer?.disconnect()
  stopMic()
})

const segments = 14
</script>

<template>
  <div class="sc-host">
    <div class="sc">
      <div class="sc__copy">
        <span class="sc__beta">Beta</span>
        <h2>Mac mini 没有麦克风，就用 MacBook 的</h2>
        <p>
          一台电脑提供实体麦克风，另一台多出一个「DeskFuse 麦克风」。开会、录音、语音输入时选它就行；共享期间它会自动设成系统默认输入。
        </p>
        <ul class="sc__list">
          <li>已配对的电脑直接可用，不用逐台授权</li>
          <li>网络抖动时自动补上丢掉的音频包</li>
          <li>DeskFuse 只转发声音，不录音、不保存</li>
        </ul>
      </div>

      <div class="sc__demo">
        <div class="mic">
          <div class="mic__route">
            <div class="node">
              <b>MacBook Pro</b>
              <span>提供 MacBook Pro 麦克风</span>
            </div>
            <div class="link" :style="{ '--level': level.toFixed(3) }" aria-hidden="true"><i></i></div>
            <div class="node">
              <b>Mac mini</b>
              <span>多出 DeskFuse 麦克风</span>
            </div>
          </div>

          <canvas ref="canvas" class="mic__wave" aria-hidden="true"></canvas>

          <div class="meeting">
            <div class="meeting__row">
              <span class="meeting__label">麦克风</span>
              <span class="meeting__select">DeskFuse 麦克风（来自 MacBook Pro）</span>
            </div>
            <div class="meeting__row">
              <span class="meeting__label">输入音量</span>
              <span class="meeting__meter" aria-hidden="true">
                <i v-for="n in segments" :key="n" :class="{ on: level * segments * 1.3 >= n }"></i>
              </span>
            </div>
          </div>

          <div class="mic__try">
            <button class="btn btn--quiet btn--small" type="button" @click="toggle">
              {{ state === 'live' ? '停止使用我的麦克风' : '用我的麦克风试试' }}
            </button>
            <p v-if="state === 'live'">正在用你的麦克风驱动波形。声音只在这个网页里换算成音量，不会上传或保存。</p>
            <p v-else-if="state === 'denied'">没有拿到麦克风权限，继续播放示例声音。可以在浏览器地址栏里重新允许。</p>
            <p v-else-if="state === 'unsupported'">这个浏览器不能读取麦克风，继续播放示例声音。</p>
            <p v-else>现在是示例声音。</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mic {
  display: grid;
  gap: 18px;
  padding: 22px;
  border-radius: 16px;
  background: #e3e1dc;
}

.mic__route {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(40px, 0.6fr) minmax(0, 1fr);
  align-items: center;
}

.node {
  display: grid;
  gap: 2px;
  padding: 12px 14px;
  border-radius: 10px;
  background: var(--c-graphite);
  color: #fff;
}

.node:last-child {
  background: linear-gradient(180deg, #ff7442, var(--c-signal) 70%);
}

.node b {
  font-size: 0.9375rem;
}

.node span {
  color: rgba(255, 255, 255, 0.78);
  font-size: 0.75rem;
}

.link {
  position: relative;
  height: 4px;
  overflow: hidden;
}

.link i {
  position: absolute;
  inset: 0 -24px 0 0;
  background: repeating-linear-gradient(90deg, var(--c-signal) 0 10px, transparent 10px 18px);
  opacity: calc(0.35 + var(--level) * 0.9);
  animation: flow 0.6s linear infinite;
}

@keyframes flow {
  to {
    transform: translateX(18px);
  }
}

.mic__wave {
  width: 100%;
  height: clamp(72px, 12cqi, 120px);
}

.meeting {
  display: grid;
  gap: 10px;
  padding: 14px 16px;
  border-radius: 10px;
  background: #fff;
  font-size: 0.8125rem;
}

.meeting__row {
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}

.meeting__label {
  color: var(--c-muted);
}

.meeting__select {
  padding: 4px 10px;
  border-radius: 6px;
  box-shadow: inset 0 0 0 1px var(--c-line);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meeting__meter {
  display: flex;
  gap: 3px;
}

.meeting__meter i {
  flex: 1;
  height: 8px;
  border-radius: 2px;
  background: var(--c-desk);
  transition: background 0.08s;
}

.meeting__meter i.on {
  background: var(--c-signal);
}

.mic__try {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 14px;
}

.mic__try p {
  flex: 1;
  min-width: 14em;
  color: var(--c-muted);
  font-size: 0.8125rem;
  line-height: 1.5;
}

@media (prefers-reduced-motion: reduce) {
  .link i {
    animation: none;
  }
}
</style>
