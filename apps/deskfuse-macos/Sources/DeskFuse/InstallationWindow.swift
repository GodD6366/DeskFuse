import AppKit
import SwiftUI
import Observation

@MainActor
@Observable
final class InstallationModel {
    enum Phase: Equatable { case welcome, installing, installed, opening, failed }
    var phase: Phase = .welcome
    var english = !(Locale.preferredLanguages.first?.hasPrefix("zh") ?? false)
    var failure: String?
    let destination: URL
    let icon: NSImage
    private var installationFailure: InstallationFailure?

    init(destination: URL = AppInstallation.destination(), icon: NSImage? = nil) {
        self.destination = destination
        self.icon = icon ?? Bundle.main.url(forResource: "DeskFuse-icon-1024", withExtension: "png")
            .flatMap(NSImage.init(contentsOf:)) ?? NSImage(named: NSImage.applicationIconName) ?? NSImage()
    }

    var busy: Bool { phase == .installing || phase == .opening }
    func text(_ chinese: String, _ english: String) -> String { self.english ? english : chinese }
    var title: String {
        switch phase {
        case .welcome: text("欢迎安装 DeskFuse", "Welcome to DeskFuse")
        case .installing: text("正在安装 DeskFuse", "Installing DeskFuse")
        case .installed, .opening: text("DeskFuse 已安装", "DeskFuse is installed")
        case .failed: text("安装尚未完成", "Installation incomplete")
        }
    }
    var subtitle: String {
        switch phase {
        case .welcome:
            text("让多台电脑，像一台一样工作。\n共享剪贴板、文件和键鼠。",
                 "Your computers, working as one.\nShare your clipboard, files, and controls.")
        case .installing:
            text("正在复制到应用程序文件夹，请稍候。", "Copying to Applications. Just a moment.")
        case .installed, .opening:
            text("DeskFuse 已可以使用。", "DeskFuse is ready to use.")
        case .failed:
            installationFailure?.message(english: english) ?? failure
                ?? text("请重试安装。", "Please try installing again.")
        }
    }
    var buttonTitle: String {
        switch phase {
        case .welcome: text("点击安装", "Install")
        case .installing: text("正在安装…", "Installing…")
        case .installed: text("打开 DeskFuse", "Open DeskFuse")
        case .opening: text("正在打开…", "Opening…")
        case .failed: text("重新安装", "Try again")
        }
    }

    func performAction() {
        guard !busy else { return }
        if phase == .installed { openInstalledApplication(); return }
        Task { await install() }
    }

    private func install() async {
        guard !busy else { return }
        let previousPhase = phase
        phase = .installing
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: "com.deskfuse.app")
            .filter { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }
        if !running.isEmpty {
            let alert = NSAlert()
            alert.messageText = text("退出 DeskFuse 并安装？", "Quit DeskFuse and install?")
            alert.informativeText = text("安装需要退出正在运行的 DeskFuse。你的设置和数据会保留。",
                                         "DeskFuse needs to quit before installation. Your settings and data will be kept.")
            alert.addButton(withTitle: text("退出并安装", "Quit and install"))
            alert.addButton(withTitle: text("取消", "Cancel"))
            guard alert.runModal() == .alertFirstButtonReturn else {
                phase = previousPhase
                return
            }
        }
        failure = nil
        installationFailure = nil
        do {
            for app in running { app.terminate() }
            for _ in 0..<100 {
                if running.allSatisfy(\.isTerminated) { break }
                try await Task.sleep(for: .milliseconds(100))
            }
            guard running.allSatisfy(\.isTerminated) else { throw InstallationFailure.runningApplication }
            let source = Bundle.main.bundleURL
            let target = destination
            // ditto and signature verification block; keep the install window responsive.
            try await Task.detached(priority: .userInitiated) {
                try AppInstallation.install(source: source, destination: target)
            }.value
            phase = .installed
        } catch {
            installationFailure = error as? InstallationFailure
            failure = error.localizedDescription
            phase = .failed
        }
    }

    private func openInstalledApplication() {
        phase = .opening
        failure = nil
        let configuration = NSWorkspace.OpenConfiguration()
        // The installer has the same bundle ID; do not reactivate the disk-image copy.
        configuration.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: destination, configuration: configuration) { _, error in
            Task { @MainActor in
                if let error {
                    self.failure = error.localizedDescription
                    self.phase = .installed
                } else {
                    self.phase = .installed
                    NSApp.terminate(nil)
                }
            }
        }
    }
}

