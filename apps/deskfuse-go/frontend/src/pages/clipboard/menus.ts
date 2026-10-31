// 历史行、卡片与预览“更多”共用的菜单模型，对应 ClipboardHistoryContextMenu.swift 与 ClipboardPreviewPane.moreMenu。

import type { ItemMeta, LibraryItem } from '../../types/clipboard'
import {
  contentTypeOf, editExternally, loadPeers, loadWorkflows, openInPreview, revealItem, runWorkflow, saveItem,
  shareToPeer, showShelf,
} from './actions'
import { errorMessage, library, setMembership } from './library'
import type { MenuItem } from './menu'

export const isMac = /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? '')
export const modKey = isMac ? '⌘' : 'Ctrl+'

export interface ItemMenuContext {
  item: LibraryItem
  title: string | null
  meta?: ItemMeta
  copyEnabled?: boolean
  /** 面板持有“粘贴目标应用”时显示「粘贴至“X”」；主页面不提供。 */
  pasteTarget?: string | null
  onPaste?: () => void
  onPastePlainText?: () => void
  /** 与原生 ClipboardPreviewPolicy 一致：不可预览的条目没有「预览」项。 */
  showsPreview?: boolean
  onPreview?: () => void
  onCopy: () => void
  onEditTitle: () => void
  onEditText: () => void
  onDelete: () => void
  fail: (message: string) => void
  /** 工作流功能关闭时，“发送到设备”只提供直接发送。 */
  workflowsEnabled?: boolean
}

const attempt = (ctx: ItemMenuContext, task: () => Promise<unknown>) => () => {
  void task().catch((error) => ctx.fail(errorMessage(error)))
}

const saveLabel = (item: LibraryItem) => (item.has_image ? '保存图片' : item.file_count > 0 ? '保存文件' : '保存文本')

function peersMenu(ctx: ItemMenuContext): MenuItem {
  return {
    label: '发送到设备',
    load: async () => {
      const peers = await loadPeers()
      if (peers.length === 0) return [{ label: '请先添加一台设备', disabled: true }]
      const type = contentTypeOf(ctx.item)
      const flows = ctx.workflowsEnabled === false
        ? []
        : (await loadWorkflows()).filter((flow) => flow.enabled && (!flow.acceptsKnown || flow.accepts.includes(type)))
      return peers.map((peer) => ({
        label: peer.name,
        disabled: !peer.connected,
        // 与原生 WorkflowShareMenu 一致：先直接发送，工作流作为该设备下的附加选项。
        children: [
          { label: '直接发送', run: attempt(ctx, () => shareToPeer(ctx.item.id, peer.id)) },
          ...(flows.length
            ? [{ separator: true } as MenuItem, ...flows.map((flow) => ({
                label: `使用「${flow.name}」发送`,
                run: attempt(ctx, () => shareToPeer(ctx.item.id, peer.id, flow.id)),
              }))]
            : []),
        ],
      }))
    },
  }
}

function workflowsMenu(ctx: ItemMenuContext): MenuItem {
  return {
    label: '使用工作流',
    load: async () => {
      const type = contentTypeOf(ctx.item)
      const flows = (await loadWorkflows()).filter((f) => f.enabled && (!f.acceptsKnown || f.accepts.includes(type)))
      if (flows.length === 0) return [{ label: '没有适合此内容的已启用工作流', disabled: true }]
      return flows.map((flow) => ({ label: flow.name, run: attempt(ctx, () => runWorkflow(flow.id, ctx.item.id)) }))
    },
  }
}

