import AppKit
import SwiftUI
import DeskFuseEngine

/// A native menu owns its rows for the entire tracking session. SwiftUI card
/// refreshes (preview, polling, selection) must not replace an open submenu.
@MainActor enum ClipboardHistoryMenuSession {
    static var isTracking = false
    static let didEnd = Notification.Name("DeskFuse.clipboardHistoryMenuDidEnd")
}

struct ClipboardHistoryMenuContext {
    var item: HistoryItem
    var title: String?
    var memberships: [String]
    var meta: ClipboardItemMeta?
    var copyEnabled = true
    var pasteTargetName: String?
    var onCopy: (() -> Void)?
    var onPaste: (() -> Void)?
    var onEditTitle: () -> Void
    var showsPreview = false
    var onPreview: (() -> Void)?
    var onDelete: () -> Void
}

extension View {
    func clipboardHistoryContextMenu(_ context: ClipboardHistoryMenuContext) -> some View {
        overlay {
            ClipboardHistoryContextMenu(context: context)
                .accessibilityHidden(true)
        }
    }
}

private struct ClipboardHistoryContextMenu: NSViewRepresentable {
    let context: ClipboardHistoryMenuContext

    func makeNSView(context: Context) -> MenuView { MenuView(configuration: self) }
    func updateNSView(_ view: MenuView, context: Context) { view.configuration = self }

    @MainActor final class MenuAction: NSObject {
        let action: () -> Void
        init(_ action: @escaping () -> Void) { self.action = action }
        @objc func invoke(_ sender: NSMenuItem) { action() }
    }

    @MainActor final class MenuView: NSView {
        var configuration: ClipboardHistoryContextMenu
        init(configuration: ClipboardHistoryContextMenu) {
            self.configuration = configuration
            super.init(frame: .zero)
        }
        required init?(coder: NSCoder) { nil }

        // Leave selection, double-click, scrolling and dragging with the card.
        override func hitTest(_ point: NSPoint) -> NSView? {
            guard let event = NSApp.currentEvent,
                  event.type == .rightMouseDown ||
                    (event.type == .leftMouseDown && event.modifierFlags.contains(.control))
            else { return nil }
            return super.hitTest(point)
        }
        override func rightMouseDown(with event: NSEvent) { showMenu(event) }
        override func mouseDown(with event: NSEvent) {
            if event.modifierFlags.contains(.control) { showMenu(event) }
            else { super.mouseDown(with: event) }
        }

