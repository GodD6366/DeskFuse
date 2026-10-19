import AppKit
import Observation
import SwiftUI
import DeskFuseEngine

/// 全新安装的首次启动引导：欢迎 → 连接另一台电脑 → 选择功能。
/// 可跳过、可返回；权限只在功能真正需要时当场申请。
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

struct OnboardingFlow: View {
    @Bindable var store: AppStore
    @State private var step: Step = .welcome
    @State private var appeared = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    enum Step: Int, CaseIterable {
        case welcome, connect, features

        var title: String {
            switch self {
            case .welcome: "欢迎使用 DeskFuse"
            case .connect: "连接另一台电脑"
            case .features: "选择要用的功能"
            }
        }
    }

    var body: some View {
        VStack(spacing: DFSpace.xl) {
            header
            Group {
                switch step {
                case .welcome: welcomeStep
                case .connect: connectStep
                case .features: featureStep
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            footer
        }
        .padding(DFSpace.xxl)
        .frame(width: 560)
        .onAppear {
            withAnimation(DFMotion.settle) { appeared = true }
        }
    }

    private var header: some View {
        VStack(spacing: DFSpace.s) {
            Image(systemName: step == .welcome ? "doc.on.clipboard" : step == .connect ? "laptopcomputer.and.iphone" : "switch.2")
                .font(DFType.icon(.hero, weight: .light))
                .foregroundStyle(DFPalette.accent)
                .accessibilityHidden(true)
            Text(step.title)
                .font(DFType.pageTitle)
            HStack(spacing: DFSpace.xs) {
                ForEach(Step.allCases, id: \.rawValue) { item in
                    Capsule()
                        .fill(item == step ? DFPalette.accent : DFPalette.line(.light))
                        .frame(width: item == step ? 18 : 6, height: 4)
                }
            }
            .animation(DFMotion.snappy, value: step)
            .accessibilityLabel("第 \(step.rawValue + 1) 步，共 \(Step.allCases.count) 步")
        }
    }

    // MARK: - 步骤一：欢迎

    private var welcomeStep: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            Text("一套键鼠、一个剪贴板，在多台 Mac 之间无缝工作。")
                .font(DFType.body)
            DFValueRow(title: "这台电脑", value: store.snapshot?.localName ?? "这台 Mac", selectable: true)
            Text("接下来先连接另一台电脑，再选择要用的功能。")
                .font(DFType.detail)
                .dfSecondaryText()
        }
    }

    // MARK: - 步骤二：连接另一台电脑

