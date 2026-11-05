<script setup lang="ts">
import { useDownloads } from '../../composables/useDownloads'
import clipboardShot from '../../assets/shots/clipboard.webp'

const { tag, prerelease, arm64 } = useDownloads()

const contents = [
  { index: '01', title: '键鼠共享', note: '光标从屏幕边缘过去' },
  { index: '02', title: '剪贴板', note: '复制一次，两边都能粘' },
  { index: '03', title: '麦克风', note: '让这台电脑多一个麦克风' },
  { index: '04', title: '鼠标按键', note: '侧键和快捷键跟着走' },
  { index: '05', title: '远程应用', note: '高负载应用留在另一台' },
]
</script>

<template>
  <section id="top" class="hero">
    <div class="page">
      <div class="hero__mast">
        <span>DESKFUSE / ISSUE 01</span>
        <span>LOCAL NETWORK / 2026</span>
      </div>

      <div class="hero__grid">
        <div class="hero__copy">
          <span class="eyebrow">本地桌面融合</span>
          <h1>桌上几台电脑，<br />当一台来用。</h1>
          <p class="hero__deck">
            一套键鼠划过所有屏幕，剪贴板和文件跟着走。另一台 Mac 的高负载 App 开成这台的窗口，默认走独立虚拟屏；Mac mini 也能用上 MacBook 的麦克风。
          </p>
          <div class="hero__actions">
            <a class="btn btn--primary" :href="arm64.url">下载 DeskFuse</a>
            <a class="hero__link" href="#capabilities">看看它能做什么 <span aria-hidden="true">→</span></a>
          </div>
          <p class="hero__meta">
            {{ tag }}{{ prerelease ? ' 预发行' : '' }} · macOS 14 或更新 · Apple 芯片与 Intel
          </p>
        </div>

        <div class="hero__plate-col">
          <figure class="hero__plate">
            <img
              :src="clipboardShot"
              width="2200"
              height="1394"
              fetchpriority="high"
              alt="DeskFuse 剪贴板界面：历史里保存着文字、图片和文件，右侧预览当前复制的内容"
            />
            <figcaption>
              <b>Fig. 01</b>
              剪贴板历史，文字、图片与文件都在同一条时间线上。
              <span>5922 ITEMS / THIS MAC</span>
            </figcaption>
          </figure>
          <span class="hero__margin" aria-hidden="true">不经过云端 / NO CLOUD</span>
        </div>
      </div>

      <div class="hero__contents">
        <a v-for="item in contents" :key="item.index" class="hero__content" href="#capabilities">
          <span>{{ item.index }}</span>
          <b>{{ item.title }}</b>
          <small>{{ item.note }}</small>
        </a>
      </div>

      <div class="hero__folio">
        <span>DESKFUSE / 001</span>
        <span>2 SCREENS · 1.0 MS · LOCAL</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.hero {
  padding-bottom: clamp(64px, 8vw, 110px);
  background: var(--paper);
}

.hero__mast {
  display: flex;
  justify-content: space-between;
  gap: 20px;
  padding: 12px 0 10px;
  border-bottom: 1px solid var(--rule-strong);
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.08em;
  line-height: 1.5;
}

.hero__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(0, 0.85fr);
  align-items: start;
  gap: clamp(32px, 5vw, 72px);
  padding-block: clamp(44px, 6vw, 78px) clamp(36px, 5vw, 64px);
}

.hero__copy {
  padding-top: 8px;
}

.hero h1 {
  max-width: 11em;
  margin-top: 14px;
  font-family: var(--font-display);
  font-size: clamp(3rem, 1.6rem + 4.4vw, 5.25rem);
  font-weight: 500;
  letter-spacing: 0;
  line-height: 1.08;
}

.hero__deck {
  max-width: 34em;
  margin-top: 26px;
  color: var(--body);
  font-size: clamp(1.0625rem, 1rem + 0.3vw, 1.25rem);
  line-height: 1.8;
}

.hero__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 24px;
  margin-top: 30px;
}

