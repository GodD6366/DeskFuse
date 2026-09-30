import AppKit

/// Presents the active remote application through DeskFuse's Dock tile.
///
/// macOS owns one Dock tile per application process. Remote windows all live
/// in the DeskFuse process, so the tile temporarily adopts the focused remote
/// application's icon and carries an explicit "远程" badge while any remote
/// window is open. The original DeskFuse icon and Dock visibility policy return
/// when the last remote window closes.
@available(macOS 14.2, *)
@MainActor final class RemoteApplicationDockController: NSObject {
    static let shared = RemoteApplicationDockController()

    private struct Entry {
        let title: String
        let icon: NSImage?
    }

    private var entries: [UUID: Entry] = [:]
    private var order: [UUID] = []
    private var activeID: UUID?
    private var originalIcon: NSImage?

    private override init() { super.init() }

    var hasActiveApplication: Bool { !entries.isEmpty }

    func present(id: UUID, title: String, iconPNG: Data?) {
        if entries.isEmpty {
            originalIcon = NSApp.applicationIconImage.copy() as? NSImage
        }
        if entries[id] == nil { order.append(id) }
        entries[id] = Entry(title: title, icon: iconPNG.flatMap { NSImage(data: $0) })
        activeID = id
        refreshDockTile()
    }

    func focus(id: UUID) {
        guard entries[id] != nil else { return }
        activeID = id
        refreshDockTile()
    }

    func close(id: UUID) {
        entries.removeValue(forKey: id)
        order.removeAll { $0 == id }
        if activeID == id { activeID = order.first(where: { entries[$0] != nil }) }
        if entries.isEmpty {
            restoreDockTile()
        } else {
            refreshDockTile()
        }
    }

    func reset() {
        guard !entries.isEmpty else { return }
        entries.removeAll()
        order.removeAll()
        activeID = nil
        restoreDockTile()
    }

    /// Builds the menu shown when the user secondary-clicks the single Dock tile.
    /// Each item keeps the individual remote application's icon and focuses its
    /// corresponding window, so multiple remote applications remain discoverable.
    func makeDockMenu() -> NSMenu? {
        guard !entries.isEmpty else { return nil }
        let menu = NSMenu()
        menu.autoenablesItems = false

        let heading = NSMenuItem(title: "打开的远程应用", action: nil, keyEquivalent: "")
        heading.isEnabled = false
        menu.addItem(heading)
        menu.addItem(.separator())

        for id in order {
            guard let entry = entries[id] else { continue }
            let title = entry.title.count > 30 ? String(entry.title.prefix(29)) + "…" : entry.title
            let item = NSMenuItem(title: title, action: #selector(selectDockMenuItem(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = id
            item.image = entry.icon
            item.state = id == activeID ? .on : .off
            menu.addItem(item)
        }
        return menu
    }

    @objc private func selectDockMenuItem(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? UUID else { return }
        _ = RemoteApplicationManager.shared.focusViewer(id: id)
    }

    private func refreshDockTile() {
        if let icon = activeID.flatMap({ entries[$0]?.icon }) {
            NSApp.applicationIconImage = icon
        } else if let originalIcon {
            NSApp.applicationIconImage = originalIcon
        }
        // 远程应用窗口打开时，遵循全局 Dock 图标设置
        // 如果用户选择隐藏 Dock 图标，保持 .accessory 策略
        if NSApp.activationPolicy() != .regular {
            // 当前是隐藏状态，不强制显示
            NSApp.dockTile.badgeLabel = nil
        } else {
            NSApp.dockTile.badgeLabel = "远程"
        }
        NSApp.dockTile.display()
    }

    private func restoreDockTile() {
        if let originalIcon { NSApp.applicationIconImage = originalIcon }
        self.originalIcon = nil
        NSApp.dockTile.badgeLabel = nil
        NSApp.dockTile.display()
        AppStore.shared.applyDockIconPolicy()
    }
}
