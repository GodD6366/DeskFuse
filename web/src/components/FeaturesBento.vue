<script setup lang="ts">
import { h, type Component } from 'vue'
import {
  ClipboardDemo,
  SearchDemo,
  TransferDemo,
  DrawerDemo,
  DiscoveryDemo,
  DisplaysDemo,
  QuicDemo,
} from './demos'

interface Feature {
  title: string
  desc: string
  icon: Component
  wide?: boolean
  demo: Component
}

const icon = (paths: string): Component =>
  h('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': 1.8, 'stroke-linecap': 'round', 'stroke-linejoin': 'round', class: 'card-icon', innerHTML: paths })

const features: Feature[] = [
  {
    title: '剪贴板同步',
    desc: '文本、图片与原生格式在可信设备间实时同步。在 Mac 上复制，到另一台设备直接粘贴 —— 不依赖 iCloud，局域网内即时抵达。',
    wide: true,
    icon: icon('<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M9 12h6M9 16h4"/>'),
    demo: ClipboardDemo,
  },
  {
    title: '剪贴板历史',
    desc: '跨设备同步的剪贴板历史，支持图片内容搜索。昨天复制过的截图，今天搜得到。',
    icon: icon('<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>'),
    demo: SearchDemo,
  },
  {
    title: '文件传输',
    desc: '按需传输，小文件可提前送达；完整保留名称与文件夹结构，历史缓存有容量上限与清理回执。',
    icon: icon('<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/><path d="M12 11v6M9 14l3-3 3 3"/>'),
    demo: TransferDemo,
  },
  {
    title: '暂存抽屉',
    desc: '把文件拖进 240 × 240 的悬浮抽屉暂存，整批拖出到 Finder，或直接发送给另一台设备。',
    icon: icon('<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>'),
    demo: DrawerDemo,
  },
  {
    title: '设备发现与配对',
    desc: '自动发现局域网设备，也可手动输入 IPv4 定位；配对仍需对端确认，IP 不作为信任依据。',
    icon: icon('<circle cx="12" cy="12" r="2"/><path d="M4.93 19.07a10 10 0 0 1 0-14.14M19.07 4.93a10 10 0 0 1 0 14.14M7.76 16.24a6 6 0 0 1 0-8.49M16.24 7.76a6 6 0 0 1 0 8.49"/>'),
    demo: DiscoveryDemo,
  },
  {
    title: '屏幕与键鼠共享',
    desc: '一套键鼠跨屏控制多台设备：真实显示器布局、独立开关、权限状态可见，紧急收回一键生效。不再受同一 Apple 账号限制。',
    wide: true,
    icon: icon('<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>'),
    demo: DisplaysDemo,
  },
  {
    title: 'QUIC 加密与隐私',
    desc: '传输基于 QUIC 加密，设备身份持久可信。日志只记录元数据、状态与哈希 —— 永不记录剪贴板正文。',
    icon: icon('<rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>'),
    demo: QuicDemo,
  },
]
</script>

<template>
  <section class="section section-alt" id="features">
    <div class="section-inner">
      <div class="section-head" v-reveal>
        <p class="eyebrow">能做什么</p>
        <h2>一台 Mac 放不下的时候，<br />DeskFuse 把它们连成一体。</h2>
        <p class="lead">剪贴板、文件、屏幕与键鼠，在可信设备之间自由流转。</p>
      </div>
      <div class="bento">
        <div
          v-for="f in features"
          :key="f.title"
          :class="f.wide ? 'card-slot-wide' : 'card-slot'"
          v-reveal
        >
          <div class="card">
            <component :is="f.icon" />
            <h3>{{ f.title }}</h3>
            <p>{{ f.desc }}</p>
            <div class="card-visual"><component :is="f.demo" /></div>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>