        private func showMenu(_ event: NSEvent) {
            let value = configuration.context
            let item = value.item
            ClipboardLibraryStore.shared.remember(item, memberships: value.memberships, title: value.title)
            let menu = NSMenu()
            menu.autoenablesItems = false
            var targets: [MenuAction] = []
            func add(_ title: String, to menu: NSMenu, enabled: Bool = true,
                     checked: Bool = false, key: String = "", modifiers: NSEvent.ModifierFlags = .command,
                     image: String? = nil, action: @escaping () -> Void) {
                let target = MenuAction(action)
                targets.append(target)
                let row = NSMenuItem(title: title, action: #selector(MenuAction.invoke(_:)), keyEquivalent: key)
                row.target = target
                row.isEnabled = enabled
                row.state = checked ? .on : .off
                if !key.isEmpty { row.keyEquivalentModifierMask = modifiers }
                if let image {
                    row.image = NSImage(systemSymbolName: image, accessibilityDescription: title)
                }
                menu.addItem(row)
            }
            func submenu(_ title: String, enabled: Bool = true, build: (NSMenu) -> Void) {
                let child = NSMenu(title: title)
                child.autoenablesItems = false
                build(child)
                let row = NSMenuItem(title: title, action: nil, keyEquivalent: "")
                row.submenu = child
                row.isEnabled = enabled && child.items.contains(where: \.isEnabled)
                menu.addItem(row)
            }

            if let meta = value.meta, meta.remote {
                let deviceName = meta.originDevice ?? "未知设备"
                let status = meta.available
                    ? "来自 \(deviceName)（已获取）"
                    : "来自 \(deviceName)（使用时自动获取）"
                add(status, to: menu, enabled: false, action: {})
                menu.addItem(.separator())
            }

            let canUse = value.copyEnabled
            let plainText = ClipboardItemActions.canCopyPlainText(item)
            if let name = value.pasteTargetName, let paste = value.onPaste {
                add("粘贴至“\(name)”", to: menu, enabled: canUse, image: "arrow.uturn.forward") {
                    paste()
                }
                if plainText {
                    add("粘贴为纯文本", to: menu, enabled: canUse) {
                        ClipboardItemActions.copy(item.id, plainText: true, paste: true)
                    }
                }
            }
            add("复制", to: menu, enabled: canUse, key: "c") {
                if let copy = value.onCopy {
                    copy()
                } else {
                    ClipboardItemActions.copy(item.id, paste: false)
                }
            }
            menu.addItem(.separator())

            let hasTitle = !(value.title ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            add(hasTitle ? "编辑标题" : "添加标题", to: menu, key: "t", action: value.onEditTitle)
            add(item.hasImage ? "保存图片" : item.fileCount > 0 ? "保存文件" : "保存文本",
                to: menu, enabled: canUse, key: "s") {
                ClipboardPanelController.shared.close()
                ClipboardItemActions.save(item)
            }
            if item.fileCount == 0 {
                add(item.hasImage ? "编辑图片" : "编辑文本", to: menu, enabled: canUse, key: "e") {
                    // 图片经外部编辑器（预览.app）打开，焦点离开本应用，关闭面板；
                    // 文本编辑是应用内窗口，面板保持打开，保存时原地更新条目。
                    if item.hasImage { ClipboardPanelController.shared.close() }
                    ClipboardItemActions.edit(item, title: value.title)
                }
            }
            if item.fileCount > 0 || item.hasImage {
                add(item.fileCount > 0 ? "在“预览”中打开文件" : "在“预览”中打开", to: menu, enabled: canUse, key: "o") {
                    ClipboardPanelController.shared.close()
                    ClipboardItemActions.openInPreview(item.id)
                }
            }
            if item.fileCount > 0 {
                add("在访达中显示", to: menu, enabled: canUse) {
                    ClipboardItemActions.revealInFinder(item.id)
                }
            }

            submenu("添加到列表") { child in
                let lists = ClipboardLibraryStore.shared.lists.filter { $0.kind == "manual" }
                if lists.isEmpty {
                    add("请先新建手动列表", to: child, enabled: false, action: {})
                }
                for list in lists {
                    let present = value.memberships.contains(list.id)
                    add(list.name, to: child, checked: present) {
                        ClipboardLibraryStore.shared.perform([
                            "op": "membership", "list_id": list.id,
                            "history_id": item.id, "present": !present,
                        ])
                    }
                }
            }
            menu.addItem(.separator())
            add("删除", to: menu, image: "delete.left", action: value.onDelete)
            menu.addItem(.separator())

            if item.fileCount == 0, value.showsPreview, let preview = value.onPreview {
                add("预览", to: menu, key: " ", modifiers: []) {
                    preview()
                }
            }
            submenu("发送到设备") { child in
                let peers = (AppStore.shared.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked }
                if peers.isEmpty {
                    add("请先添加一台设备", to: child, enabled: false, action: {})
                }
                for peer in peers {
                    add(peer.name, to: child, enabled: peer.connected) {
                        AppStore.shared.shareWorkflow(nil, historyID: item.id, peerID: peer.id)
                    }
                }
            }
            if AppStore.shared.workflowsEnabled {
                submenu("使用工作流") { child in
                    let type = ClipboardItemActions.contentType(for: item)
                    let flows = AppStore.shared.workflowDefinitions.filter { $0.enabled && $0.accepts(type) }
                    if flows.isEmpty {
                        add("没有适合此内容的已启用工作流", to: child, enabled: false, action: {})
                    }
                    for flow in flows {
                        add(flow.name, to: child) {
                            AppStore.shared.runWorkflow(flow.id, historyID: item.id)
                        }
                    }
                }
            }

            ClipboardHistoryMenuSession.isTracking = true
            defer {
                ClipboardHistoryMenuSession.isTracking = false
                NotificationCenter.default.post(name: ClipboardHistoryMenuSession.didEnd, object: nil)
            }
            // NSMenuItem.target is weak; retain action closures until tracking ends.
            withExtendedLifetime(targets) {
                NSMenu.popUpContextMenu(menu, with: event, for: self)
            }
        }
    }
}
