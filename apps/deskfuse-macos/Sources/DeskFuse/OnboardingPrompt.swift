import AppKit
import Observation
import SwiftUI

/// 全新安装的首次启动引导：功能选择 + 权限清单，一次展示后不再出现。
@MainActor @Observable
final class OnboardingController {
    static let completedKey = "deskfuse.onboarding.completed.v1"
    var isPresented = false

    func present() { isPresented = true }

    func complete() {
        UserDefaults.standard.set(true, forKey: Self.completedKey)
        isPresented = false
    }
}

/// 欢迎页 sheet：三行功能开关与最终存储同源实时绑定，权限区只列缺失项。
struct OnboardingPrompt: View {
    @Bindable var store: AppStore
    @State private var missing: [LaunchPermission] = []
    @State private var appeared = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private let didBecomeActive = NotificationCenter.default
        .publisher(for: NSApplication.didBecomeActiveNotification)

    var body: some View {
        VStack(spacing: 20) {
            header
                .sectionEntrance(appeared: appeared, order: 0, reduceMotion: reduceMotion)
            featureSection
                .sectionEntrance(appeared: appeared, order: 1, reduceMotion: reduceMotion)
            permissionSection
                .sectionEntrance(appeared: appeared, order: 2, reduceMotion: reduceMotion)
            Button("开始使用") { store.onboarding.complete() }
                .modifier(NativeGlassAction())
                .controlSize(.large)
                .keyboardShortcut(.defaultAction)
                .sectionEntrance(appeared: appeared, order: 3, reduceMotion: reduceMotion)
        }
        .padding(24)
        .frame(width: 560)
        .onAppear {
            refreshPermissions()
            withAnimation(.spring(response: 0.5, dampingFraction: 0.8)) {
                appeared = true
            }
        }
        .onReceive(didBecomeActive) { _ in refreshPermissions() }
    }

    private var header: some View {
        VStack(spacing: 8) {
            Image(systemName: "doc.on.clipboard")
                .font(.system(size: 40))
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            Text("欢迎使用 DeskFuse")
                .font(.title2.weight(.semibold))
            Text("在所有设备间，自由流转")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }

    private var featureSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            sectionLabel("选择要用的功能")
            VStack(spacing: 0) {
                featureRow(
                    symbol: "doc.on.clipboard", color: .blue,
                    title: "剪贴板同步",
                    detail: "文字、图片和文件，复制即达另一台设备",
                    isOn: Binding(
                        get: { store.snapshot?.clipboardEnabled ?? true },
                        set: { store.send(.setClipboard(enabled: $0)) }
                    )
                )
                Divider().padding(.leading, 64)
                featureRow(
                    symbol: "rectangle.on.rectangle", color: .purple,
                    title: "键鼠共享",
                    detail: "一套键鼠控制多台电脑，穿越屏幕边缘切换",
                    isOn: Binding(
                        get: { store.snapshot?.input.settings.enabled ?? false },
                        set: { enabled in
                            guard var settings = store.snapshot?.input.settings else { return }
                            settings.enabled = enabled
                            store.send(.setInput(settings: settings))
                        }
                    )
                )
                if #available(macOS 14.2, *) {
                    Divider().padding(.leading, 64)
                    featureRow(
                        symbol: "square.grid.2x2", color: .teal,
                        title: "远程应用",
                        detail: "在这台 Mac 上打开另一台电脑的应用窗口",
                        isOn: Binding(
                            get: { RemoteApplicationManager.shared.enabled },
                            set: { RemoteApplicationManager.shared.enabled = $0 }
                        )
                    )
                }
            }
            .background(
                Color(nsColor: .controlBackgroundColor).opacity(0.52),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
            )
            Text("之后可在「设置 → 通用」随时调整")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var permissionSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            sectionLabel("系统权限")
            if missing.isEmpty {
                Label("权限已就绪", systemImage: "checkmark.circle.fill")
                    .font(.callout)
                    .foregroundStyle(.green)
            } else {
                VStack(spacing: 12) {
                    ForEach(missing) { permission in
                        PermissionPromptRow(permission: permission, controller: store.permissionPrompt)
                    }
                }
            }
        }
    }

    private func sectionLabel(_ title: String) -> some View {
        Text(title)
            .font(.footnote)
            .foregroundStyle(.secondary)
    }

    private func featureRow(symbol: String, color: Color, title: String, detail: String,
                            isOn: Binding<Bool>) -> some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 17))
                .foregroundStyle(color)
                .frame(width: 40, height: 40)
                .background(
                    color.opacity(0.15),
                    in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                )
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.body.weight(.medium))
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Toggle(title, isOn: isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private func refreshPermissions() {
        missing = LaunchPermission.allCases.filter { !store.permissionPrompt.isGranted($0) }
    }
}

/// 分区入场：依次 8pt 上浮淡入，只播一次；Reduce Motion 下只留淡入。
private struct SectionEntrance: ViewModifier {
    let appeared: Bool
    let order: Int
    let reduceMotion: Bool

    func body(content: Content) -> some View {
        content
            .opacity(appeared ? 1 : 0)
            .offset(y: appeared || reduceMotion ? 0 : 8)
            .animation(
                .spring(response: 0.5, dampingFraction: 0.8).delay(Double(order) * 0.08),
                value: appeared
            )
    }
}

private extension View {
    func sectionEntrance(appeared: Bool, order: Int, reduceMotion: Bool) -> some View {
        modifier(SectionEntrance(appeared: appeared, order: order, reduceMotion: reduceMotion))
    }
}
