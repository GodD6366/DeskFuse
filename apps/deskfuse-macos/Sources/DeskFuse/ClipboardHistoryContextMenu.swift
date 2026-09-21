import AppKit
import SwiftUI

/// A native menu owns its rows for the entire tracking session. SwiftUI card
/// refreshes (preview, polling, selection) must not replace an open submenu.
@MainActor enum ClipboardHistoryMenuSession {
    static var isTracking = false
    static let didEnd = Notification.Name("DeskFuse.clipboardHistoryMenuDidEnd")
}

extension View {
    func clipboardHistoryContextMenu(
        itemID: UInt64, memberships: [String],
        meta: ClipboardItemMeta?,
        onCopy: (() -> Void)? = nil, copyEnabled: Bool = true,
        onEdit: @escaping () -> Void, onDelete: @escaping () -> Void
    ) -> some View {
        overlay {
            ClipboardHistoryContextMenu(itemID: itemID, memberships: memberships,
                meta: meta, onCopy: onCopy, copyEnabled: copyEnabled, onEdit: onEdit, onDelete: onDelete)
                .accessibilityHidden(true)
        }
    }
}

private struct ClipboardHistoryContextMenu: NSViewRepresentable {
    let itemID: UInt64
    let memberships: [String]
    let meta: ClipboardItemMeta?
    let onCopy: (() -> Void)?
    let copyEnabled: Bool
    let onEdit: () -> Void
    let onDelete: () -> Void

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
            let value = configuration
            let menu = NSMenu()
            menu.autoenablesItems = false
            var targets: [MenuAction] = []
            func add(_ title: String, to menu: NSMenu, enabled: Bool = true,
                     checked: Bool = false, action: @escaping () -> Void) {
                let target = MenuAction(action)
                targets.append(target)
                let item = NSMenuItem(title: title, action: #selector(MenuAction.invoke(_:)), keyEquivalent: "")
                item.target = target
                item.isEnabled = enabled
                item.state = checked ? .on : .off
                menu.addItem(item)
            }
            if let copy = value.onCopy {
                add("仅复制", to: menu, enabled: value.copyEnabled, action: copy)
                menu.addItem(.separator())
            }

            // Show origin device and availability status for remote items
            if let meta = value.meta, meta.remote {
                let deviceName = meta.originDevice ?? "未知设备"
                let statusIcon: String
                let statusText: String
                if meta.available {
                    statusIcon = "✓"
                    statusText = "来自 \(deviceName)（已获取）"
                } else {
                    statusIcon = "↓"
                    statusText = "来自 \(deviceName)（使用时自动获取）"
                }
                add("\(statusIcon) \(statusText)", to: menu, enabled: false, action: {})
                menu.addItem(.separator())
            }

            add("编辑标题", to: menu, action: value.onEdit)
            menu.addItem(.separator())
            let membershipMenu = NSMenu(title: "手动列表")
            membershipMenu.autoenablesItems = false
            let lists = ClipboardLibraryStore.shared.lists.filter { $0.kind == "manual" }
            if lists.isEmpty {
                add("请先新建手动列表", to: membershipMenu, enabled: false, action: {})
            }
            for list in lists {
                let present = value.memberships.contains(list.id)
                add(list.name, to: membershipMenu, checked: present) {
                    ClipboardLibraryStore.shared.perform([
                        "op": "membership", "list_id": list.id,
                        "history_id": value.itemID, "present": !present,
                    ])
                }
            }
            let membership = NSMenuItem(title: "手动列表", action: nil, keyEquivalent: "")
            membership.submenu = membershipMenu
            menu.addItem(membership)
            menu.addItem(.separator())
            add("删除", to: menu, action: value.onDelete)

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
