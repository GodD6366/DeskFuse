<script setup lang="ts">
import GlyphIcon from '../GlyphIcon.vue'
import { useCopy } from '../../composables/useCopy'
import { useDownloads } from '../../composables/useDownloads'
import { HOMEBREW_ONE_LINE } from '../../config/install'
import { REPOSITORY_URL } from '../../config/distribution'

const { tag, prerelease, releaseUrl, arm64, intel, windows, arm64Size, intelSize, windowsSize, date } = useDownloads()
const { copied, copy } = useCopy()
const releasesUrl = `${REPOSITORY_URL}/releases`
const kindLabel = (kind: 'zip' | 'dmg') => (kind === 'dmg' ? 'DMG' : 'ZIP')
</script>

<template>
  <section id="download" class="dl section section--gray">
    <div class="page">
      <header class="section-heading">
        <h2>现在，把你的电脑连起来。</h2>
        <p>在要协同的电脑上安装 DeskFuse，配对一次。之后，键鼠、剪贴板和文件就能跟着你走。</p>
      </header>

      <div class="dl__panel">
        <div class="dl__release">
          <div>
            <span class="dl__label">DeskFuse 桌面版</span>
            <a class="dl__tag" :href="releaseUrl" target="_blank" rel="noopener">
              {{ tag }}{{ prerelease ? ' 预发行' : '' }} <span aria-hidden="true">↗</span>
            </a>
          </div>
          <p>选择适合你的安装包<span v-if="date"> · {{ date }}</span></p>
        </div>

        <div class="dl__choices">
          <a class="dl__choice" :href="arm64.url">
            <span class="dl__choice-icon"><GlyphIcon name="download" :size="23" /></span>
            <span class="dl__choice-copy">
              <strong>Apple 芯片</strong>
              <small>{{ kindLabel(arm64.kind) }}<template v-if="arm64Size"> · {{ arm64Size }}</template></small>
            </span>
            <span class="dl__choice-arrow" aria-hidden="true">↓</span>
          </a>
          <a class="dl__choice" :href="intel.url">
            <span class="dl__choice-icon"><GlyphIcon name="download" :size="23" /></span>
            <span class="dl__choice-copy">
              <strong>Intel 芯片</strong>
              <small>{{ kindLabel(intel.kind) }}<template v-if="intelSize"> · {{ intelSize }}</template></small>
            </span>
            <span class="dl__choice-arrow" aria-hidden="true">↓</span>
          </a>
          <a
            class="dl__choice"
            :href="windows.url"
            :target="windows.available ? undefined : '_blank'"
            :rel="windows.available ? undefined : 'noopener'"
          >
            <span class="dl__choice-icon"><GlyphIcon name="windows" :size="23" /></span>
            <span class="dl__choice-copy">
              <strong>Windows x64</strong>
              <small>Beta · {{ windows.available ? 'EXE' : '查看 Release' }}<template v-if="windowsSize"> · {{ windowsSize }}</template></small>
            </span>
            <span class="dl__choice-arrow" aria-hidden="true">{{ windows.available ? '↓' : '↗' }}</span>
          </a>
        </div>

        <p class="dl__install-note">打开安装包并按提示完成安装。键鼠共享需要按引导授予对应系统权限。</p>

        <div class="dl__other">
          <button type="button" @click="copy(HOMEBREW_ONE_LINE)">
            <GlyphIcon :name="copied ? 'check' : 'copy'" :size="17" />
            {{ copied ? '已复制 Homebrew 命令' : '复制 Homebrew 命令' }}
          </button>
          <a :href="releasesUrl" target="_blank" rel="noopener">全部版本与更新记录 <span aria-hidden="true">↗</span></a>
        </div>
        <p class="dl__homebrew">Homebrew 仅支持 Apple 芯片。旧 tap 用户先执行 <code>brew untap godd6366/deskfuse</code>。</p>
      </div>

      <ol class="dl__steps">
        <li>
          <span>1</span>
          <div><h3>下载并打开</h3><p>按电脑选择安装包，打开后按安装窗口的提示操作。</p></div>
        </li>
        <li>
          <span>2</span>
          <div><h3>允许所需权限</h3><p>按实际使用的功能逐项允许权限，不需要的权限可以不开启。</p></div>
        </li>
        <li>
          <span>3</span>
          <div><h3>在两端确认配对</h3><p>在附近设备里点「添加」，并在两台电脑上分别确认。</p></div>
        </li>
      </ol>

      <div class="dl__fineprint">
        <p><strong>Windows x64 Beta：</strong>安装器要求 Authenticode 签名；安装包随 Release 发布，具体能力以版本说明为准。</p>
        <p>签名状态与安装指引以 Release 说明为准。遇到系统拦截时，请先确认安装包来源与签名。</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.dl__panel {
  max-width: 900px;
  margin: 0 auto;
  padding: clamp(26px, 4vw, 44px);
  border-radius: 12px;
  background: #fff;
  box-shadow: 0 16px 44px rgb(0 0 0 / 7%);
}

