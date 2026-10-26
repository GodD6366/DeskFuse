import AppKit
import SwiftUI

/// Independent editor for a text clip. Saving rewrites the original library
/// item in place; the clipboard panel stays open and no new history entry
/// is created through the pasteboard.
@MainActor final class ClipboardContentEditorController: NSObject, NSWindowDelegate {
    static let shared = ClipboardContentEditorController()
    private var windows: [UInt64: NSWindow] = [:]

    /// 面板的外点击监视据此放行编辑窗口，编辑期间剪贴板面板保持打开。
    func owns(_ window: NSWindow) -> Bool {
        windows.values.contains { $0 === window }
    }

    func present(itemID: UInt64, title: String, text: String, rtf: Data, at point: NSPoint?) {
        if let existing = windows[itemID] {
            existing.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        let hosting = NSHostingController(rootView: ClipboardContentEditor(
            itemID: itemID, title: title, initial: Self.initialText(text, rtf: rtf)
        ) { [weak self] in
            self?.windows[itemID]?.performClose(nil)
        })
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 400),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = title
        window.contentViewController = hosting
        window.delegate = self
        window.minSize = NSSize(width: 480, height: 320)
        window.isReleasedWhenClosed = false
        // 剪贴板面板浮在弹出菜单层级；编辑期间面板不关闭，编辑窗口要盖过它。
        if ClipboardPanelController.shared.isShown {
            window.level = .popUpMenu
        }
        if let point { place(window, near: point) } else { window.center() }
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        windows[itemID] = window
    }

    /// 窗口左上角贴近触发点，整体收进该点所在屏幕的可见区域。
    private func place(_ window: NSWindow, near point: NSPoint) {
        guard let screen = NSScreen.screens.first(where: { NSMouseInRect(point, $0.frame, false) }) ?? NSScreen.main
        else { window.center(); return }
        let visible = screen.visibleFrame
        var frame = window.frame
        frame.origin.x = min(max(point.x - 24, visible.minX), visible.maxX - frame.width)
        frame.origin.y = min(max(point.y + 24 - frame.height, visible.minY), visible.maxY - frame.height)
        window.setFrame(frame, display: false)
    }

    /// 已保存过样式的条目带回 RTF；文字以引擎的纯文本为准，颜色交给系统
    /// 文本色，暗色模式下不会出现黑字。
    private static func initialText(_ text: String, rtf: Data) -> NSAttributedString {
        let base: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 15),
            .foregroundColor: NSColor.textColor,
        ]
        guard !rtf.isEmpty,
              let rich = NSMutableAttributedString(rtf: rtf, documentAttributes: nil),
              rich.string == text
        else { return NSAttributedString(string: text, attributes: base) }
        let range = NSRange(location: 0, length: rich.length)
        rich.removeAttribute(.backgroundColor, range: range)
        rich.addAttribute(.foregroundColor, value: NSColor.textColor, range: range)
        return rich
    }

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        windows = windows.filter { $0.value !== window }
    }
}

private struct ClipboardContentEditor: View {
    let itemID: UInt64
    let title: String
    let close: () -> Void
    @State private var attributed: NSAttributedString
    @State private var plainOnly = false
    @State private var coordinator = ClipboardTextViewBridge()

    init(itemID: UInt64, title: String, initial: NSAttributedString, close: @escaping () -> Void) {
        self.itemID = itemID
        self.title = title
        self.close = close
        _attributed = State(initialValue: initial)
    }

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            ClipboardTextView(text: $attributed, plainOnly: plainOnly, bridge: coordinator)
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
        }
        .frame(minWidth: 480, minHeight: 320)
        .background(Color(nsColor: .textBackgroundColor))
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            styleButton("B", help: "粗体") { coordinator.toggle(.bold) }
                .font(DFType.body.weight(.bold))
            styleButton("I", help: "斜体") { coordinator.toggle(.italic) }
                .italic()
            styleButton("U", help: "下划线") { coordinator.toggle(.underline) }
                .underline()
            styleButton("S", help: "删除线") { coordinator.toggle(.strikethrough) }
                .strikethrough()
            Spacer(minLength: 8)
            Button(plainOnly ? "使用富文本" : "使用纯文本") {
                plainOnly.toggle()
                if plainOnly {
                    attributed = NSAttributedString(string: attributed.string, attributes: [
                        .font: NSFont.systemFont(ofSize: 15),
                        .foregroundColor: NSColor.textColor,
                    ])
                }
            }
            .help(plainOnly ? "恢复富文本编辑" : "去掉样式，只保留文字")
            Button("保存") { save() }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.dfPrimary)
        }
        .controlSize(.regular)
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private func styleButton(_ title: String, help: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(.bordered)
            .help(help)
            .frame(minWidth: 28)
    }

    private func save() {
        ClipboardItemActions.updateText(itemID, attributed: attributed)
        close()
    }
}

