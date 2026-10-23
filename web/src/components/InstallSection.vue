<script setup lang="ts">
import { computed } from 'vue'
import { useLatestRelease, assetUrl, FALLBACK_RELEASE_TAG } from '../composables/useLatestRelease'
import { useCopy } from '../composables/useCopy'
import { HOMEBREW_ONE_LINE, HOMEBREW_SCRIPT, HOMEBREW_UPGRADE } from '../config/install'
import {
  ARM64_DISK_IMAGE,
  INTEL_DISK_IMAGE,
  REPOSITORY_URL,
  fallbackAssetUrl,
  macAppDownloadUrl,
} from '../config/distribution'

const { release } = useLatestRelease()
const { copied, copy } = useCopy()

const arm64Url = computed(() => macAppDownloadUrl(release.value?.assets, ARM64_DISK_IMAGE))
const intelUrl = computed(() => macAppDownloadUrl(release.value?.assets, INTEL_DISK_IMAGE))
const checksumsUrl = computed(() => assetUrl(release.value, 'SHA256SUMS', fallbackAssetUrl('SHA256SUMS')))
const versionLabel = computed(() => release.value?.tag ?? FALLBACK_RELEASE_TAG)
const usesAppPackage = computed(() => arm64Url.value.includes('-update.zip'))
const packageLabel = computed(() => (usesAppPackage.value ? '应用包' : '磁盘映像'))

const platforms = [
  ['macOS 14+ · Apple Silicon', '主平台', 'primary'],
  ['macOS 26+', 'Liquid Glass', 'ok'],
  ['Intel Mac', 'x86_64', 'ok'],
  ['Windows x64', 'Beta，独立验收', 'beta'],
  ['Linux', '不在首发范围', 'muted'],
]

const steps = [
  {
    index: '01',
    title: '安装',
    text: '打开下载的包，按安装窗口把 DeskFuse 放进「应用程序」。',
  },
  {
    index: '02',
    title: '授权',
    text: '允许访问局域网；使用键鼠共享时，再授予辅助功能权限。',
  },
  {
    index: '03',
    title: '配对',
    text: '在设备页选中另一台 Mac，在对端确认。之后打开开关即可使用。',
  },
]

const issuesUrl = `${REPOSITORY_URL}/issues`
const releasesUrl = `${REPOSITORY_URL}/releases`
</script>

<template>
  <section id="install" class="section install" aria-labelledby="install-title">
    <div class="container">
      <header class="section-head">
        <div class="section-head__main">
          <p class="section-index">05 / 安装</p>
          <h2 id="install-title" v-reveal.title>接上。</h2>
        </div>
        <div class="section-head__aside" v-reveal="90">
          <p class="lead">
            macOS 14+，一分钟装好。安装包经 Developer ID 签名与公证；用 Homebrew 安装的话，跟着 brew 一起更新。
          </p>
        </div>
      </header>

      <div class="install__grid">
        <div class="install__download" v-reveal>
          <div class="install__panel-head">
            <span>直接下载</span>
            <span class="install__version">{{ versionLabel }} · {{ packageLabel }}</span>
          </div>

          <div class="install__download-body">
            <a class="install__primary" :href="arm64Url">
              <span class="install__primary-main">
                <span class="install__chip-label">Apple Silicon</span>
                <strong>下载 macOS 版</strong>
              </span>
              <span class="install__primary-meta">
                arm64
                <i aria-hidden="true">↓</i>
              </span>
            </a>

            <a class="install__intel" :href="intelUrl">
              <span>
                <b>Intel Mac</b>
                <small>x86_64 · 同一发行版</small>
              </span>
              <span class="install__intel-arrow" aria-hidden="true">↗</span>
            </a>

            <div class="install__file-links">
              <a :href="checksumsUrl" target="_blank" rel="noopener">SHA256SUMS</a>
              <a :href="releasesUrl" target="_blank" rel="noopener">发布时间线</a>
            </div>
          </div>

          <div class="install__signature">
            <span class="install__signature-mark" aria-hidden="true"></span>
            <p>
              Developer ID 签名与公证。如果你的版本还没有应用内自动安装，按安装窗口提示操作即可。
            </p>
          </div>
        </div>

        <div class="install__terminal" v-reveal="80">
          <div class="terminal__head">
            <span class="terminal__lights" aria-hidden="true"><i></i><i></i><i></i></span>
            <span class="terminal__title">zsh — deskfuse</span>
            <span class="terminal__recommend">推荐</span>
            <button
              class="copy-button"
              :class="{ copied }"
              type="button"
              :aria-label="copied ? '已复制 Homebrew 命令' : '复制 Homebrew 命令'"
              @click="copy(HOMEBREW_ONE_LINE)"
            >
              {{ copied ? '已复制 ✓' : '复制命令' }}
            </button>
          </div>
          <pre class="terminal__code"><code><span class="terminal__prompt">$</span> {{ HOMEBREW_SCRIPT }}</code></pre>
          <div class="terminal__foot">
            <div>
              <span class="terminal__label">升级</span>
              <code>{{ HOMEBREW_UPGRADE }}</code>
            </div>
            <span class="terminal__hint">macOS 14+ · Homebrew</span>
          </div>
          <span class="sr-only" aria-live="polite">{{ copied ? 'Homebrew 命令已复制' : '' }}</span>
        </div>
      </div>

      <ol class="install__steps" aria-label="首次安装步骤" v-reveal="60">
        <li v-for="step in steps" :key="step.index" class="install__step">
          <span class="install__step-index">{{ step.index }}</span>
          <h3>{{ step.title }}</h3>
          <p>{{ step.text }}</p>
        </li>
      </ol>

      <div class="install__bottom">
        <div class="platforms" v-reveal="80">
          <div v-for="[name, status, kind] in platforms" :key="name" class="platform" :class="`is-${kind}`">
            <span class="platform__name">{{ name }}</span>
            <span class="platform__status">{{ status }}</span>
          </div>
        </div>

        <div class="install__help" v-reveal="120">
          <span class="install__help-mark" aria-hidden="true">?</span>
          <div>
            <b>安装或配对卡住了？</b>
            <p>带上版本号与报错，在 GitHub 开一个问题；应用内「帮助 → 反馈问题」也能一起导出脱敏诊断。</p>
          </div>
          <a class="text-link" :href="issuesUrl" target="_blank" rel="noopener">反馈问题</a>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.install {
  background:
    radial-gradient(ellipse 50% 45% at 8% 12%, rgba(89, 243, 218, 0.06), transparent 70%),
    var(--bg);
}

