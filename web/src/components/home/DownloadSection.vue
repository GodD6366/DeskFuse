<script setup lang="ts">
import { useCopy } from '../../composables/useCopy'
import { useDownloads } from '../../composables/useDownloads'
import { HOMEBREW_ONE_LINE } from '../../config/install'
import { REPOSITORY_URL } from '../../config/distribution'

const { tag, prerelease, releaseUrl, arm64, intel, arm64Size, intelSize, date } = useDownloads()
const { copied, copy } = useCopy()

const releasesUrl = `${REPOSITORY_URL}/releases`
const kindLabel = (kind: 'zip' | 'dmg') => (kind === 'dmg' ? '.dmg' : '.zip')
</script>

<template>
  <section id="download" class="dl section">
    <div class="page">
      <header class="dl__head">
        <h2>把 DeskFuse<br />装进两台 Mac</h2>
        <p>从 GitHub Releases 取得安装包。两台电脑都要装；装好后配对一次，之后就不用再管。</p>
      </header>

      <div class="dl__grid">
        <div class="dl__main">
          <div class="dl__release">
            <span class="badge">最新版</span>
            <a class="dl__tag" :href="releaseUrl" target="_blank" rel="noopener">
              {{ tag }}{{ prerelease ? ' 预发行' : '' }}
            </a>
            <span v-if="date" class="dl__date">{{ date }}</span>
            <span class="dl__req">macOS 14 或更新</span>
            <a class="dl__release-link" :href="releaseUrl" target="_blank" rel="noopener">
              打开 Release <span aria-hidden="true">↗</span>
            </a>
          </div>

          <div class="dl__platform">
            <div class="dl__platform-head">
              <h3>macOS</h3>
              <span>按芯片选择安装包</span>
            </div>

            <a class="dl__asset dl__asset--primary" :href="arm64.url">
              <span class="dl__asset-name">Apple 芯片</span>
              <span class="dl__asset-file">{{ kindLabel(arm64.kind) }}<em v-if="arm64Size">{{ arm64Size }}</em></span>
              <span class="dl__asset-arrow" aria-hidden="true">↓</span>
            </a>

            <a class="dl__asset" :href="intel.url">
              <span class="dl__asset-name">Intel 芯片</span>
              <span class="dl__asset-file">{{ kindLabel(intel.kind) }}<em v-if="intelSize">{{ intelSize }}</em></span>
              <span class="dl__asset-arrow" aria-hidden="true">↓</span>
            </a>

            <p class="dl__note">
              打开压缩包里的 DeskFuse.app，按安装窗口的提示装进「应用程序」。键鼠共享需要辅助功能授权。
            </p>
          </div>

          <ol class="dl__steps">
            <li>
              <span>01</span>
              <h3>下载并打开</h3>
              <p>按芯片选安装包，打开后按安装窗口的提示操作。</p>
            </li>
            <li>
              <span>02</span>
              <h3>装进「应用程序」</h3>
              <p>逐项允许需要的系统权限。键鼠共享还需要辅助功能授权。</p>
            </li>
            <li>
              <span>03</span>
              <h3>两台都装好</h3>
              <p>在附近设备里点「添加」，并在两台电脑上分别确认。</p>
            </li>
          </ol>
        </div>

        <aside class="dl__side">
          <section class="dl__side-block">
            <h3>Windows</h3>
            <p class="dl__side-lead">x64 客户端仍在测试，暂不提供公开下载。准备好后会出现在 Release。</p>
            <span class="dl__status">测试中</span>
          </section>

          <section class="dl__side-block">
            <h3>其他安装方式</h3>
            <button class="dl__text-action" type="button" @click="copy(HOMEBREW_ONE_LINE)">
              <span>{{ copied ? '已复制 Homebrew 命令' : '复制 Homebrew 命令' }}</span>
              <span aria-hidden="true">{{ copied ? '✓' : '⧉' }}</span>
            </button>
            <a class="dl__text-action" :href="releasesUrl" target="_blank" rel="noopener">
              <span>全部版本与更新记录</span>
              <span aria-hidden="true">↗</span>
            </a>
            <p class="dl__side-note">
              Homebrew 仅支持 Apple 芯片。以前从旧发行仓库添加过 tap 的，先执行
              <code>brew untap godd6366/deskfuse</code>。
            </p>
          </section>

          <p class="dl__fineprint">
            签名状态以 Release 说明为准。macOS 拦截时，在「系统设置 › 隐私与安全性」里允许本次安装。
          </p>
        </aside>
      </div>
    </div>
  </section>
</template>

<style scoped>
.dl {
  border-top: 1px solid var(--rule-strong);
}

.dl__head {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 340px);
  gap: 24px;
  align-items: start;
  margin-bottom: 44px;
  padding-top: 14px;
}

.dl__head h2 {
  grid-column: 1;
  font-size: var(--t-h2);
}

.dl__head p {
  grid-column: 2;
  color: var(--body);
  font-size: 1rem;
  line-height: 1.8;
}

.dl__grid {
  display: grid;
  grid-template-columns: minmax(0, 1.35fr) minmax(260px, 0.65fr);
  gap: clamp(32px, 5vw, 72px);
  align-items: start;
}

.dl__release {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px 16px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--rule-strong);
}

.dl__tag {
  color: var(--ink);
  font-family: var(--font-serif-en);
  font-size: 1.25rem;
  font-weight: 500;
  text-decoration: none;
}

.dl__tag:hover {
  text-decoration: underline;
  text-underline-offset: 3px;
}