.dl__release {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding-bottom: 24px;
  border-bottom: 1px solid #e8e8ed;
}

.dl__label {
  display: block;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.5;
}

.dl__tag {
  display: inline-block;
  margin-top: 4px;
  color: var(--ink);
  font-size: 1.5rem;
  font-weight: 600;
  line-height: 1.4;
  text-decoration: none;
}

.dl__tag span {
  color: var(--link);
  font-size: 0.875rem;
  font-weight: 400;
}

.dl__release > p {
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.6;
  text-align: right;
}

.dl__choices {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 14px;
  margin-top: 28px;
}

.dl__choice {
  display: grid;
  grid-template-columns: 42px minmax(0, 1fr) auto;
  align-items: center;
  gap: 14px;
  min-height: 92px;
  padding: 16px 20px;
  border: 1px solid var(--rule);
  border-radius: 12px;
  color: var(--ink);
  text-decoration: none;
  transition: border-color 0.18s var(--ease), background-color 0.18s var(--ease), transform 0.18s var(--ease);
}

.dl__choice:active {
  transform: scale(0.99);
}

.dl__choice-icon {
  display: grid;
  place-items: center;
  width: 42px;
  height: 42px;
  border-radius: 50%;
  background: var(--paper-2);
  color: var(--vermilion);
}

.dl__choice-copy {
  display: grid;
  gap: 3px;
}

.dl__choice-copy strong {
  font-size: 1rem;
  font-weight: 600;
  line-height: 1.5;
}

.dl__choice-copy small {
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.5;
}

.dl__choice-arrow {
  color: var(--link);
  font-size: 1.25rem;
}

.dl__install-note {
  margin-top: 18px;
  color: var(--muted);
  font-size: 0.8125rem;
  line-height: 1.7;
}

.dl__other {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 26px;
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid #e8e8ed;
}

.dl__other button,
.dl__other a {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-height: 40px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--link);
  font: inherit;
  font-size: 0.875rem;
  line-height: 1.5;
  text-decoration: none;
  cursor: pointer;
}

.dl__other button:hover,
.dl__other a:hover {
  text-decoration: underline;
  text-underline-offset: 4px;
}

.dl__homebrew {
  margin-top: 8px;
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.7;
}

.dl__homebrew code {
  overflow-wrap: anywhere;
}

.dl__steps {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 32px;
  max-width: 1000px;
  margin: 48px auto 0;
  padding: 0;
  list-style: none;
}

.dl__steps li {
  display: grid;
  grid-template-columns: 30px minmax(0, 1fr);
  gap: 13px;
}

.dl__steps li > span {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  background: #e8e8ed;
  color: var(--body);
  font-size: 0.75rem;
  font-weight: 500;
}

.dl__steps h3 {
  font-size: 1rem;
  line-height: 1.75;
}

.dl__steps p {
  margin-top: 7px;
  color: var(--body);
  font-size: 0.875rem;
  line-height: 1.7;
}

.dl__fineprint {
  display: grid;
  gap: 6px;
  max-width: 900px;
  margin: 44px auto 0;
  padding-top: 20px;
  border-top: 1px solid var(--rule);
  color: var(--muted);
  font-size: 0.75rem;
  line-height: 1.7;
}

.dl__fineprint strong {
  color: var(--body);
  font-weight: 500;
}

@media (hover: hover) and (pointer: fine) {
  .dl__choice:hover {
    border-color: var(--vermilion);
    background: #fafafd;
  }
}

@media (max-width: 800px) {
  .dl__choices {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 640px) {
  .dl__release {
    align-items: flex-start;
    flex-direction: column;
    gap: 10px;
  }

  .dl__release > p {
    text-align: left;
  }

  .dl__choices,
  .dl__steps {
    grid-template-columns: 1fr;
  }

  .dl__steps {
    gap: 24px;
    margin-top: 36px;
  }

  .dl__choice {
    min-height: 84px;
    padding: 14px 16px;
  }

  .dl__other {
    align-items: flex-start;
    flex-direction: column;
    gap: 2px;
  }
}
</style>