    private var connectStep: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            Text("两台电脑都打开 DeskFuse，并在同一个网络里。选中后由对方确认。")
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
            let candidates = nearbyDevices
            if candidates.isEmpty {
                DFEmptyState(symbol: "wifi",
                             title: "还没有发现可添加的电脑",
                             message: "确认另一台电脑已打开 DeskFuse 且在同一网络，或手动输入它的地址。",
                             actionTitle: "刷新", action: { store.send(.refreshDevices) })
                    .frame(height: 190)
            } else {
                VStack(spacing: 0) {
                    ForEach(candidates, id: \.id) { device in
                        nearbyRow(device)
                        if device.id != candidates.last?.id { Divider() }
                    }
                }
            }
            manualPairingRow
            if !pairedDevices.isEmpty {
                Label("已连接 \(pairedDevices.map(\.name).joined(separator: "、"))", systemImage: "checkmark.circle.fill")
                    .font(DFType.detail)
                    .foregroundStyle(DFPalette.success)
            }
        }
    }

    private func nearbyRow(_ device: Device) -> some View {
        HStack(spacing: DFSpace.m) {
            Image(systemName: "laptopcomputer")
                .font(DFType.icon(.medium))
                .dfSecondaryText()
                .frame(width: 24)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(device.name).font(DFType.bodyMedium)
                Text(device.detail.isEmpty ? "同一网络" : device.detail)
                    .font(DFType.detail)
                    .dfSecondaryText()
            }
            Spacer(minLength: DFSpace.s)
            if store.pairingDecisions.contains(device.id) {
                HStack(spacing: DFSpace.xs) {
                    ProgressView().controlSize(.small)
                    Text("等待对方确认").font(DFType.detail).dfSecondaryText()
                }
            } else {
                Button("添加") { store.send(.pair(peerId: device.id), pairing: device.id) }
                    .buttonStyle(.dfSecondary)
            }
        }
        .padding(.vertical, DFSpace.xs)
    }

    @State private var manualAddress = ""
    @State private var showManualAddress = false

    private var manualPairingRow: some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            if showManualAddress {
                HStack(spacing: DFSpace.s) {
                    TextField("例如 192.168.1.20", text: $manualAddress)
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 220)
                        .onSubmit(pairByIP)
                    Button("配对", action: pairByIP)
                        .buttonStyle(.dfPrimary)
                        .disabled(manualAddress.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            } else {
                Button("找不到？手动输入地址") { showManualAddress = true }
                    .buttonStyle(.dfPlain)
            }
        }
    }

    private func pairByIP() {
        let value = manualAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        store.send(.pairAt(ip: value))
        manualAddress = ""
        showManualAddress = false
    }

    // MARK: - 步骤三：选择功能

    private var featureStep: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            DFToggleRow("剪贴板同步",
                        detail: "文字、图片和文件，复制即达另一台电脑。",
                        symbol: "doc.on.clipboard",
                        isOn: Binding(get: { store.snapshot?.clipboardEnabled ?? true },
                                      set: { store.send(.setClipboard(enabled: $0)) }))
            Divider()
            DFToggleRow("键鼠共享",
                        detail: "一套键鼠控制多台电脑，穿越屏幕边缘切换；需要辅助功能权限。",
                        symbol: "rectangle.on.rectangle",
                        isOn: Binding(get: { store.snapshot?.input.settings.enabled ?? false },
                                      set: { setInputSharing($0) }))
            if !store.permissions.isGranted(.accessibility), store.snapshot?.input.settings.enabled == true {
                PermissionRow(permission: .accessibility, compact: true)
            }
            Divider()
            DFToggleRow("文件传输",
                        detail: "拖入文件即可发送，也可扫码从手机取用。",
                        symbol: "arrow.up.arrow.down.square",
                        isOn: Binding(get: { true }, set: { _ in }))
                .disabled(true)
            if #available(macOS 14.2, *) {
                Divider()
                DFToggleRow("远程应用",
                            detail: "在这台 Mac 上打开另一台电脑的应用窗口。",
                            symbol: "square.grid.2x2", badge: "Beta",
                            isOn: Binding(get: { RemoteApplicationManager.shared.enabled },
                                          set: { RemoteApplicationManager.shared.enabled = $0 }))
            }
            Text("剪贴板同步和文件传输默认开启，之后可在各功能页随时调整。")
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// 开启键鼠共享时同时设置 controller，避免只打开开关后停在「等待主控」。
    private func setInputSharing(_ enabled: Bool) {
        guard var settings = store.snapshot?.input.settings else { return }
        settings.enabled = enabled
        if enabled, !settings.controller {
            settings.controller = true
        }
        store.send(.setInput(settings: settings))
        if enabled, !store.permissions.isGranted(.accessibility) {
            store.permissions.authorize(.accessibility)
        }
    }

    // MARK: - 底部导航

    private var footer: some View {
        HStack(spacing: DFSpace.m) {
            Button("跳过") { store.onboarding.complete() }
                .buttonStyle(.dfPlain)
            Spacer(minLength: DFSpace.s)
            if step != .welcome {
                Button("上一步") { advance(-1) }
                    .buttonStyle(.dfSecondary)
            }
            Button(step == .features ? "开始使用" : "下一步") { advance(1) }
                .buttonStyle(.dfPrimary)
                .keyboardShortcut(.defaultAction)
        }
    }

    private func advance(_ delta: Int) {
        let next = step.rawValue + delta
        guard let target = Step(rawValue: next) else {
            store.onboarding.complete()
            store.selectedPage = .devices
            return
        }
        withAnimation(DFMotion.settle) { step = target }
    }

    private var nearbyDevices: [Device] {
        (store.snapshot?.devices ?? [])
            .filter { !$0.trusted && !$0.blocked }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    private var pairedDevices: [Device] {
        (store.snapshot?.devices ?? []).filter { $0.trusted && $0.connected && !$0.blocked }
    }
}
