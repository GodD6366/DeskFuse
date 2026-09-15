import AppKit
import SwiftUI

/// SwiftUI has no macOS TabView style with an external tab selector. AppKit
/// retains each hosting controller (and its draft state), loads pages lazily,
/// and removes inactive pages from keyboard focus and accessibility traversal.
/// Selection and settings values remain owned by the SwiftUI parent.
struct SettingsPageContainer: NSViewControllerRepresentable {
    struct Page {
        let id: String
        let content: AnyView
    }

    let selection: String
    let pages: [Page]

    func makeNSViewController(context: Context) -> NSTabViewController {
        let controller = NSTabViewController()
        controller.tabStyle = .unspecified
        controller.tabView.tabViewType = .noTabsNoBorder
        controller.tabView.drawsBackground = false
        controller.transitionOptions = []
        controller.canPropagateSelectedChildViewControllerTitle = false
        for page in pages {
            let host = NSHostingController(rootView: page.content)
            // Fill the parent; an empty form must not shrink the tab container.
            host.sizingOptions = []
            let item = NSTabViewItem(viewController: host)
            item.identifier = page.id
            controller.addTabViewItem(item)
        }
        selectPage(in: controller)
        return controller
    }

    func updateNSViewController(_ controller: NSTabViewController, context: Context) {
        for (item, page) in zip(controller.tabViewItems, pages) {
            if let host = item.viewController as? NSHostingController<AnyView> {
                host.rootView = page.content
            }
        }
        selectPage(in: controller)
    }

    private func selectPage(in controller: NSTabViewController) {
        let index = pages.firstIndex { $0.id == selection } ?? 0
        if controller.selectedTabViewItemIndex != index {
            controller.selectedTabViewItemIndex = index
        }
    }
}
