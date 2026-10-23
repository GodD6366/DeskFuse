<script setup lang="ts">
import { computed } from 'vue'
import { asset } from './demos'
import { useLatestRelease, FALLBACK_RELEASE_TAG } from '../composables/useLatestRelease'
import { useCopy } from '../composables/useCopy'
import { HOMEBREW_ONE_LINE } from '../config/install'
import { ARM64_DISK_IMAGE, INTEL_DISK_IMAGE, macAppDownloadUrl } from '../config/distribution'

const { release } = useLatestRelease()
const { copied, copy } = useCopy()

const arm64Url = computed(() => macAppDownloadUrl(release.value?.assets, ARM64_DISK_IMAGE))
const intelUrl = computed(() => macAppDownloadUrl(release.value?.assets, INTEL_DISK_IMAGE))
const usesAppPackage = computed(() => arm64Url.value.includes('-update.zip'))
const badgeText = computed(() =>
  release.value?.tag
    ? `最新版本 ${release.value.tag} · macOS arm64 / Intel · Windows x64`
    : `最新版本 ${FALLBACK_RELEASE_TAG} · macOS arm64 / Intel · Windows x64`,
)
</script>

<template>
  <section class="hero">
    <div v-reveal>
      <p class="hero-badge"><span class="pulse-dot"></span>{{ badgeText }}</p>
      <h1 class="hero-title">在所有设备间<br />自由流转。</h1>
      <p class="hero-sub">
        DeskFuse 是 macOS 优先的原生桌面协作工具 —— 剪贴板同步、文件传输、暂存抽屉、键鼠共享，
        由 Rust 引擎与 SwiftUI 打造，经 QUIC 加密，只在你的可信设备之间运行。
      </p>
      <div class="hero-cta">
        <a class="btn btn-primary btn-lg" :href="arm64Url">下载 Apple Silicon 版</a>
        <a class="link-chevron" :href="intelUrl">下载 Intel 版</a>
      </div>
      <div class="hero-install">
        <span class="hero-install-prompt" aria-hidden="true">$</span>
        <code>{{ HOMEBREW_ONE_LINE }}</code>
        <button class="hero-copy" :class="{ copied }" @click="copy(HOMEBREW_ONE_LINE)">
          {{ copied ? '已复制 ✓' : '复制' }}
        </button>
      </div>
      <p class="hero-note">
        macOS 14+ · {{ usesAppPackage ? '打开应用后按窗口安装 · ' : '' }}Developer ID 签名与公证
      </p>
    </div>

    <div class="hero-shot" v-reveal>
      <div class="shot-frame">
        <img :src="asset('assets/shot-clipboard.jpg')" alt="DeskFuse 剪贴板界面" />
      </div>
    </div>
  </section>
</template>
