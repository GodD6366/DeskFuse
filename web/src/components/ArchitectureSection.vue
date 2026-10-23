<script setup lang="ts">
import { REPOSITORY_URL } from '../config/distribution'

const architectureUrl = `${REPOSITORY_URL}/blob/main/docs/ARCHITECTURE.md`

const layers = [
  {
    index: '01',
    name: 'SwiftUI / AppKit',
    detail: '原生窗口 · 拖拽 · 菜单栏 · 安装窗口',
    tag: 'NATIVE UI',
  },
  {
    index: '02',
    name: 'UniFFI',
    detail: '类型化桥接，界面不持有运行时句柄',
    tag: 'BINDING',
  },
  {
    index: '03',
    name: 'Rust 2024 引擎',
    detail: '剪贴板 · 文件 · 输入 · 历史 · 更新',
    tag: 'ENGINE',
  },
  {
    index: '04',
    name: 'QUIC / 局域网',
    detail: '身份认证的连接，设备之间直接加密',
    tag: 'TRANSPORT',
  },
]

const specs = [
  ['系统', 'macOS 14+'],
  ['芯片', 'Apple Silicon / Intel'],
  ['引擎', 'Rust 2024'],
  ['传输', 'QUIC · 局域网直连'],
  ['发现', 'Bonjour'],
  ['公网通道', '默认关闭'],
  ['日志', '元数据与哈希'],
  ['签名', 'Developer ID · 公证'],
]

const principles = [
  {
    title: '先确认，再信任',
    text: '配对必须在对端确认。IP、发现信息与连接请求本身，都不是授权依据。',
  },
  {
    title: '本地优先',
    text: '默认不经过云端中转，设备之间直接连接。跨公网通道默认关闭，不替代原有信任链。',
  },
  {
    title: '日志克制',
    text: '只记录元数据、状态与哈希。默认不把剪贴板正文或文件内容写进诊断日志。',
  },
]
</script>

<template>
  <section id="architecture" class="section section--paper architecture" aria-labelledby="architecture-title">
    <div class="container">
      <div class="architecture__top">
        <div class="architecture__intro">
          <p class="section-index">03 / 架构</p>
          <h2 id="architecture-title" v-reveal.title>不是网页套壳。</h2>
          <p class="lead" v-reveal="80">
            DeskFuse 的界面是 SwiftUI 与 AppKit，窗口、拖拽、菜单栏都是系统原生。剪贴板、文件、输入与存储，由同一个 Rust 2024 引擎驱动。
          </p>
          <a
            class="text-link architecture__link"
            :href="architectureUrl"
            target="_blank"
            rel="noopener"
            v-reveal="140"
          >
            阅读架构文档
          </a>
        </div>

        <div class="architecture__diagram" v-reveal="120">
          <div class="architecture__diagram-head">
            <span>原生链路</span>
            <span>FIG. 01</span>
          </div>
          <ol class="stack">
            <li v-for="layer in layers" :key="layer.index" class="stack__layer">
              <span class="stack__index">{{ layer.index }}</span>
              <span class="stack__body">
                <b>{{ layer.name }}</b>
                <small>{{ layer.detail }}</small>
              </span>
              <span class="stack__tag">{{ layer.tag }}</span>
            </li>
          </ol>
          <div class="architecture__diagram-foot">
            <span>共享 Rust 核心</span>
            <span class="architecture__diagram-line" aria-hidden="true"></span>
            <span>macOS / Windows</span>
          </div>
        </div>
      </div>

      <dl class="specs" v-reveal="80">
        <div v-for="[label, value] in specs" :key="label" class="specs__item">
          <dt>{{ label }}</dt>
          <dd>{{ value }}</dd>
        </div>
      </dl>

      <div class="principles">
        <div v-for="(principle, index) in principles" :key="principle.title" class="principle" v-reveal="index * 70">
          <span class="principle__mark" aria-hidden="true"></span>
          <h3>{{ principle.title }}</h3>
          <p>{{ principle.text }}</p>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.architecture {
  padding-block: clamp(96px, 12vw, 176px);
}

.architecture__top {
  display: grid;
  grid-template-columns: minmax(0, 0.82fr) minmax(0, 1.18fr);
  gap: clamp(40px, 7vw, 120px);
  align-items: start;
}

.architecture__intro {
  padding-top: 6px;
}

.architecture__intro h2 {
  color: var(--paper-ink);
  font-family: var(--display);
  font-weight: 400;
  font-size: clamp(44px, 5.7vw, 82px);
  line-height: 1.06;
  letter-spacing: -0.015em;
}

.architecture__intro .lead {
  margin-top: 28px;
  max-width: 38ch;
  color: var(--paper-muted);
}

.architecture__link {
  margin-top: 30px;
}

