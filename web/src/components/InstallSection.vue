<script setup lang="ts">
import { computed } from 'vue'
import { useLatestRelease, assetUrl } from '../composables/useLatestRelease'
import { useCopy } from '../composables/useCopy'
import { HOMEBREW_ONE_LINE, HOMEBREW_SCRIPT, HOMEBREW_UPGRADE } from '../config/install'
import {
  ARM64_DISK_IMAGE,
  INTEL_DISK_IMAGE,
  fallbackAssetUrl,
  macAppDownloadUrl,
} from '../config/distribution'

const { release } = useLatestRelease()
const { copied, copy } = useCopy()

const arm64Url = computed(() => macAppDownloadUrl(release.value?.assets, ARM64_DISK_IMAGE))
const intelUrl = computed(() => macAppDownloadUrl(release.value?.assets, INTEL_DISK_IMAGE))
const checksumsUrl = computed(() => assetUrl(release.value, 'SHA256SUMS', fallbackAssetUrl('SHA256SUMS')))
const usesAppPackage = computed(() => arm64Url.value.includes('-update.zip'))

function downloadLabel(url: string, chip: string): string {
  return url.includes('-update.zip') ? `${chip}应用包` : `${chip}磁盘映像`
}

const platforms: [string, string, string][] = [
  ['macOS 14+ · Apple Silicon', '主平台', 'ok'],
  ['macOS 26+', '原生 Liquid Glass', 'ok'],
  ['Windows x64', '实验性离线安装器', 'warn'],
  ['Intel Mac', 'x86_64', 'ok'],
  ['Linux', '不在首发范围', 'muted'],
]
</script>

<template>
  <section class="section" id="install">
    <div class="section-inner">
      <div class="section-head" v-reveal>
        <p class="eyebrow">安装</p>
        <h2>两种方式，一分钟上手。</h2>
      </div>
      <div class="install-grid">
        <div class="install-card" v-reveal>
          <h3>Homebrew <span class="tag">推荐</span></h3>
          <p>macOS 14+ · Apple Silicon，一条命令装好，随 brew 更新。</p>
          <div class="code-block">
            <button class="copy-btn" :class="{ copied }" @click="copy(HOMEBREW_ONE_LINE)">
              {{ copied ? '已复制 ✓' : '复制' }}
            </button>
            <pre><code>{{ HOMEBREW_SCRIPT }}</code></pre>
          </div>
          <p class="install-note">升级：<code>{{ HOMEBREW_UPGRADE }}</code></p>
        </div>

        <div class="install-card" v-reveal>
          <h3>下载应用</h3>
          <p>选择芯片版本下载{{ usesAppPackage ? '应用包' : '' }}。打开里面的 DeskFuse.app，按安装窗口装进「应用程序」。</p>
          <ol class="steps">
            <li><a :href="arm64Url">{{ downloadLabel(arm64Url, 'Apple Silicon（arm64）') }}</a></li>
            <li><a :href="intelUrl">{{ downloadLabel(intelUrl, 'Intel（x86_64）') }}</a></li>
            <li><a :href="checksumsUrl">下载 SHA256SUMS 校验文件</a></li>
            <li>运行 <code>shasum -a 256 -c SHA256SUMS</code> 校验完整性</li>
            <li v-if="usesAppPackage">打开压缩包里的 DeskFuse.app，在安装窗口点「点击安装」</li>
            <li v-else>打开磁盘映像里的 DeskFuse.app，在安装窗口点「点击安装」</li>
            <li>按提示允许局域网访问；键鼠共享另需辅助功能授权</li>
          </ol>
          <p class="install-note">公开预发行经 Developer ID 签名并公证。已经安装的应用会在后台自动更新，不会再出现这个窗口。</p>
        </div>
      </div>

      <div class="platforms" v-reveal>
        <div v-for="[name, status, kind] in platforms" :key="name" class="platform">
          <span class="platform-name">{{ name }}</span>
          <span class="platform-status" :class="kind">{{ status }}</span>
        </div>
      </div>
    </div>
  </section>
</template>
