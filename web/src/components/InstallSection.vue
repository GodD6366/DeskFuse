<script setup lang="ts">
import { useDownloads } from '../composables/useDownloads'
import { useCopy } from '../composables/useCopy'
import { HOMEBREW_SCRIPT, HOMEBREW_UPGRADE } from '../config/install'
import { REPOSITORY_URL } from '../config/distribution'

const { tag, prerelease, releaseUrl, arm64, intel, arm64Size } = useDownloads()
const { copied, copy } = useCopy()
const releasesUrl = `${REPOSITORY_URL}/releases`
</script>

<template>
  <section id="install" class="section install">
    <div class="page">
      <div class="section-head">
        <h2>安装</h2>
        <p>
          两台电脑都要装。当前版本
          <a class="link" :href="releaseUrl" target="_blank" rel="noopener">{{ tag }}</a>{{ prerelease ? '（预发行）' : '' }}，需要 macOS 14 或更新。
        </p>
      </div>

      <div class="install__grid">
        <div class="install__main">
          <div class="install__downloads">
            <a class="btn btn--primary" :href="arm64.url">
              下载 Apple 芯片版<span v-if="arm64Size" class="btn__size">{{ arm64Size }}</span>
            </a>
            <a class="btn btn--quiet" :href="intel.url">下载 Intel 芯片版</a>
          </div>
          <p class="install__chip">
            不确定是哪种芯片：点屏幕左上角的苹果菜单，选「关于本机」，「芯片」一栏写着 Apple M 开头的就是 Apple 芯片。
          </p>

          <ol class="steps">
            <li>
              <h3>打开安装窗口</h3>
              <p v-if="arm64.kind === 'zip'">打开下载的压缩包，双击里面的 DeskFuse.app。</p>
              <p v-else>打开下载的磁盘映像，双击里面的 DeskFuse.app。</p>
            </li>
            <li>
              <h3>装进「应用程序」</h3>
              <p>点「点击安装」，装好后点「打开 DeskFuse」，再按引导逐项打开需要的系统权限。</p>
            </li>
            <li>
              <h3>添加另一台电脑</h3>
              <p>
                另一台也装好后，在「设备」页的附近列表里点「添加」，并在两边确认。两台电脑要连在同一个网络里；列表里没有时，点「+」输入对方的 IP 地址。
              </p>
            </li>
          </ol>
        </div>

        <aside class="install__side">
          <div class="brew">
            <div class="brew__head">
              <h3>用 Homebrew 安装</h3>
              <button class="btn btn--quiet btn--small" type="button" @click="copy(HOMEBREW_SCRIPT)">
                {{ copied ? '已复制' : '复制命令' }}
              </button>
            </div>
            <pre><code>{{ HOMEBREW_SCRIPT }}</code></pre>
            <p>仅支持 Apple 芯片。以前从旧发行仓库添加过 tap 的，先执行 <code>brew untap godd6366/deskfuse</code>。</p>
          </div>

          <div class="install__notes">
            <h3>以后的更新</h3>
            <p>在「设置 › 关于与更新」里检查新版本。用 Homebrew 安装的，执行：</p>
            <pre><code>{{ HOMEBREW_UPGRADE }}</code></pre>
            <p>
              Windows 版还在测试，暂不提供下载。旧版本见
              <a class="link" :href="releasesUrl" target="_blank" rel="noopener">全部版本</a>。
            </p>
          </div>
        </aside>
      </div>
    </div>
  </section>
</template>

<style scoped>
.install {
  background: var(--c-paper);
}

.install__grid {
  display: grid;
  grid-template-columns: minmax(0, 7fr) minmax(0, 5fr);
  gap: clamp(40px, 6vw, 88px);
}

.install__downloads {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

.btn__size {
  font-weight: 400;
  opacity: 0.8;
}

.install__chip {
  max-width: 34em;
  margin-top: 14px;
  color: var(--c-muted);
  font-size: var(--t-small);
}

.steps {
  margin: 48px 0 0;
  padding: 0;
  list-style: none;
  counter-reset: step;
}

.steps li {
  position: relative;
  padding: 0 0 32px 52px;
  counter-increment: step;
}

.steps li::before {
  content: counter(step);
  position: absolute;
  left: 0;
  top: -2px;
  display: grid;
  place-items: center;
  width: 32px;
  height: 24px;
  border-radius: 5px;
  background: var(--c-graphite);
  color: #fff;
  font-size: var(--t-small);
  font-weight: 600;
}

.steps li:not(:last-child)::after {
  content: '';
  position: absolute;
  left: 15.5px;
  top: 30px;
  bottom: 6px;
  width: 1px;
  background: var(--c-line);
}

.steps h3 {
  font-size: 1.125rem;
}

.steps p {
  max-width: 32em;
  margin-top: 6px;
  color: var(--c-muted);
}

.install__side {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  align-content: start;
  gap: 40px;
}

.brew {
  padding: 24px;
  border-radius: var(--r-card);
  background: var(--c-desk);
}

.brew__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.install__side h3 {
  font-size: 1.125rem;
}

pre {
  margin: 16px 0 0;
  padding: 16px 18px;
  overflow-x: auto;
  border-radius: var(--r-control);
  background: var(--c-graphite);
  color: #ecebe7;
  font-size: 0.8125rem;
  line-height: 1.7;
}

pre code {
  font-size: inherit;
}

.install__side p {
  margin-top: 14px;
  color: var(--c-muted);
  font-size: 0.9375rem;
}

.install__side p code {
  color: var(--c-ink);
  word-break: break-all;
}

@media (max-width: 960px) {
  .install__grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
