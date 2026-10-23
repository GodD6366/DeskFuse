<script setup lang="ts">
import { computed } from 'vue'
import LinkScene from './LinkScene.vue'
import { useLatestRelease, FALLBACK_RELEASE_TAG } from '../composables/useLatestRelease'
import {
  ARM64_DISK_IMAGE,
  INTEL_DISK_IMAGE,
  macAppDownloadUrl,
} from '../config/distribution'

const { release } = useLatestRelease()

const arm64Url = computed(() => macAppDownloadUrl(release.value?.assets, ARM64_DISK_IMAGE))
const intelUrl = computed(() => macAppDownloadUrl(release.value?.assets, INTEL_DISK_IMAGE))
const versionLabel = computed(() => release.value?.tag ?? FALLBACK_RELEASE_TAG)

const selectChannel = (key: string) => {
  window.dispatchEvent(new CustomEvent('deskfuse:select-channel', { detail: key }))
}

const capabilities = [
  { id: 'clipboard', index: '01', title: '剪贴板', detail: '文本、图片、原生格式' },
  { id: 'transfer', index: '02', title: '文件与暂存', detail: '队列、抽屉、文件夹结构' },
  { id: 'control', index: '03', title: '屏幕与键鼠', detail: '真实布局，一键收回' },
  { id: 'security', index: '04', title: '本地链路', detail: 'QUIC 加密，默认局域网' },
]
</script>

<template>
  <section id="top" class="hero">
    <div class="hero__grid" aria-hidden="true"></div>

    <div class="container hero__stage">
      <div class="hero__copy">
        <div class="hero__meta">
          <span class="hero__status"><i aria-hidden="true"></i>macOS 14+</span>
          <span class="hero__meta-sep" aria-hidden="true"></span>
          <span>{{ versionLabel }} · 公开预发行</span>
        </div>

        <div class="hero__headline">
          <h1>
            <span class="hero__line"><span>多台 Mac，</span></span>
            <span class="hero__line hero__line--indent"><span>接成一台。</span></span>
          </h1>
          <p class="hero__sub">
            剪贴板、文件、屏幕与键鼠，在可信设备之间直接流转。<span class="nowrap">Rust 与 SwiftUI</span>
            原生打造，QUIC 加密，默认只在局域网直连。
          </p>

          <div class="hero__actions">
            <a class="btn hero__download" :href="arm64Url">下载 Apple Silicon 版</a>
            <a class="text-link hero__intel" :href="intelUrl">Intel 版</a>
            <a class="hero__install-link" href="#install">其他安装方式</a>
          </div>

          <p class="hero__note">
            Developer ID 签名与公证 · 不需要账号 · 局域网直连
          </p>
        </div>
      </div>

      <div class="hero__visual">
        <LinkScene />
        <span class="hero__visual-mark hero__visual-mark--tl" aria-hidden="true">LIVE / 02</span>
        <span class="hero__visual-mark hero__visual-mark--br" aria-hidden="true">LOCAL-FIRST</span>
      </div>
    </div>

    <nav class="container hero__rail" aria-label="核心能力">
      <a
        v-for="item in capabilities"
        :key="item.id"
        class="hero__rail-item"
        href="#abilities"
        @click="selectChannel(item.id)"
      >
        <span class="hero__rail-index">{{ item.index }}</span>
        <span class="hero__rail-body">
          <b>{{ item.title }}</b>
          <small>{{ item.detail }}</small>
        </span>
      </a>
    </nav>
  </section>
</template>

<style scoped>
.hero {
  position: relative;
  min-height: 100vh;
  min-height: 100svh;
  padding-top: var(--nav-h);
  overflow: clip;
  background:
    radial-gradient(ellipse 70% 55% at 88% 16%, rgba(30, 113, 97, 0.32), transparent 65%),
    radial-gradient(ellipse 45% 40% at 8% 82%, rgba(255, 106, 60, 0.06), transparent 70%),
    var(--bg);
}

