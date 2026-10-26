<script setup lang="ts">
import { useDownloads } from '../composables/useDownloads'

defineProps<{ cue?: boolean }>()

const { tag, prerelease, arm64, arm64Size } = useDownloads()
</script>

<template>
  <div class="hero-copy">
    <h1><span>桌上几台电脑，</span><span>当一台来用。</span></h1>
    <p class="hero-copy__lead">
      一套键鼠划过所有屏幕；另一台 Mac 的 App 开成这台的窗口；Mac mini 用上 MacBook 的麦克风。剪贴板和文件也跟着走，全部在你自己的电脑之间直连。
    </p>
    <div class="hero-copy__actions">
      <a class="btn btn--primary" :href="arm64.url">下载 Apple 芯片版</a>
      <a class="btn btn--quiet" href="#install">其他安装方式</a>
    </div>
    <p class="hero-copy__meta">
      {{ tag }}{{ prerelease ? ' 预发行' : '' }}，需要 macOS 14 或更新<template v-if="arm64Size">，{{ arm64Size }}</template>
    </p>
    <p v-if="cue" class="hero-copy__cue">往下滚动，镜头会推进到每一块屏幕里</p>
  </div>
</template>

<style scoped>
h1 {
  font-size: var(--t-h1);
  font-weight: 700;
  line-height: 1.1;
  letter-spacing: -0.03em;
}

h1 span {
  display: block;
  white-space: nowrap;
}

.hero-copy__lead {
  max-width: 28em;
  margin-top: 24px;
  color: var(--c-muted);
  font-size: var(--t-lead);
  line-height: 1.65;
}

.hero-copy__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  margin-top: 32px;
}

.hero-copy__meta {
  margin-top: 14px;
  color: var(--c-muted);
  font-size: var(--t-small);
}

.hero-copy__cue {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 48px;
  color: var(--c-muted);
  font-size: var(--t-small);
}

.hero-copy__cue::before {
  content: '';
  width: 14px;
  height: 22px;
  border-radius: 7px;
  box-shadow: inset 0 0 0 1.5px currentColor;
  background: radial-gradient(circle at 50% 30%, currentColor 0 2px, transparent 2.5px);
  flex: none;
}

@media (max-width: 720px) {
  .hero-copy__lead {
    font-size: 1.125rem;
  }
}
</style>