.install__grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.02fr);
  gap: clamp(18px, 3vw, 32px);
  align-items: stretch;
}

.install__download,
.install__terminal {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border: 1px solid var(--line);
  background: var(--surface);
  clip-path: polygon(0 0, calc(100% - 18px) 0, 100% 18px, 100% 100%, 0 100%);
}

.install__panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 13px 18px;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  font-family: var(--mono);
  font-size: 9.5px;
  letter-spacing: 0.06em;
}

.install__version {
  color: var(--faint);
}

.install__download-body {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 10px;
  padding: 18px;
}

.install__primary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  min-height: 92px;
  padding: 18px 20px;
  background: var(--aqua);
  color: var(--bg-deep);
  clip-path: polygon(0 0, calc(100% - 14px) 0, 100% 14px, 100% 100%, 0 100%);
  transition:
    background-color 0.2s ease,
    transform 0.16s ease;
}

.install__primary:hover {
  background: var(--aqua-soft);
}

.install__primary:active {
  transform: scale(0.992);
}

.install__primary-main {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.install__chip-label {
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.05em;
  opacity: 0.65;
}

.install__primary strong {
  font-size: clamp(20px, 2.2vw, 28px);
  font-weight: 700;
  letter-spacing: -0.02em;
}

.install__primary-meta {
  display: flex;
  align-items: center;
  gap: 12px;
  font-family: var(--mono);
  font-size: 10px;
}

.install__primary-meta i {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border: 1px solid rgba(6, 16, 14, 0.35);
  border-radius: 50%;
  font-size: 16px;
  font-style: normal;
  transition: transform 0.25s var(--ease-out);
}

.install__primary:hover .install__primary-meta i {
  transform: translateY(2px);
}

.install__intel {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 62px;
  padding: 13px 18px;
  border: 1px solid var(--line);
  transition:
    border-color 0.2s ease,
    background-color 0.2s ease;
}

.install__intel:hover {
  border-color: var(--line-strong);
  background: rgba(207, 255, 243, 0.03);
}

.install__intel span:first-child {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.install__intel b {
  font-size: 14px;
  font-weight: 550;
}

.install__intel small {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
}

.install__intel-arrow {
  color: var(--faint);
  font-size: 17px;
}

.install__file-links {
  display: flex;
  gap: 22px;
  padding: 6px 2px 0;
}

.install__file-links a {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9.5px;
  letter-spacing: 0.03em;
  border-bottom: 1px solid var(--line);
  transition:
    color 0.2s ease,
    border-color 0.2s ease;
}

.install__file-links a:hover {
  color: var(--aqua);
  border-color: var(--aqua);
}

.install__signature {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin-top: auto;
  padding: 14px 18px;
  border-top: 1px solid var(--line);
}

.install__signature-mark {
  flex: none;
  width: 6px;
  height: 6px;
  margin-top: 6px;
  border: 1px solid var(--aqua);
  border-radius: 50%;
}

.install__signature p {
  color: var(--faint);
  font-size: 10px;
  line-height: 1.65;
}

/* terminal */

.install__terminal {
  background: var(--bg-deep);
}

.terminal__head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 11px 14px;
  border-bottom: 1px solid var(--line);
}

.terminal__lights {
  display: flex;
  gap: 5px;
}

.terminal__lights i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: rgba(255, 106, 60, 0.8);
}