.hero__grid {
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(rgba(207, 255, 243, 0.055) 1px, transparent 1px),
    linear-gradient(90deg, rgba(207, 255, 243, 0.055) 1px, transparent 1px);
  background-size: 88px 88px;
  background-position: center -1px;
  mask-image: linear-gradient(180deg, rgba(0, 0, 0, 0.7), transparent 78%);
  -webkit-mask-image: linear-gradient(180deg, rgba(0, 0, 0, 0.7), transparent 78%);
  pointer-events: none;
}

.hero__stage {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.1fr);
  align-items: stretch;
  min-height: clamp(570px, calc(100svh - var(--nav-h) - 118px), 820px);
}

.hero__copy {
  position: relative;
  z-index: 3;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  min-width: 0;
  padding: clamp(28px, 4vh, 48px) 32px clamp(32px, 5vh, 64px) 0;
}

.hero__meta {
  display: flex;
  align-items: center;
  gap: 12px;
  color: var(--muted);
  font-family: var(--mono);
  font-size: 10.5px;
  letter-spacing: 0.06em;
  animation: hero-fade 0.8s var(--ease-out) 0.65s both;
}

.hero__status {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--ink);
}

.hero__status i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--aqua);
  box-shadow: 0 0 9px rgba(89, 243, 218, 0.8);
  animation: hero-pulse 2.6s ease-in-out infinite;
}

.hero__meta-sep {
  width: 18px;
  height: 1px;
  background: var(--line-strong);
}

.hero__headline {
  position: relative;
  z-index: 2;
  padding-top: 60px;
}

h1 {
  width: 116%;
  color: var(--ink);
  font-family: var(--display);
  font-weight: 400;
  font-size: clamp(58px, 8.5vw, 132px);
  line-height: 1.04;
  letter-spacing: -0.012em;
  text-shadow: 0 2px 40px rgba(6, 16, 14, 0.55);
}

.hero__line {
  display: block;
  overflow: hidden;
  padding-bottom: 0.08em;
  margin-bottom: -0.08em;
}

.hero__line > span {
  display: block;
  transform: translateY(112%);
  animation: hero-line 1.05s var(--ease-out) 0.12s forwards;
}

.hero__line--indent {
  padding-left: 0.34em;
}

.hero__line--indent > span {
  animation-delay: 0.22s;
}

.hero__sub {
  max-width: 46ch;
  margin-top: clamp(22px, 3vh, 34px);
  color: var(--muted);
  font-size: clamp(15px, 1.25vw, 17px);
  line-height: 1.7;
  text-wrap: balance;
  animation: hero-fade 0.9s var(--ease-out) 0.5s both;
}

.hero__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 14px 22px;
  margin-top: 30px;
  animation: hero-fade 0.9s var(--ease-out) 0.58s both;
}

.hero__download {
  min-height: 52px;
  padding-inline: 26px 30px;
  font-size: 15px;
}

.hero__intel {
  border-bottom-color: var(--line-strong);
}

.hero__install-link {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 10.5px;
  letter-spacing: 0.04em;
  border-bottom: 1px solid transparent;
  transition:
    color 0.2s ease,
    border-color 0.2s ease;
}

.hero__install-link:hover {
  color: var(--aqua);
  border-color: var(--aqua);
}

.nowrap {
  white-space: nowrap;
}

.hero__note {
  margin-top: 16px;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9.5px;
  letter-spacing: 0.03em;
  animation: hero-fade 0.9s var(--ease-out) 0.66s both;
}

.hero__visual {
  position: relative;
  z-index: 1;
  min-width: 0;
  min-height: 500px;
  margin-right: calc(var(--gutter) * -1);
  padding: 16px 0 24px 18px;
  animation: hero-fade 1.2s var(--ease-out) 0.12s both;
}

.hero__visual::before {
  content: "";
  position: absolute;
  inset: 0 -10% 0 -16%;
  z-index: 2;
  pointer-events: none;
  background: linear-gradient(
    90deg,
    var(--bg) 0%,
    rgba(6, 16, 14, 0.82) 4%,
    rgba(6, 16, 14, 0.2) 16%,
    transparent 30%
  );
}

