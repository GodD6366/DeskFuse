import AppKit
import OSLog
import Quartz

/// Space 打开的系统快速查看（QLPreviewPanel）。剪贴板面板和主窗口都在界面进程里，
/// 所以面板只上屏、不抢键盘：空格、方向键和 Esc 仍由列表处理，列表再告诉这里切换或关闭。
@MainActor final class ClipboardSystemQuickLook: NSObject {
    static let shared = ClipboardSystemQuickLook()
    private static let logger = Logger(subsystem: "cc.deskfuse", category: "quick_look")

    private var urls: [URL] = []
    private var itemID: UInt64?
    /// 用户想让快速查看开着。内容还在加载时面板尚未上屏，方向键切换也要跟随。
    private var active = false
    private var request: UUID?

    var isVisible: Bool { QLPreviewPanel.sharedPreviewPanelExists() && QLPreviewPanel.shared().isVisible }
    var window: NSWindow? { isVisible ? QLPreviewPanel.shared() : nil }

    func toggle(_ id: UInt64) {
        if active, itemID == id { close("toggle") } else { show(id) }
    }

    /// 选择变化时调用；没有打开快速查看就什么也不做。
    func follow(_ id: UInt64) {
        guard active, itemID != id else { return }
        show(id)
    }

    /// 返回关闭前是否开着，Esc 据此决定是关快速查看还是继续收起面板。
    @discardableResult
    func close(_ reason: String = "unspecified") -> Bool {
        let wasOpen = active || isVisible
        if wasOpen { Self.logger.notice("close reason=\(reason, privacy: .public)") }
        active = false
        request = nil
        itemID = nil
        // 只收起可见的面板；数据源由 endPreviewPanelControl 解绑。面板没有控制者时再去改它会被系统拒绝。
        if QLPreviewPanel.sharedPreviewPanelExists(), let panel = QLPreviewPanel.shared(), panel.isVisible {
            panel.orderOut(nil)
        }
        urls = []
        return wasOpen
    }

    private func show(_ id: UInt64) {
        Self.logger.notice("show id=\(id, privacy: .public)")
        active = true
        itemID = id
        let token = UUID()
        request = token
        Task { @MainActor in
            do {
                let urls = try await ClipboardItemActions.quickLookURLs(id)
                guard request == token, active else { return }
                present(urls)
            } catch {
                Self.logger.error("load id=\(id, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
                guard request == token else { return }
                close("load failed")
                ClipboardItemActions.report(error.userFacingMessage)
            }
        }
    }

    private func present(_ urls: [URL]) {
        self.urls = urls
        guard let panel = QLPreviewPanel.shared() else { return }
        // 不抢键盘：空格、方向键和 Esc 继续由剪贴板列表处理。
        if !panel.isVisible { panel.orderFrontRegardless() }
        // orderFront 不会让面板去找控制者；不显式更新，数据源会被丢掉，只显示 “No items selected”。
        panel.updateController()
        if panel.dataSource == nil { attach(panel) }
        panel.reloadData()
        panel.currentPreviewItemIndex = 0
        raise(panel)
        Self.logger.notice("present id=\(self.itemID ?? 0, privacy: .public) items=\(urls.count, privacy: .public) controller=\(panel.dataSource === self, privacy: .public) level=\(panel.level.rawValue, privacy: .public) key=\(NSApp.keyWindow.map { String(describing: type(of: $0)) } ?? "nil", privacy: .public)")
        // 面板上屏后会重新计算层级，下一轮再压一次。
        DispatchQueue.main.async { [weak self] in
            guard let self, self.active, panel.isVisible else { return }
            self.raise(panel)
        }
    }

    /// 剪贴板弹窗是 `.popUpMenu` 层级；快速查看必须在它之上，也要能盖住全屏应用。
    private func raise(_ panel: QLPreviewPanel) {
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.popUpMenu.rawValue + 1)
        panel.collectionBehavior.formUnion([.canJoinAllSpaces, .fullScreenAuxiliary])
    }

    private func attach(_ panel: QLPreviewPanel) {
        panel.dataSource = self
        panel.delegate = self
    }

    // QLPreviewPanel 会沿响应链找“控制者”；找不到就清空数据源，只剩 “No items selected”。
    // 剪贴板弹窗和 AppDelegate 都把这三步转到这里，无论哪个窗口是 key 都能接管。
    func acceptsControl() -> Bool { active }
    func beginControl(_ panel: QLPreviewPanel) { attach(panel) }
    func endControl(_ panel: QLPreviewPanel) {
        if panel.dataSource === self { panel.dataSource = nil }
        if panel.delegate === self { panel.delegate = nil }
    }
}

extension ClipboardSystemQuickLook: @preconcurrency QLPreviewPanelDataSource, @preconcurrency QLPreviewPanelDelegate {
    func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int { urls.count }

    func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> (any QLPreviewItem)! {
        urls.indices.contains(index) ? urls[index] as NSURL : nil
    }

    /// 用户点过快速查看后键盘到了这里：空格和列表里一样用来关闭。
    func previewPanel(_ panel: QLPreviewPanel!, handle event: NSEvent!) -> Bool {
        guard event.type == .keyDown, event.keyCode == 49,
              event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty else { return false }
        close("space in panel")
        return true
    }

    /// 用户用快速查看自己的关闭按钮或 Esc 关掉时，同步状态，下一次空格重新打开。
    func windowWillClose(_ notification: Notification) {
        Self.logger.notice("panel windowWillClose active=\(self.active, privacy: .public)")
        active = false
        request = nil
        itemID = nil
    }
}