.terminal__lights i:nth-child(2) {
  background: rgba(216, 183, 94, 0.8);
}

.terminal__lights i:nth-child(3) {
  background: rgba(89, 243, 218, 0.8);
}

.terminal__title {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.05em;
}

.terminal__recommend {
  padding: 2px 7px;
  border: 1px solid rgba(89, 243, 218, 0.28);
  color: var(--aqua);
  font-family: var(--mono);
  font-size: 8px;
  letter-spacing: 0.04em;
}

.terminal__head .copy-button {
  margin-left: auto;
  font-size: 9px;
}

.terminal__code {
  flex: 1;
  min-height: 210px;
  padding: 24px 20px;
  overflow-x: auto;
  color: var(--ink);
  font-size: 12px;
  line-height: 1.9;
  tab-size: 2;
  white-space: pre;
}

.terminal__prompt {
  color: var(--orange);
}

.terminal__code code {
  font-size: inherit;
}

.terminal__foot {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 20px 18px;
  border-top: 1px solid var(--line);
}

.terminal__foot > div {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.terminal__label {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8px;
  letter-spacing: 0.06em;
}

.terminal__foot code {
  color: var(--muted);
  font-size: 10.5px;
  overflow-wrap: anywhere;
}

.terminal__hint {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 8.5px;
  white-space: nowrap;
}

/* steps */

.install__steps {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  margin-top: clamp(42px, 5vw, 64px);
  border-top: 1px solid var(--line);
  border-left: 1px solid var(--line);
  list-style: none;
}

.install__step {
  display: grid;
  grid-template-columns: 30px minmax(0, 1fr);
  gap: 6px 12px;
  align-content: start;
  padding: 20px;
  border-right: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
}

.install__step-index {
  grid-row: span 2;
  padding-top: 3px;
  color: var(--orange);
  font-family: var(--mono);
  font-size: 9px;
}

.install__step h3 {
  color: var(--ink);
  font-size: 14px;
  font-weight: 600;
}

.install__step p {
  color: var(--muted);
  font-size: 11.5px;
  line-height: 1.7;
}

.install__bottom {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: clamp(24px, 4vw, 48px);
  margin-top: 24px;
}

.platforms {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--line);
}

.platform {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  padding: 11px 2px;
  border-bottom: 1px solid var(--line);
}

.platform__name {
  color: var(--ink);
  font-size: 12px;
}

.platform__status {
  color: var(--faint);
  font-family: var(--mono);
  font-size: 9.5px;
  white-space: nowrap;
}

.platform.is-primary .platform__status {
  color: var(--aqua);
}

.platform.is-ok .platform__status {
  color: var(--muted);
}

.platform.is-beta .platform__status {
  color: var(--orange-soft);
}

.install__help {
  display: flex;
  align-items: flex-start;
  gap: 14px;
  padding: 18px;
  border: 1px solid var(--line);
  background: rgba(207, 255, 243, 0.018);
}

.install__help-mark {
  display: grid;
  place-items: center;
  flex: none;
  width: 24px;
  height: 24px;
  border: 1px solid var(--line-strong);
  border-radius: 50%;
  color: var(--aqua);
  font-family: var(--mono);
  font-size: 11px;
}

.install__help div {
  min-width: 0;
}

.install__help b {
  color: var(--ink);
  font-size: 13px;
  font-weight: 600;
}

.install__help p {
  margin-top: 4px;
  color: var(--faint);
  font-size: 11px;
  line-height: 1.7;
}

.install__help .text-link {
  flex: none;
  margin-left: auto;
  font-size: 11px;
  white-space: nowrap;
}

@media (max-width: 1000px) {
  .install__grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .install__bottom {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 760px) {
  .install__steps {
    grid-template-columns: minmax(0, 1fr);
  }

  .install__step {
    padding: 16px;
  }

  .terminal__code {
    min-height: 170px;
    font-size: 11px;
  }
}

@media (max-width: 560px) {
  .install__download-body {
    padding: 12px;
  }

  .install__primary {
    min-height: 82px;
    padding: 14px 16px;
  }

  .install__primary-meta {
    font-size: 9px;
  }

  .install__primary-meta i {
    width: 26px;
    height: 26px;
  }

  .terminal__foot {
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }

  .install__help {
    flex-wrap: wrap;
  }

  .install__help .text-link {
    margin-left: 38px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .install__primary-meta i {
    transition: none;
  }
}
</style>