struct InstallationView: View {
    @Bindable var model: InstallationModel
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Image(nsImage: model.icon)
                .resizable().interpolation(.high)
                .frame(width: 72, height: 72)
                .accessibilityHidden(true)
                .padding(.bottom, 36)
            Text(model.title)
                .font(DFInstallerStyle.title)
                .foregroundStyle(.primary)
                .padding(.bottom, 16)
            if model.phase == .failed {
                ScrollView {
                    subtitle.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 150)
            } else {
                subtitle
            }
            Spacer(minLength: 24)
            if model.phase == .installing {
                ProgressView().progressViewStyle(.linear)
                    .tint(DFPalette.accent)
                    .padding(.bottom, 18)
            }
            if model.phase == .installed, let failure = model.failure {
                Text(failure).font(DFInstallerStyle.footer)
                    .foregroundStyle(DFPalette.danger)
                    .textSelection(.enabled)
                    .padding(.bottom, 12)
            }
            HStack(spacing: 16) {
                Button(action: model.performAction) {
                    Text(model.buttonTitle)
                        .frame(maxWidth: .infinity)
                        .frame(height: DFInstallerStyle.buttonHeight)
                        .contentShape(Rectangle())
                }
                .buttonStyle(InstallerButtonStyle(primary: true))
                .disabled(model.busy)
                .keyboardShortcut(.defaultAction)
                Button { model.english.toggle() } label: {
                    HStack(spacing: 8) {
                        Text(model.english ? "中文" : "English")
                        Image(systemName: "arrow.left.arrow.right")
                    }
                    .frame(width: 124, height: DFInstallerStyle.buttonHeight)
                    .contentShape(Rectangle())
                }
                .buttonStyle(InstallerButtonStyle(primary: false))
                .accessibilityLabel(model.text("切换为英文", "Switch to Chinese"))
            }
            HStack(spacing: 6) {
                Text(model.text("安装遇到问题？", "Having trouble?"))
                Link(model.text("立即反馈", "Get help"),
                     destination: URL(string: "https://github.com/GodD6366/deskfuse-release/issues")!)
            }
            .font(DFInstallerStyle.footer)
            .foregroundStyle(DFPalette.secondaryText(scheme))
            .padding(.top, 12)
        }
        .padding(.horizontal, DFInstallerStyle.inset)
        .padding(.top, 76)
        .padding(.bottom, 28)
        .frame(width: DFInstallerStyle.layoutSize.width, height: DFInstallerStyle.layoutSize.height)
        .background(DFPalette.surface(scheme))
        .scaleEffect(DFInstallerStyle.scale)
        .frame(width: DFInstallerStyle.size.width, height: DFInstallerStyle.size.height)
    }

    private var subtitle: some View {
        Text(model.subtitle)
            .font(DFInstallerStyle.body)
            .foregroundStyle(DFPalette.secondaryText(scheme))
            .lineSpacing(5)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
            .accessibilityAddTraits(.updatesFrequently)
    }
}

private struct InstallerButtonStyle: ButtonStyle {
    let primary: Bool
    @Environment(\.isEnabled) private var enabled
    @Environment(\.colorScheme) private var scheme

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(DFInstallerStyle.button)
            .foregroundStyle(primary ? Color.white : Color.primary)
            .background(primary ? DFPalette.accent : DFPalette.surface(scheme))
            .clipShape(RoundedRectangle(cornerRadius: DFInstallerStyle.radius))
            .overlay {
                if !primary {
                    RoundedRectangle(cornerRadius: DFInstallerStyle.radius)
                        .strokeBorder(DFPalette.line(scheme), lineWidth: 1)
                }
            }
            .opacity(enabled ? (configuration.isPressed ? 0.78 : 1) : 0.6)
    }
}

@MainActor
final class InstallationWindowController: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let model: InstallationModel
    private(set) var window: NSWindow?

    init(model: InstallationModel = InstallationModel()) { self.model = model }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let menu = NSMenu()
        let applicationItem = NSMenuItem()
        let applicationMenu = NSMenu()
        applicationMenu.addItem(withTitle: model.text("退出安装", "Quit Installer"),
                                action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        applicationItem.submenu = applicationMenu
        menu.addItem(applicationItem)
        NSApp.mainMenu = menu
        showWindow()
    }

    func showWindow() {
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: DFInstallerStyle.size),
            styleMask: [.titled, .closable, .miniaturizable, .fullSizeContentView],
            backing: .buffered, defer: false
        )
        window.title = model.text("安装 DeskFuse", "Install DeskFuse")
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.titlebarSeparatorStyle = .none
        window.isReleasedWhenClosed = false
        window.delegate = self
        let content = NSHostingView(rootView: InstallationView(model: model).ignoresSafeArea())
        content.sizingOptions = []
        window.contentView = content
        // The full-size content already includes the transparent title bar.
        window.setFrame(NSRect(origin: window.frame.origin, size: DFInstallerStyle.size), display: false)
        window.center()
        self.window = window
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool { !model.busy }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        model.busy ? .terminateCancel : .terminateNow
    }
}