/** 右键菜单。 */
export function itemMenu(ctx: ItemMenuContext): MenuItem[] {
  const { item, meta } = ctx
  const canUse = ctx.copyEnabled !== false
  const items: MenuItem[] = []

  if (meta?.remote) {
    const device = meta.originDevice ?? '未知设备'
    items.push({ label: meta.available ? `来自 ${device}（已获取）` : `来自 ${device}（使用时自动获取）`, disabled: true }, { separator: true })
  }

  if (ctx.pasteTarget && ctx.onPaste) {
    items.push({ label: `粘贴至“${ctx.pasteTarget}”`, icon: 'paste', disabled: !canUse, run: ctx.onPaste })
    if (item.file_count === 0 && !item.has_image && ctx.onPastePlainText) {
      items.push({ label: '粘贴为纯文本', disabled: !canUse, run: ctx.onPastePlainText })
    }
  }
  items.push({ label: '复制', shortcut: `${modKey}C`, disabled: !canUse, run: ctx.onCopy }, { separator: true })

  const hasTitle = (ctx.title ?? '').trim() !== ''
  items.push({ label: hasTitle ? '编辑标题' : '添加标题', shortcut: `${modKey}T`, run: ctx.onEditTitle })
  items.push({ label: saveLabel(item), shortcut: `${modKey}S`, disabled: !canUse, run: attempt(ctx, () => saveItem(item)) })
  if (item.file_count === 0) {
    items.push({
      label: item.has_image ? '编辑图片' : '编辑文本', shortcut: `${modKey}E`, disabled: !canUse,
      run: item.has_image ? attempt(ctx, () => editExternally(item, ctx.title)) : ctx.onEditText,
    })
  }
  if (item.file_count > 0 || item.has_image) {
    items.push({
      label: item.file_count > 0 ? '在“预览”中打开文件' : '在“预览”中打开', shortcut: `${modKey}O`, disabled: !canUse,
      run: attempt(ctx, () => openInPreview(item.id)),
    })
  }
  if (item.file_count > 0) {
    items.push({ label: '在访达中显示', disabled: !canUse, run: attempt(ctx, () => revealItem(item.id)) })
  }

  const manual = library.lists.filter((l) => l.kind === 'manual')
  items.push({
    label: '添加到列表',
    children: manual.length === 0
      ? [{ label: '请先新建手动列表', disabled: true }]
      : manual.map((list) => {
          const present = item.memberships.includes(list.id)
          return { label: list.name, checked: present, run: attempt(ctx, () => setMembership(list.id, item.id, !present)) }
        }),
  })
  items.push({ separator: true }, { label: '删除', icon: 'trash', run: ctx.onDelete }, { separator: true })
  if (item.file_count === 0 && ctx.showsPreview && ctx.onPreview) {
    items.push({ label: '预览', shortcut: '空格', run: ctx.onPreview })
  }
  items.push(peersMenu(ctx), workflowsMenu(ctx))
  return items
}

/** 预览面板右下角的“更多操作”菜单。 */
export function moreMenu(ctx: ItemMenuContext & { copying: boolean; hasMissingFiles: boolean; hasLocalFiles: boolean }): MenuItem[] {
  const { item } = ctx
  const items: MenuItem[] = [
    { label: '复制原始内容', icon: 'copy', disabled: ctx.copying || ctx.hasMissingFiles, run: ctx.onCopy },
  ]
  if (item.file_count === 0) {
    items.push({
      label: item.has_image ? '编辑图片' : '编辑文本', icon: 'pencil',
      run: item.has_image ? attempt(ctx, () => editExternally(item, ctx.title)) : ctx.onEditText,
    })
  }
  items.push({
    label: item.has_image ? '保存图片…' : item.file_count > 0 ? '保存文件…' : '保存文本…', icon: 'download',
    run: attempt(ctx, () => saveItem(item)),
  })
  items.push(workflowsMenu(ctx), peersMenu({ ...ctx }))
  if (item.file_count > 0 && ctx.hasLocalFiles) {
    items.push(
      { label: '在访达中显示', icon: 'folder', run: attempt(ctx, () => revealItem(item.id)) },
      { label: '在“预览”中打开', icon: 'eye', run: attempt(ctx, () => openInPreview(item.id)) },
    )
  }
  items.push({ separator: true }, { label: '打开文件暂存区', icon: 'tray', run: attempt(ctx, () => showShelf()) })
  return items
}