private enum ClipboardTextStyle {
    case bold, italic, underline, strikethrough
}

@MainActor private final class ClipboardTextViewBridge {
    weak var textView: NSTextView?

    func toggle(_ style: ClipboardTextStyle) {
        guard let textView else { return }
        textView.window?.makeFirstResponder(textView)
        let range = textView.selectedRange
        guard range.length > 0, let storage = textView.textStorage,
              textView.shouldChangeText(in: range, replacementString: nil)
        else { return }
        // 经 shouldChange/didChange 改样式：既登记撤销，也通知代理把结果
        // 同步回 SwiftUI 状态，保存时才拿得到加粗等样式。
        storage.beginEditing()
        defer {
            storage.endEditing()
            textView.didChangeText()
        }
        switch style {
        case .bold:
            toggleTrait(.boldFontMask, in: storage, range: range, fallback: textView.font)
        case .italic:
            toggleTrait(.italicFontMask, in: storage, range: range, fallback: textView.font)
        case .underline:
            toggleLineStyle(.underlineStyle, in: storage, range: range)
        case .strikethrough:
            toggleLineStyle(.strikethroughStyle, in: storage, range: range)
        }
    }

    private func toggleTrait(
        _ trait: NSFontTraitMask, in storage: NSTextStorage, range: NSRange, fallback: NSFont?
    ) {
        let font = (storage.attribute(.font, at: range.location, effectiveRange: nil) as? NSFont)
            ?? fallback
            ?? NSFont.systemFont(ofSize: 15)
        let manager = NSFontManager.shared
        let next = manager.traits(of: font).contains(trait)
            ? manager.convert(font, toNotHaveTrait: trait)
            : manager.convert(font, toHaveTrait: trait)
        storage.addAttribute(.font, value: next, range: range)
    }

    private func toggleLineStyle(_ key: NSAttributedString.Key, in storage: NSTextStorage, range: NSRange) {
        let current = (storage.attribute(key, at: range.location, effectiveRange: nil) as? NSNumber)?.intValue ?? 0
        if current == 0 {
            storage.addAttribute(key, value: NSUnderlineStyle.single.rawValue, range: range)
        } else {
            storage.removeAttribute(key, range: range)
        }
    }
}

private struct ClipboardTextView: NSViewRepresentable {
    @Binding var text: NSAttributedString
    var plainOnly: Bool
    var bridge: ClipboardTextViewBridge

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        let view = NSTextView()
        view.delegate = context.coordinator
        view.isRichText = !plainOnly
        view.allowsUndo = true
        view.usesFindBar = true
        view.isAutomaticQuoteSubstitutionEnabled = false
        view.font = .systemFont(ofSize: 15)
        view.textContainerInset = NSSize(width: 4, height: 8)
        view.minSize = NSSize(width: 0, height: 0)
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.isVerticallyResizable = true
        view.isHorizontallyResizable = false
        view.textContainer?.widthTracksTextView = true
        view.drawsBackground = false
        view.textStorage?.setAttributedString(text)
        scroll.documentView = view
        bridge.textView = view
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let view = scroll.documentView as? NSTextView else { return }
        view.isRichText = !plainOnly
        bridge.textView = view
        if view.attributedString() != text {
            view.textStorage?.setAttributedString(text)
        }
    }

    @MainActor final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<NSAttributedString>
        init(text: Binding<NSAttributedString>) { self.text = text }
        func textDidChange(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            text.wrappedValue = view.attributedString()
        }
    }
}