.hero__link {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding-bottom: 2px;
  border-bottom: 1px solid var(--vermilion);
  color: var(--vermilion);
  font-size: 0.9375rem;
  font-weight: 600;
  text-decoration: none;
}

.hero__link:hover {
  color: var(--vermilion-dark);
}

.hero__meta {
  margin-top: 19px;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  letter-spacing: 0.015em;
  line-height: 1.6;
}

.hero__plate-col {
  display: flex;
  align-items: stretch;
  gap: 14px;
}

.hero__plate {
  flex: 1;
  min-width: 0;
  padding: 6px;
  border: 1px solid var(--rule-strong);
  background: var(--paper);
}

.hero__plate img {
  width: 100%;
  height: auto;
  border: 1px solid var(--rule);
}

.hero__plate figcaption {
  display: grid;
  gap: 3px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.65;
}

.hero__plate figcaption b {
  color: var(--vermilion);
  font-family: var(--font-serif-en);
  font-size: 0.875rem;
  font-weight: 500;
}

.hero__plate figcaption span {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.02em;
}

.hero__margin {
  align-self: stretch;
  padding-top: 8px;
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.12em;
  line-height: 1.5;
  white-space: nowrap;
  writing-mode: vertical-rl;
}

.hero__contents {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  border-top: 1px solid var(--rule-strong);
  border-bottom: 1px solid var(--rule);
}

.hero__content {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 3px 10px;
  padding: 16px 18px 16px 0;
  border-right: 1px solid var(--rule);
  color: var(--ink);
  text-decoration: none;
}

.hero__content + .hero__content {
  padding-left: 18px;
}

.hero__content:last-child {
  border-right: 0;
}

.hero__content span {
  grid-row: span 2;
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.04em;
  line-height: 1.7;
}

.hero__content b {
  font-size: 0.9375rem;
  font-weight: 600;
  line-height: 1.5;
}

.hero__content small {
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.6;
}

.hero__content:hover b {
  color: var(--vermilion);
}

.hero__folio {
  display: flex;
  justify-content: space-between;
  gap: 20px;
  padding-top: 12px;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.5625rem;
  letter-spacing: 0.06em;
  line-height: 1.5;
}

@media (max-width: 960px) {
  .hero__grid {
    grid-template-columns: 1fr;
    gap: 34px;
  }

  .hero__plate-col {
    max-width: 640px;
  }

  .hero__contents {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .hero__content:nth-child(2n) {
    border-right: 0;
  }

  .hero__content:nth-child(n + 3) {
    border-top: 1px solid var(--rule);
  }

  .hero__content:nth-child(odd) {
    padding-left: 0;
  }

  .hero__content:nth-child(even) {
    padding-left: 18px;
  }
}

@media (max-width: 380px) {
  .hero h1 {
    font-size: 2.5rem;
  }
}

@media (max-width: 640px) {
  .hero__mast {
    font-size: 0.5625rem;
  }

  .hero__mast span:last-child {
    text-align: right;
  }

  .hero__grid {
    padding-top: 32px;
  }

  .hero h1 {
    margin-top: 12px;
  }

  .hero__deck {
    margin-top: 20px;
    font-size: 1rem;
  }

  .hero__actions {
    gap: 18px;
    margin-top: 24px;
  }

  .hero__meta {
    font-size: 0.625rem;
  }

  .hero__plate-col {
    gap: 0;
  }

  .hero__margin {
    display: none;
  }

  .hero__contents {
    grid-template-columns: 1fr;
  }

  .hero__content,
  .hero__content + .hero__content,
  .hero__content:nth-child(2n),
  .hero__content:nth-child(odd),
  .hero__content:nth-child(even) {
    padding: 12px 0;
    border-right: 0;
    border-top: 1px solid var(--rule);
  }

  .hero__content:first-child {
    border-top: 0;
  }

  .hero__folio {
    font-size: 0.5rem;
  }
}

@media (prefers-reduced-motion: no-preference) {
  .hero__plate {
    animation: plate-in 240ms var(--ease) both;
  }

  @keyframes plate-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
}
</style>