.dl__date,
.dl__req {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  letter-spacing: 0.02em;
}

.dl__date::before {
  margin-right: 12px;
  color: var(--rule);
  content: '/';
}

.dl__req {
  padding-left: 16px;
  border-left: 1px solid var(--rule);
}

.dl__release-link {
  margin-left: auto;
  color: var(--vermilion);
  font-size: 0.8125rem;
  text-decoration: none;
}

.dl__release-link:hover {
  text-decoration: underline;
  text-underline-offset: 3px;
}

.dl__platform {
  margin-top: 26px;
}

.dl__platform-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 10px;
}

.dl__platform-head h3 {
  font-size: 1.35rem;
}

.dl__platform-head span {
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.04em;
}

.dl__asset {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 14px;
  min-height: 64px;
  padding: 0 16px;
  border-top: 1px solid var(--rule);
  color: var(--ink);
  text-decoration: none;
  transition:
    background-color 0.18s var(--ease),
    color 0.18s var(--ease);
}

.dl__asset:last-of-type {
  border-bottom: 1px solid var(--rule);
}

.dl__asset:hover {
  background: var(--paper-2);
}

.dl__asset--primary {
  border-top-color: var(--vermilion);
  border-bottom: 1px solid var(--vermilion);
  background: var(--vermilion);
  color: var(--paper);
}

.dl__asset--primary:hover {
  background: var(--vermilion-dark);
  color: var(--paper);
}

.dl__asset-name {
  font-size: 1rem;
  font-weight: 600;
}

.dl__asset-file {
  display: inline-flex;
  align-items: baseline;
  gap: 8px;
  font-family: var(--font-mono);
  font-size: 0.6875rem;
  letter-spacing: 0.03em;
  opacity: 0.8;
}

.dl__asset-file em {
  font-style: normal;
  font-size: 0.625rem;
  opacity: 0.75;
}

.dl__asset-arrow {
  font-family: var(--font-mono);
  font-size: 1rem;
}

.dl__note {
  max-width: 44em;
  margin-top: 12px;
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.7;
}

.dl__steps {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0;
  margin: 44px 0 0;
  padding: 0;
  border-top: 1px solid var(--rule-strong);
  list-style: none;
}

.dl__steps li {
  padding: 18px 20px 0 0;
  border-right: 1px solid var(--rule);
}

.dl__steps li + li {
  padding-left: 20px;
}

.dl__steps li:last-child {
  border-right: 0;
}

.dl__steps span {
  display: block;
  color: var(--vermilion);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.06em;
}

.dl__steps h3 {
  margin-top: 8px;
  font-family: var(--font-sans);
  font-size: 0.9375rem;
  font-weight: 600;
  line-height: 1.4;
}

.dl__steps p {
  margin-top: 7px;
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.65;
}

.dl__side {
  display: grid;
  gap: 36px;
  padding-left: clamp(20px, 3vw, 36px);
  border-left: 1px solid var(--rule);
}

.dl__side-block h3 {
  padding-bottom: 8px;
  border-bottom: 1px solid var(--rule-strong);
  font-family: var(--font-sans);
  font-size: 0.8125rem;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.dl__side-lead {
  margin-top: 12px;
  color: var(--body);
  font-size: 0.9375rem;
  line-height: 1.8;
}

.dl__status {
  display: inline-block;
  margin-top: 12px;
  color: var(--muted);
  font-family: var(--font-mono);
  font-size: 0.625rem;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.dl__text-action {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  width: 100%;
  padding: 12px 0;
  border: 0;
  border-bottom: 1px solid var(--rule);
  background: transparent;
  color: var(--ink);
  font: inherit;
  font-size: 0.875rem;
  text-align: left;
  text-decoration: none;
  cursor: pointer;
}

.dl__text-action:first-of-type {
  margin-top: 10px;
  border-top: 1px solid var(--rule);
}

.dl__text-action:hover {
  color: var(--vermilion);
}

.dl__text-action span:last-child {
  font-family: var(--font-mono);
  font-size: 0.75rem;
}

.dl__side-note {
  margin-top: 12px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.7;
}

.dl__side-note code {
  padding: 1px 3px;
  background: var(--paper-2);
  font-size: 0.6875rem;
}

.dl__fineprint {
  padding-top: 12px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.7;
}

@media (max-width: 1080px) {
  .dl__head {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .dl__head h2,
  .dl__head p {
    grid-column: 1;
  }

  .dl__head p {
    max-width: 38em;
  }
}

@media (max-width: 960px) {
  .dl__grid {
    grid-template-columns: 1fr;
  }

  .dl__side {
    padding-left: 0;
    border-left: 0;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 32px;
  }

  .dl__fineprint {
    grid-column: 1 / -1;
  }
}

@media (max-width: 640px) {
  .dl__head {
    margin-bottom: 32px;
  }

  .dl__release {
    gap: 8px 12px;
  }

  .dl__req {
    padding-left: 0;
    border-left: 0;
  }

  .dl__release-link {
    width: 100%;
    margin-left: 0;
    margin-top: 4px;
  }

  .dl__steps {
    grid-template-columns: 1fr;
  }

  .dl__steps li,
  .dl__steps li + li {
    padding: 16px 0;
    border-right: 0;
    border-bottom: 1px solid var(--rule);
  }

  .dl__steps li:last-child {
    border-bottom: 0;
  }

  .dl__side {
    grid-template-columns: 1fr;
    gap: 28px;
  }
}
</style>
