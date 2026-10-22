import { defineComponent, h, type VNodeChild } from 'vue'

/** 静态资源路径（base 为 './'，拼接后相对当前页面解析） */
export const asset = (p: string) => `${import.meta.env.BASE_URL}${p}`

/* ---------- 功能卡片的迷你演示（纯静态结构，样式全部走全局 CSS） ---------- */

const I = (paths: string, size = 12, sw = 2.4) =>
  h('svg', { viewBox: '0 0 24 24', width: size, height: size, fill: 'none', stroke: 'currentColor', 'stroke-width': sw, 'stroke-linecap': 'round', 'stroke-linejoin': 'round', innerHTML: paths })

const clip = (iconCls: string, icon: VNodeChild, title: string, time: string, active = false): VNodeChild =>
  h('div', { class: `mini-clip${active ? ' active' : ''}` }, [
    h('span', { class: `mini-clip-icon ${iconCls}` }, [icon]),
    h('div', { class: 'mini-clip-body' }, [h('b', title), h('i', time)]),
  ])

const LINK = '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>'
const IMG = '<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.5-3.5a2 2 0 0 0-3 0L6 20"/>'
const TEXT = '<line x1="4" y1="6" x2="20" y2="6"/><line x1="4" y1="12" x2="16" y2="12"/><line x1="4" y1="18" x2="12" y2="18"/>'
const FILE = '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/>'
const FOLDER = '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>'
const BOX = '<path d="M21 8v13H3V8M1 3h22v5H1zM10 12h4"/>'
const LOCK = '<rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>'
const SEARCH = '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>'

const defineDemo = (render: () => VNodeChild) => defineComponent({ setup: () => render })

export const ClipboardDemo = defineDemo(() =>
  h('div', { class: 'mini-window mini-surface' }, [
    h('div', { class: 'mini-window-head' }, [
      h('span', { class: 'mini-dot' }),
      h('span', { class: 'mini-title' }, '剪贴板 · 所有设备'),
      h('span', { class: 'mini-sync' }, [h('i'), '已同步到 2 台设备']),
    ]),
    h('div', { class: 'mini-grid' }, [
      clip('icon-link', I(LINK), 'https://www.apple.com/cn', '13:24', true),
      clip('icon-img', I(IMG), 'IMG_2887.JPG · 2.4 MB', '13:21'),
      clip('icon-text', I(TEXT), '真正的效率，是让技术隐形。', '13:18'),
      clip('icon-file', I(FILE), '设计稿-首页.png · 3.4 MB', '13:12'),
    ]),
  ]),
)

export const SearchDemo = defineDemo(() =>
  h('div', { class: 'search-demo' }, [
    h('div', { class: 'mini-search mini-surface' }, [I(SEARCH, 13), h('span', '风景截图'), h('kbd', '⌘F')]),
    h('div', { class: 'mini-result mini-surface' }, [
      h('span', { class: 'mini-thumb' }),
      h('div', { class: 'mini-clip-body' }, [h('b', '风景图.png'), h('i', '命中图片内容 · 3.4 MB')]),
      h('span', { class: 'mini-hit' }, 'OCR'),
    ]),
    h('div', { class: 'mini-result dim mini-surface' }, [
      h('span', { class: 'mini-thumb t2' }),
      h('div', { class: 'mini-clip-body' }, [h('b', '桌面截图.png'), h('i', '昨天 18:02')]),
    ]),
  ]),
)

export const TransferDemo = defineDemo(() =>
  h('div', { class: 'transfer-demo' }, [
    h('div', { class: 'mini-file-row mini-surface' }, [
      h('span', { class: 'mini-clip-icon icon-file' }, [I(FOLDER)]),
      h('div', { class: 'mini-clip-body' }, [h('b', '项目备份.zip'), h('i', '128 MB · 接收中')]),
      h('span', { class: 'mini-speed' }, '42 MB/s'),
    ]),
    h('div', { class: 'mini-progress' }, [h('i')]),
    h('div', { class: 'mini-file-row done mini-surface' }, [
      h('span', { class: 'mini-clip-icon icon-img' }, [I(IMG)]),
      h('div', { class: 'mini-clip-body' }, [h('b', '设计稿-首页.png'), h('i', '3.4 MB · 已写入剪贴板')]),
      h('span', { class: 'mini-done-check' }, '已完成 ✓'),
    ]),
  ]),
)

export const DrawerDemo = defineDemo(() =>
  h('div', { class: 'drawer-demo' }, [
    h('div', { class: 'mini-drawer mini-surface' }, [
      h('div', { class: 'mini-drawer-head' }, [h('span', '暂存抽屉'), h('i', '3 项')]),
      h('div', { class: 'mini-drawer-files' }, [
        h('span', { class: 'dfile f1' }, [I(FILE, 13)]),
        h('span', { class: 'dfile f2' }, [I(IMG, 13)]),
        h('span', { class: 'dfile f3' }, [I(BOX, 13)]),
      ]),
    ]),
  ]),
)

export const DiscoveryDemo = defineDemo(() =>
  h('div', { class: 'discovery-demo' }, [
    h('span', { class: 'dev-radar' }),
    h('div', { class: 'mini-device mini-surface' }, [h('span', { class: 'dev-dot on' }), h('b', 'MacBook Pro 14'), h('i', '在线 · 205 ms')]),
    h('div', { class: 'mini-device mini-surface' }, [h('span', { class: 'dev-dot on' }), h('b', 'Mac mini 工作室'), h('i', '在线')]),
    h('div', { class: 'mini-device off mini-surface' }, [h('span', { class: 'dev-dot' }), h('b', 'MacBook Air'), h('i', '离线')]),
  ]),
)

export const DisplaysDemo = defineDemo(() =>
  h('div', { class: 'displays-demo' }, [
    h('div', { class: 'screen-canvas mini-surface' }, [
      h('div', { class: 'screen-box s2' }, [
        h('div', { class: 'screen-bar' }),
        h('div', { class: 'screen-info' }, [h('b', '02'), h('span', 'MacBook Pro 14'), h('i', '1512 × 982 pt')]),
      ]),
      h('div', { class: 'screen-box s1' }, [
        h('div', { class: 'screen-bar' }),
        h('div', { class: 'screen-info' }, [h('b', '01'), h('span', '本机 · DeskFuse'), h('i', '2048 × 1152 pt')]),
      ]),
      h('svg', { class: 'screen-cursor', viewBox: '0 0 24 24', width: 16, height: 16, innerHTML: '<path d="M4 2l16 11-7.2 1.4L9 22z" fill="#fff" stroke="#0a0e1a" stroke-width="1.6" stroke-linejoin="round"/>' }),
    ]),
    h('span', { class: 'screen-toggle mini-surface' }, [h('i'), '共享键鼠 · 已开启']),
  ]),
)

export const QuicDemo = defineDemo(() =>
  h('div', { class: 'quic-demo' }, [
    h('span', { class: 'tunnel-node mini-surface' }, '这台 Mac'),
    h('span', { class: 'tunnel-line' }, [h('span', { class: 'tunnel-lock' }, [I(LOCK)])]),
    h('span', { class: 'tunnel-node mini-surface' }, 'MacBook Pro 14'),
  ]),
)
