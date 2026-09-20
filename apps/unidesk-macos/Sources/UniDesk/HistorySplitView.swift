import SwiftUI
import AppKit

/// Keep the native divider and allow manual resizing, starting history at its
/// maximum width while preserving the preview's minimum size.
struct HistorySplitView<History: View, Detail: View>: NSViewControllerRepresentable {
    var expandHistory = false
    @ViewBuilder let history: () -> History
    @ViewBuilder let detail: () -> Detail

    final class Controller: NSSplitViewController {
        let historyHost: NSHostingController<History>
        let detailHost: NSHostingController<Detail>
        private var positioned = false
        private var expanded: Bool
        private var listWidth: CGFloat = 360

        init(history: History, detail: Detail, expanded: Bool) {
            self.expanded = expanded
            historyHost = NSHostingController(rootView: history)
            detailHost = NSHostingController(rootView: detail)
            super.init(nibName: nil, bundle: nil)
        }
        required init?(coder: NSCoder) { nil }

        override func viewDidLoad() {
            super.viewDidLoad()
            splitView.isVertical = true
            splitView.dividerStyle = .thin
            let historyItem = NSSplitViewItem(viewController: historyHost)
            historyItem.minimumThickness = 240
            historyItem.maximumThickness = 360
            historyItem.holdingPriority = NSLayoutConstraint.Priority(260)
            let detailItem = NSSplitViewItem(viewController: detailHost)
            detailItem.minimumThickness = 340
            detailItem.holdingPriority = .defaultLow
            addSplitViewItem(historyItem)
            addSplitViewItem(detailItem)
        }

        override func viewDidLayout() {
            super.viewDidLayout()
            guard !positioned, splitView.bounds.width >= 600 else { return }
            positioned = true
            // Run after the initial hosting-view constraints have been installed.
            DispatchQueue.main.async { [weak self] in
                self?.positionHistory()
            }
        }

        func updateExpansion(_ expanded: Bool) {
            guard self.expanded != expanded else { return }
            if expanded, positioned {
                listWidth = min(360, max(240, historyHost.view.frame.width))
            }
            self.expanded = expanded
            guard positioned else { return }
            DispatchQueue.main.async { [weak self] in self?.positionHistory() }
        }

        private func positionHistory() {
            // Expand to the existing history limit, retaining room for preview.
            let available = splitView.bounds.width - splitView.dividerThickness - 340
            splitView.setPosition(min(expanded ? 360 : listWidth, max(240, available)), ofDividerAt: 0)
        }
    }

    func makeNSViewController(context: Context) -> Controller {
        Controller(history: history(), detail: detail(), expanded: expandHistory)
    }
    func updateNSViewController(_ controller: Controller, context: Context) {
        controller.updateExpansion(expandHistory)
        controller.historyHost.rootView = history()
        controller.detailHost.rootView = detail()
    }
}
