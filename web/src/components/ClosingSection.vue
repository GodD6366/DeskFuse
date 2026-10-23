<script setup lang="ts">
import { computed } from 'vue'
import { useLatestRelease, FALLBACK_RELEASE_TAG } from '../composables/useLatestRelease'
import {
  ARM64_DISK_IMAGE,
  INTEL_DISK_IMAGE,
  REPOSITORY_URL,
  macAppDownloadUrl,
} from '../config/distribution'

const { release } = useLatestRelease()
const arm64Url = computed(() => macAppDownloadUrl(release.value?.assets, ARM64_DISK_IMAGE))
const intelUrl = computed(() => macAppDownloadUrl(release.value?.assets, INTEL_DISK_IMAGE))
const versionLabel = computed(() => release.value?.tag ?? FALLBACK_RELEASE_TAG)
const releasesUrl = `${REPOSITORY_URL}/releases`
</script>

<template>
  <section id="closing" class="closing" aria-labelledby="closing-title">
    <div class="closing__field" aria-hidden="true">
      <span class="closing__line"></span>
      <span class="closing__node closing__node--a"></span>
      <span class="closing__node closing__node--b"></span>
      <span class="closing__pulse"></span>
    </div>

    <div class="container closing__inner">
      <div class="closing__copy">
        <p class="section-index">06 / 接上</p>
        <h2 id="closing-title" v-reveal.title>
          桌面，从此<br />
          只有一张。
        </h2>
      </div>

      <div class="closing__side" v-reveal="100">
        <p class="lead">
          把第二台 Mac 接上，剩下的交给 DeskFuse。剪贴板、文件、屏幕和键鼠，共用一套桌面。
        </p>
        <div class="closing__actions">
          <a class="btn closing__cta" :href="arm64Url">下载 Apple Silicon 版</a>
          <a class="text-link" :href="intelUrl">Intel 版</a>
        </div>
        <div class="closing__meta">
          <span>{{ versionLabel }}</span>
          <span>macOS 14+</span>
          <a :href="releasesUrl" target="_blank" rel="noopener">全部版本</a>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.closing {
  position: relative;
  overflow: hidden;
  padding-block: clamp(100px, 13vw, 190px);
  border-top: 1px solid var(--line);
  background:
    radial-gradient(ellipse 55% 75% at 88% 50%, rgba(89, 243, 218, 0.09), transparent 65%),
    linear-gradient(135deg, #081713, #06100e 65%);
}

.closing__field {
  position: absolute;
  right: -6%;
  bottom: 13%;
  width: 58%;
  height: 1px;
  pointer-events: none;
}

.closing__line {
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent, rgba(89, 243, 218, 0.45) 32%, rgba(89, 243, 218, 0.12));
}

.closing__node {
  position: absolute;
  top: -4px;
  width: 9px;
  height: 9px;
  border: 1px solid rgba(89, 243, 218, 0.65);
  border-radius: 50%;
  background: var(--bg);
}

.closing__node--a {
  left: 30%;
}

.closing__node--b {
  right: 18%;
  border-color: var(--orange);
}

.closing__pulse {
  position: absolute;
  top: -3px;
  left: 30%;
  width: 7px;
  height: 7px;
  background: var(--aqua);
  box-shadow: 0 0 14px rgba(89, 243, 218, 0.8);
  animation: closing-pulse 5s var(--ease-in-out) infinite;
}

.closing__inner {
  position: relative;
  z-index: 1;
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(280px, 0.65fr);
  gap: clamp(36px, 7vw, 120px);
  align-items: end;
}

.closing__copy .section-index {
  margin-bottom: 24px;
}

.closing h2 {
  color: var(--ink);
  font-family: var(--display);
  font-weight: 400;
  font-size: clamp(62px, 9vw, 142px);
  line-height: 1.06;
  letter-spacing: -0.015em;
}

.closing__side {
  padding-bottom: 8px;
}

.closing__side .lead {
  max-width: 34ch;
}

.closing__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 16px 22px;
  margin-top: 28px;
}

.closing__cta {
  min-height: 52px;
  padding-inline: 24px 28px;
}

.closing__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 18px;
  margin-top: 18px;
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.03em;
}

.closing__meta a {
  border-bottom: 1px solid var(--line);
}

.closing__meta a:hover {
  color: var(--aqua);
  border-color: var(--aqua);
}

@keyframes closing-pulse {
  0%,
  10% {
    left: 30%;
    opacity: 0;
  }
  20% {
    opacity: 1;
  }
  75% {
    left: calc(82% - 7px);
    opacity: 1;
  }
  90%,
  100% {
    left: calc(82% - 7px);
    opacity: 0;
  }
}

@media (max-width: 900px) {
  .closing__inner {
    grid-template-columns: minmax(0, 1fr);
    gap: 36px;
  }

  .closing__field {
    right: 0;
    bottom: 8%;
    width: 100%;
  }

  .closing__side .lead {
    max-width: 48ch;
  }
}

@media (max-width: 560px) {
  .closing h2 {
    font-size: clamp(54px, 16vw, 88px);
  }

  .closing__actions {
    align-items: flex-start;
  }

  .closing__cta {
    width: 100%;
  }

  .closing__field {
    bottom: 4%;
  }
}

@media (prefers-reduced-motion: reduce) {
  .closing__pulse {
    animation: none;
    left: calc(82% - 7px);
  }
}
</style>