.architecture__diagram {
  position: relative;
  padding: 18px;
  border: 1px solid var(--paper-line);
  background: rgba(237, 233, 220, 0.55);
}

.architecture__diagram::before,
.architecture__diagram::after {
  content: "";
  position: absolute;
  width: 5px;
  height: 5px;
  border: 1px solid rgba(16, 25, 21, 0.45);
}

.architecture__diagram::before {
  top: -3px;
  left: -3px;
}

.architecture__diagram::after {
  right: -3px;
  bottom: -3px;
}

.architecture__diagram-head,
.architecture__diagram-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  color: var(--paper-muted);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.08em;
}

.architecture__diagram-head {
  padding-bottom: 12px;
  border-bottom: 1px solid var(--paper-line);
}

.architecture__diagram-foot {
  padding-top: 14px;
  border-top: 1px solid var(--paper-line);
}

.architecture__diagram-line {
  flex: 1;
  height: 1px;
  margin: 0 16px;
  background: var(--paper-line);
}

.stack {
  display: flex;
  flex-direction: column;
  list-style: none;
}

.stack__layer {
  position: relative;
  display: grid;
  grid-template-columns: 26px minmax(0, 1fr) auto;
  gap: 16px;
  align-items: center;
  padding: 19px 4px;
  border-bottom: 1px solid var(--paper-line);
}

.stack__layer:last-child {
  border-bottom: 0;
}

.stack__layer::after {
  content: "";
  position: absolute;
  bottom: -4px;
  left: 30px;
  width: 1px;
  height: 8px;
  background: var(--paper-ink);
  opacity: 0.35;
}

.stack__layer:last-child::after {
  display: none;
}

.stack__index {
  color: rgba(16, 25, 21, 0.4);
  font-family: var(--mono);
  font-size: 9px;
}

.stack__body {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.stack__body b {
  color: var(--paper-ink);
  font-size: 17px;
  font-weight: 650;
  letter-spacing: -0.01em;
}

.stack__body small {
  color: var(--paper-muted);
  font-size: 12px;
}

.stack__tag {
  color: rgba(15, 107, 93, 0.85);
  font-family: var(--mono);
  font-size: 8px;
  letter-spacing: 0.06em;
  white-space: nowrap;
}

.specs {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin-top: clamp(64px, 8vw, 104px);
  border-top: 1px solid var(--paper-line);
  border-left: 1px solid var(--paper-line);
}

.specs__item {
  display: flex;
  flex-direction: column;
  gap: 7px;
  min-width: 0;
  padding: 18px 18px 20px;
  border-right: 1px solid var(--paper-line);
  border-bottom: 1px solid var(--paper-line);
}

.specs__item dt {
  color: var(--paper-muted);
  font-family: var(--mono);
  font-size: 9px;
  letter-spacing: 0.06em;
}

.specs__item dd {
  color: var(--paper-ink);
  font-size: 14px;
  font-weight: 550;
  letter-spacing: -0.005em;
}

.principles {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 0;
  margin-top: clamp(64px, 8vw, 104px);
  border-top: 1px solid var(--paper-line);
  border-bottom: 1px solid var(--paper-line);
}

.principle {
  position: relative;
  padding: 28px 32px 34px 0;
  border-right: 1px solid var(--paper-line);
}

.principle:not(:first-child) {
  padding-left: 32px;
}

.principle:last-child {
  border-right: 0;
}

.principle__mark {
  display: block;
  width: 20px;
  height: 1px;
  margin-bottom: 24px;
  background: var(--orange);
}

.principle h3 {
  color: var(--paper-ink);
  font-family: var(--display);
  font-size: clamp(24px, 2.4vw, 32px);
  font-weight: 400;
  line-height: 1.1;
}

.principle p {
  margin-top: 10px;
  max-width: 34ch;
  color: var(--paper-muted);
  font-size: 13px;
  line-height: 1.75;
}

@media (max-width: 1000px) {
  .architecture__top {
    grid-template-columns: minmax(0, 1fr);
    gap: 44px;
  }

  .architecture__intro .lead {
    max-width: 52ch;
  }

  .specs {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 760px) {
  .principles {
    grid-template-columns: minmax(0, 1fr);
  }

  .principle,
  .principle:not(:first-child) {
    padding: 24px 0;
    border-right: 0;
    border-bottom: 1px solid var(--paper-line);
  }

  .principle:last-child {
    border-bottom: 0;
  }

  .principle__mark {
    margin-bottom: 16px;
  }
}

@media (max-width: 520px) {
  .architecture__diagram {
    padding: 14px;
  }

  .stack__layer {
    grid-template-columns: 22px minmax(0, 1fr);
    gap: 10px;
    padding: 15px 2px;
  }

  .stack__tag {
    display: none;
  }

  .stack__body b {
    font-size: 15px;
  }

  .specs__item {
    padding: 14px 12px 16px;
  }
}
</style>