.hero__visual-mark {
  position: absolute;
  z-index: 3;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8.5px;
  letter-spacing: 0.13em;
  writing-mode: vertical-rl;
  pointer-events: none;
}

.hero__visual-mark--tl {
  top: 24px;
  right: -8px;
}

.hero__visual-mark--br {
  right: -8px;
  bottom: 30px;
}

.hero__rail {
  position: relative;
  z-index: 3;
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  border-top: 1px solid var(--line);
  animation: hero-fade 1s var(--ease-out) 0.78s both;
}

.hero__rail-item {
  display: flex;
  gap: 14px;
  min-width: 0;
  padding: 18px 20px 20px 0;
  border-right: 1px solid var(--line);
  transition: background-color 0.25s ease;
}

.hero__rail-item:not(:first-child) {
  padding-left: 20px;
}

.hero__rail-item:last-child {
  border-right: 0;
}

.hero__rail-item:hover {
  background: rgba(207, 255, 243, 0.035);
}

.hero__rail-index {
  padding-top: 2px;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.06em;
}

.hero__rail-item:hover .hero__rail-index {
  color: var(--orange);
}

.hero__rail-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.hero__rail-body b {
  color: var(--ink);
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
}

.hero__rail-body small {
  overflow: hidden;
  color: var(--faint);
  font-size: 11px;
  white-space: nowrap;
  text-overflow: ellipsis;
}

@keyframes hero-line {
  to {
    transform: translateY(0);
  }
}

@keyframes hero-fade {
  from {
    opacity: 0;
    transform: translateY(12px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}

@keyframes hero-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.35;
  }
}

@media (max-width: 1100px) {
  h1 {
    font-size: clamp(54px, 8.6vw, 92px);
  }

  .hero__copy {
    padding-right: 18px;
  }

  .hero__rail-body small {
    font-size: 10px;
  }
}

@media (max-width: 900px) {
  .hero {
    min-height: 0;
    padding-bottom: 0;
  }

  .hero__stage {
    grid-template-columns: minmax(0, 1fr);
    min-height: 0;
  }

  .hero__copy {
    padding: 28px 0 0;
  }

  .hero__headline {
    padding-top: 56px;
  }

  h1 {
    width: 100%;
    font-size: clamp(58px, 13.4vw, 112px);
  }

  .hero__visual {
    min-height: 0;
    height: clamp(340px, 64vw, 540px);
    margin: 26px 0 0;
    padding: 0;
    order: 2;
  }

  .hero__visual::before {
    inset: 0;
    background: linear-gradient(180deg, transparent 72%, var(--bg) 100%);
  }

  .hero__visual-mark {
    display: none;
  }

  .hero__rail {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    margin-top: 4px;
  }

  .hero__rail-item:nth-child(2) {
    border-right: 0;
  }

  .hero__rail-item:nth-child(-n + 2) {
    border-bottom: 1px solid var(--line);
  }

  .hero__rail-item:nth-child(3) {
    padding-left: 0;
  }
}

@media (max-width: 560px) {
  .hero__meta {
    flex-wrap: wrap;
    font-size: 9.5px;
  }

  .hero__headline {
    padding-top: 42px;
  }

  h1 {
    font-size: clamp(52px, 15.6vw, 84px);
  }

  .hero__actions {
    gap: 12px 18px;
  }

  .hero__download {
    width: 100%;
  }

  .hero__sub {
    font-size: 14.5px;
  }

  .hero__rail-item {
    padding: 14px 10px 16px 0;
  }

  .hero__rail-item:not(:first-child) {
    padding-left: 12px;
  }

  .hero__rail-index {
    display: none;
  }

  .hero__rail-body b {
    font-size: 12px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .hero__meta,
  .hero__sub,
  .hero__actions,
  .hero__note,
  .hero__visual,
  .hero__rail {
    animation: none;
  }

  .hero__line > span {
    animation: none;
    transform: none;
  }

  .hero__status i {
    animation: none;
  }
}
</style>
