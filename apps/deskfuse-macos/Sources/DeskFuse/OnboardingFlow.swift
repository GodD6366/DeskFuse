import AppKit
import Observation
import SwiftUI
import DeskFuseEngine

/// 全新安装或升级后尚未完成引导的首次启动：先逐项说明并申请系统权限，
/// 再确认默认开启的功能。中途退出后仍会继续，完成后不再重复。
@MainActor @Observable
final class OnboardingController {
    /// 当前引导版本的完成标记。旧版引导使用 v1，不能替代这一版的选择。
    static let completedKey = "deskfuse.onboarding.completed.v2"
    var isPresented = false

    /// 只看本机有没有完成当前引导；与状态库是否存在、应用是否早已安装无关。
    func shouldPresent() -> Bool {
        !UserDefaults.standard.bool(forKey: Self.completedKey)
    }

    func present() { isPresented = true }

    /// 调试用：清除完成标记并立即重走引导，中途退出后下次启动会继续，与全新安装一致。
    func restart() {
        UserDefaults.standard.removeObject(forKey: Self.completedKey)
        isPresented = true
    }

    func complete() {
        UserDefaults.standard.set(true, forKey: Self.completedKey)
        isPresented = false
    }
}

/// 首次安装时可选择的功能。这里的默认值是全新安装的开箱状态：
/// 剪贴板、键鼠共享与远程应用开启；鼠标按键与麦克风保持关闭。
/// 已安装过的本机进入引导时，会先用现有设置覆盖这些初值。
enum OnboardingFeatureKind: String, CaseIterable, Identifiable {
    case clipboard
    case input
    case remoteApplications
    case mouseButtons
    case microphone

    var id: String { rawValue }

    var title: String {
        switch self {
        case .clipboard: "剪贴板同步"
        case .input: "键鼠共享"
        case .remoteApplications: "远程应用"
        case .mouseButtons: "鼠标按键"
        case .microphone: "麦克风共享"
        }
    }

    var detail: String {
        switch self {
        case .clipboard:
            "在一台 Mac 复制，另一台直接粘贴。文字、图片和文件都会同步。"
        case .input:
            "一套键盘和鼠标穿过屏幕边缘，控制同一网络里的其他 Mac。需要辅助功能权限。"
        case .remoteApplications:
            "把另一台 Mac 上的应用窗口，直接打开在这台 Mac 上。需要屏幕录制权限。"
        case .mouseButtons:
            "把鼠标侧键映射成快捷键、窗口切换或系统操作。需要输入监控权限。"
        case .microphone:
            "把另一台 Mac 或 iPhone 的麦克风，作为这台 Mac 的输入设备。开启后还需选择音源。"
        }
    }

    var symbol: String {
        switch self {
        case .clipboard: "doc.on.clipboard"
        case .input: "keyboard"
        case .remoteApplications: "macwindow.on.rectangle"
        case .mouseButtons: "computermouse"
        case .microphone: "mic"
        }
    }

    var badge: String? {
        switch self {
        case .remoteApplications, .mouseButtons, .microphone: "Beta"
        case .clipboard, .input: nil
        }
    }

    var isDefaultOn: Bool {
        switch self {
        case .clipboard, .input, .remoteApplications: true
        case .mouseButtons, .microphone: false
        }
    }

    var isAvailable: Bool {
        guard self == .remoteApplications else { return true }
        if #available(macOS 14.2, *) { return true }
        return false
    }

    static var initialEnabled: [Self: Bool] {
        Dictionary(uniqueKeysWithValues: allCases.map { ($0, $0.isDefaultOn) })
    }
}

extension OnboardingFeatureKind {
    /// 与功能页引导共用同一套演示，首次引导和功能页看到的是同一段动画。
    @ViewBuilder var demo: some View {
        switch self {
        case .clipboard: ClipboardSyncDemo()
        case .input: InputCrossingDemo()
        case .remoteApplications: RemoteWindowDemo()
        case .mouseButtons: MouseSideButtonDemo()
        case .microphone: MicrophoneRelayDemo()
        }
    }
}

/// 功能列表行：点选后在右侧预览演示，开关决定完成设置后是否开启。
private struct OnboardingFeatureRow: View {
    let kind: OnboardingFeatureKind
    let selected: Bool
    @Binding var isOn: Bool
    let onSelect: () -> Void
    @State private var hovering = false

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        HStack(spacing: DFSpace.m) {
            Image(systemName: kind.symbol)
                .font(DFType.icon(.regular, weight: .medium))
                .foregroundStyle(isOn ? DFPalette.accent : Color.secondary)
                .frame(width: 18)
                .accessibilityHidden(true)
            Text(kind.title).font(DFType.bodyMedium)
            if let badge = kind.badge { DFBadge(badge) }
            Spacer(minLength: DFSpace.s)
            Toggle(kind.title, isOn: $isOn)
                .labelsHidden()
                .toggleStyle(OnboardingSwitchStyle())
                .accessibilityLabel(kind.title)
                .accessibilityValue(isOn ? "已开启" : "已关闭")
        }
        .padding(.horizontal, DFSpace.m)
        .frame(minHeight: 44)
        .background(selected ? DFPalette.selection : hovering ? DFPalette.hover : .clear, in: shape)
        .contentShape(shape)
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
        .animation(DFMotion.snappy, value: isOn)
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityAction(named: "查看演示", onSelect)
    }
}

/// 纯 SwiftUI 开关：引导页的卡片表面是自绘的，原生 NSSwitch 在离屏和
/// 非激活窗口里可能画出过期状态；自绘开关能保证画面与真实绑定一致。
private struct OnboardingSwitchStyle: ToggleStyle {
    @Environment(\.colorScheme) private var scheme

    func makeBody(configuration: Configuration) -> some View {
        Button {
            withMotion(DFMotion.snappy) { configuration.isOn.toggle() }
        } label: {
            Capsule(style: .continuous)
                .fill(configuration.isOn ? DFPalette.accent : DFPalette.control(scheme))
                .overlay {
                    Capsule(style: .continuous)
                        .strokeBorder(configuration.isOn ? .clear : DFPalette.line(scheme),
                                      lineWidth: DFMetrics.hairline)
                }
                .frame(width: 38, height: 22)
                .overlay {
                    Circle()
                        .fill(.white)
                        .frame(width: 18, height: 18)
                        .shadow(color: .black.opacity(0.18), radius: 1, y: 1)
                        .padding(2)
                        .frame(maxWidth: .infinity, alignment: configuration.isOn ? .trailing : .leading)
                }
                .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
    }
}

struct OnboardingFlow: View {
    @Bindable var store: AppStore

    @State private var step: Step
    @State private var enabled = OnboardingFeatureKind.initialEnabled
    /// 右侧预览的功能；同一时间只播放这一段演示，其余功能只显示为列表行。
    @State private var previewed: OnboardingFeatureKind = .clipboard
    /// 已从现有设置读取过的功能；用户手动改过的项不再被后续读取覆盖。
    @State private var seededChoices: Set<OnboardingFeatureKind> = []
    @State private var editedChoices: Set<OnboardingFeatureKind> = []
    @Environment(\.colorScheme) private var scheme
    private let requiredPermissions: [AppPermission] = [.accessibility, .screenRecording]

    init(store: AppStore, initialStep: Step = .permissions) {
        self.store = store
        _step = State(initialValue: initialStep)
    }

    enum Step: Int, CaseIterable, Identifiable {
        case permissions
        case features

        var id: Int { rawValue }

        var title: String {
            switch self {
            case .permissions: "系统权限"
            case .features: "功能开关"
            }
        }

        var detail: String {
            switch self {
            case .permissions: "允许必要的系统访问"
            case .features: "确认默认开启的功能"
            }
        }

        var symbol: String {
            switch self {
            case .permissions: "lock.shield"
            case .features: "switch.2"
            }
        }

        var headline: String {
            switch self {
            case .permissions: "先让 DeskFuse 获得必要的访问权限"
            case .features: "选择开机后默认开启的功能"
            }
        }

        var subhead: String {
            switch self {
            case .permissions:
                "这些权限由 macOS 管理，只在本机使用；随时可以在系统设置或应用中更改。"
            case .features:
                "常用功能已经为你打开，进阶功能保持关闭；完成设置后也能随时调整。"
            }
        }
    }

    var body: some View {
        HStack(spacing: 0) {
            sidebar
            VStack(spacing: 0) {
                contentHeader
                ScrollView {
                    VStack(alignment: .leading, spacing: DFSpace.xl) {
                        pageIntro
                        Group {
                            switch step {
                            case .permissions: permissionsStep
                            case .features: featuresStep
                            }
                        }
                        .transition(.opacity)
                        .id(step)
                        PermissionRelaunchNotice()
                    }
                    .frame(maxWidth: DFMetrics.focusMaxWidth, alignment: .leading)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.horizontal, DFMetrics.pageInset)
                    .padding(.top, DFSpace.l)
                    .padding(.bottom, DFSpace.xl)
                }
                footer
            }
        }
        .font(DFType.body)
        .controlSize(.small)
        .tint(DFPalette.accent)
        .background(DFPalette.canvas(scheme))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .ignoresSafeArea(.container, edges: .top)
        .allowsSystemRelaunch()
        .onAppear { seedChoicesFromCurrentSettings() }
        .onChange(of: store.snapshot?.revision, initial: true) { _, _ in
            seedChoicesFromCurrentSettings()
        }
        .onChange(of: MouseButtonController.shared.hasLoadedSettings) { _, _ in
            seedChoicesFromCurrentSettings()
        }
        .onChange(of: MicrophoneSharing.shared.snapshot?.session_id) { _, _ in
            seedChoicesFromCurrentSettings()
        }
    }

    // MARK: - 左侧步骤栏

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: DFSpace.s) {
                DeskFuseBrandMark()
                Text("DeskFuse").font(DFType.title)
                Spacer(minLength: 0)
            }
            .padding(.leading, DFMetrics.trafficLightsInset)
            .padding(.trailing, DFSpace.l)
            .frame(height: DFMetrics.titlebarHeight)

            VStack(alignment: .leading, spacing: DFSpace.l) {
                Text("初次设置")
                    .font(DFType.caption.weight(.semibold))
                    .dfSecondaryText()
                ForEach(Step.allCases) { item in
                    stepRow(item)
                }
            }
            .padding(.horizontal, DFMetrics.pageInset)
            .padding(.top, DFSpace.xl)

            Spacer(minLength: DFSpace.xl)
            privacyNote
                .padding(DFMetrics.pageInset)
        }
        .frame(width: 264)
        .frame(maxHeight: .infinity, alignment: .topLeading)
        .background(DFPalette.canvas(scheme))
        .overlay(alignment: .trailing) {
            Rectangle()
                .fill(DFPalette.line(scheme))
                .frame(width: DFMetrics.hairline)
        }
    }

    private func stepRow(_ item: Step) -> some View {
        let active = item == step
        let permissionComplete = requiredPermissions.allSatisfy { store.permissions.isGranted($0) }
        let completed = item.rawValue < step.rawValue
            && (item != .permissions || permissionComplete)
        let skipped = item.rawValue < step.rawValue && !completed
        return HStack(alignment: .top, spacing: DFSpace.m) {
            ZStack {
                Circle()
                    .fill(active ? DFPalette.accent
                          : completed ? DFPalette.success.opacity(DFOpacity.soft)
                          : DFPalette.control(scheme))
                    .frame(width: 24, height: 24)
                if completed {
                    Image(systemName: "checkmark")
                        .font(DFType.caption.weight(.semibold))
                        .foregroundStyle(DFPalette.success)
                } else {
                    Text("\(item.rawValue + 1)")
                        .font(DFType.caption.weight(.semibold))
                        .foregroundStyle(active ? .white : Color.secondary)
                }
            }
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(item.title)
                    .font(DFType.bodyMedium)
                    .foregroundStyle(active || completed ? Color.primary : Color.secondary)
                Text(item.detail)
                    .font(DFType.detail)
                    .dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel("第 \(item.rawValue + 1) 步，\(item.title)，\(active ? "进行中" : completed ? "已完成" : skipped ? "已浏览" : "未开始")")
    }

    private var privacyNote: some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            Image(systemName: "lock.shield")
                .font(DFType.icon(.medium))
                .foregroundStyle(DFPalette.accent)
                .accessibilityHidden(true)
            Text("隐私优先").font(DFType.bodyMedium)
            Text("权限只在本机使用。DeskFuse 不会上传你的剪贴板内容、屏幕画面或音频。")
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(DFSpace.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(DFPalette.control(scheme), in: RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
        .accessibilityElement(children: .combine)
    }

    // MARK: - 内容框架

    private var contentHeader: some View {
        HStack(spacing: DFSpace.s) {
            Text("初次设置").dfSecondaryText()
            Text("/").dfSecondaryText().accessibilityHidden(true)
            Text(step.title).font(DFType.bodyMedium)
            Spacer(minLength: DFSpace.s)
            Text("第 \(step.rawValue + 1) 步，共 \(Step.allCases.count) 步")
                .font(DFType.detail)
                .dfSecondaryText()
        }
        .padding(.horizontal, DFMetrics.pageInset)
        .frame(height: DFMetrics.titlebarHeight)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("初次设置，\(step.title)，第 \(step.rawValue + 1) 步，共 \(Step.allCases.count) 步")
    }

    private var pageIntro: some View {
        HStack(alignment: .top, spacing: DFSpace.l) {
            ZStack {
                RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous)
                    .fill(DFPalette.accent.opacity(DFOpacity.soft))
                Image(systemName: step.symbol)
                    .font(DFType.icon(.large))
                    .foregroundStyle(DFPalette.accent)
            }
            .frame(width: 56, height: 56)
            .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                Text(step.headline).font(DFType.pageTitle)
                Text(step.subhead)
                    .font(DFType.body)
                    .dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .contain)
    }

    private var footer: some View {
        HStack(spacing: DFSpace.m) {
            if step != .permissions {
                Button("上一步") { advance(-1) }
                    .buttonStyle(.dfSecondary)
            }
            Spacer(minLength: DFSpace.s)
            Text(footerHint)
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
            Button(step == .permissions ? "下一步：选择功能" : "开始使用") { advance(1) }
                .buttonStyle(.dfPrimary)
                .keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, DFMetrics.pageInset)
        .frame(minHeight: 64)
        .background(DFPalette.canvas(scheme))
        .overlay(alignment: .top) {
            Rectangle()
                .fill(DFPalette.line(scheme))
                .frame(height: DFMetrics.hairline)
        }
    }

    private var footerHint: String {
        switch step {
        case .permissions:
            "可以稍后授权，功能页会再次提示"
        case .features:
            enabledCount == 0 ? "所有功能保持关闭，随时可以再打开" : "将开启 \(enabledCount) 项功能，随时可以调整"
        }
    }

    // MARK: - 步骤一：权限

    private var permissionsStep: some View {
        VStack(alignment: .leading, spacing: DFSpace.xl) {
            permissionProgress

            DFSection("必要权限", symbol: "checkmark.shield",
                      footer: "键鼠共享需要辅助功能；远程应用需要屏幕录制。屏幕录制可能要在重新打开 DeskFuse 后生效。") {
                permissionRows([.accessibility, .screenRecording])
            }

            DFSection("进阶权限", symbol: "slider.horizontal.3",
                      footer: "这些权限只在对应功能开启时才会用到，可以稍后再授权。") {
                permissionRows([.inputMonitoring, .wifi, .fullDiskAccess])
            }

            DFNotice(tone: .accent,
                     title: "逐项授权，随时可改",
                     message: "点击右侧按钮后，macOS 会带你到对应的系统开关。没有授权的功能不会在后台工作。")
        }
    }

    private var permissionProgress: some View {
        let granted = requiredPermissions.filter { store.permissions.isGranted($0) }.count
        return HStack(spacing: DFSpace.m) {
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text("必要权限已授权 \(granted) / \(requiredPermissions.count)")
                    .font(DFType.bodyMedium)
                    .monospacedDigit()
                Text(granted == requiredPermissions.count ? "已经可以开始使用全部核心功能。" : "缺少权限时，对应功能会提示去授权。")
                    .font(DFType.detail)
                    .dfSecondaryText()
            }
            Spacer(minLength: DFSpace.s)
            ProgressView(value: Double(granted), total: Double(requiredPermissions.count))
                .progressViewStyle(.linear)
                .tint(DFPalette.accent)
                .frame(width: 160)
        }
        .padding(DFSpace.m)
        .background(DFPalette.control(scheme), in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("必要权限已授权 \(granted) 项，共 \(requiredPermissions.count) 项")
    }

    private func permissionRows(_ permissions: [AppPermission]) -> some View {
        VStack(spacing: 0) {
            ForEach(permissions) { permission in
                PermissionRow(permission: permission)
                if permission != permissions.last { Divider() }
            }
        }
    }

    // MARK: - 步骤二：功能

    private var featuresStep: some View {
        VStack(alignment: .leading, spacing: DFSpace.xl) {
            HStack(alignment: .top, spacing: DFSpace.l) {
                VStack(alignment: .leading, spacing: DFSpace.l) {
                    featureGroup(title: "默认开启", symbol: "checkmark.circle",
                                 kinds: OnboardingFeatureKind.allCases.filter { $0.isAvailable && $0.isDefaultOn })
                    featureGroup(title: "按需开启", symbol: "plus.circle",
                                 kinds: OnboardingFeatureKind.allCases.filter { $0.isAvailable && !$0.isDefaultOn })
                }
                .frame(width: 280)
                featurePreview
            }

            if !missingPermissions.isEmpty {
                DFNotice(tone: .warning,
                         title: "所选功能还需要系统权限",
                         message: "\(missingPermissions.map(\.title).joined(separator: "、"))尚未授权。可以先继续，功能页会再次提示。",
                         actionTitle: "去授权",
                         action: { advance(-1) })
            }

            Text("完成设置后，可在各功能页右上角的总开关或设置中随时修改。")
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func featureGroup(title: String, symbol: String, kinds: [OnboardingFeatureKind]) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            DFSectionLabel(title, symbol: symbol)
            VStack(spacing: DFSpace.xxs) {
                ForEach(kinds) { kind in
                    OnboardingFeatureRow(kind: kind, selected: kind == previewed,
                                         isOn: binding(for: kind),
                                         onSelect: { withMotion(DFMotion.snappy) { previewed = kind } })
                }
            }
            .padding(DFSpace.xs)
            .dfCard()
        }
    }

    /// 选中功能的演示与说明；关闭的功能压暗演示，打开后恢复高亮。
    private var featurePreview: some View {
        let kind = previewed
        let isOn = enabled[kind] ?? kind.isDefaultOn
        return VStack(alignment: .leading, spacing: DFSpace.m) {
            ZStack {
                RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                    .fill(DFPalette.control(scheme))
                kind.demo
                    .opacity(isOn ? 1 : 0.6)
            }
            .frame(maxWidth: .infinity)
            .frame(height: 168)

            VStack(alignment: .leading, spacing: DFSpace.xs) {
                HStack(spacing: DFSpace.s) {
                    Text(kind.title).font(DFType.title)
                    if let badge = kind.badge { DFBadge(badge) }
                    Spacer(minLength: DFSpace.s)
                    Text(isOn ? "完成后开启" : "保持关闭")
                        .font(DFType.detail)
                        .foregroundStyle(isOn ? DFPalette.accent : Color.secondary)
                }
                Text(kind.detail)
                    .font(DFType.detail)
                    .dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(DFMetrics.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .dfCard()
        .id(kind)
        .transition(.opacity)
        .animation(DFMotion.snappy, value: isOn)
        .accessibilityElement(children: .contain)
    }

    private func binding(for kind: OnboardingFeatureKind) -> Binding<Bool> {
        Binding(
            get: { enabled[kind] ?? kind.isDefaultOn },
            set: { value in
                editedChoices.insert(kind)
                withMotion(DFMotion.snappy) {
                    enabled[kind] = value
                    previewed = kind
                }
            }
        )
    }

    /// 已安装过的本机也运行引导时，开关先反映现有设置；
    /// 全新安装的实际值与要求的默认值一致，因此不需要额外分支。
    private func seedChoicesFromCurrentSettings() {
        guard let snapshot = store.snapshot else { return }
        seedChoice(.clipboard, snapshot.clipboardEnabled)
        seedChoice(.input, snapshot.input.settings.featureEnabled)
        if #available(macOS 14.2, *), OnboardingFeatureKind.remoteApplications.isAvailable {
            seedChoice(.remoteApplications, RemoteApplicationManager.shared.enabled)
        }
        if MouseButtonController.shared.hasLoadedSettings {
            seedChoice(.mouseButtons, MouseButtonController.shared.isFeatureEnabled)
        }
        if let sharing = MicrophoneSharing.shared.snapshot {
            seedChoice(.microphone, sharing.settings.enabled)
        }
    }

    private func seedChoice(_ kind: OnboardingFeatureKind, _ value: Bool) {
        guard !seededChoices.contains(kind), !editedChoices.contains(kind) else { return }
        enabled[kind] = value
        seededChoices.insert(kind)
    }

    private var enabledCount: Int {
        OnboardingFeatureKind.allCases.filter { $0.isAvailable && enabled[$0] == true }.count
    }

    /// 只提示会阻断所选功能的权限；已授权或功能未选时不打扰。
    private var missingPermissions: [AppPermission] {
        var result: [AppPermission] = []
        if enabled[.input] == true, !store.permissions.isGranted(.accessibility) {
            result.append(.accessibility)
        }
        if enabled[.remoteApplications] == true, OnboardingFeatureKind.remoteApplications.isAvailable,
           !store.permissions.isGranted(.screenRecording) {
            result.append(.screenRecording)
        }
        if enabled[.mouseButtons] == true, !store.permissions.isGranted(.inputMonitoring) {
            result.append(.inputMonitoring)
        }
        return result
    }

    // MARK: - 提交

    private func advance(_ delta: Int) {
        if delta > 0, step == .features {
            finish()
            return
        }
        guard let target = Step(rawValue: step.rawValue + delta) else { return }
        withMotion(DFMotion.settle) { step = target }
    }

    /// 引导里的选择在完成时一次性落盘。默认值与引擎默认一致，
    /// 但键鼠共享需要同时打开总开关、共享开关和主控角色。
    private func finish() {
        applyFeatureChoices()
        store.onboarding.complete()
        store.selectedPage = .devices
    }

    private func applyFeatureChoices() {
        defer { store.permissions.refresh() }
        if let snapshot = store.snapshot {
            let wantsClipboard = enabled[.clipboard] ?? true
            if snapshot.clipboardEnabled != wantsClipboard {
                store.send(.setClipboard(enabled: wantsClipboard))
            }

            let wantsInput = enabled[.input] ?? true
            var input = snapshot.input.settings
            if input.featureEnabled != wantsInput
                || input.enabled != wantsInput
                || (wantsInput && !input.controller) {
                input.featureEnabled = wantsInput
                input.enabled = wantsInput
                if wantsInput { input.controller = true }
                store.send(.setInput(settings: input))
            }
        }

        if #available(macOS 14.2, *) {
            let wantsRemote = enabled[.remoteApplications] ?? true
            if RemoteApplicationManager.shared.enabled != wantsRemote {
                RemoteApplicationManager.shared.enabled = wantsRemote
            }
        }

        applyMouseButtons(enabled[.mouseButtons] ?? false)
        applyMicrophone(enabled[.microphone] ?? false)
    }

    private func applyMouseButtons(_ on: Bool) {
        let controller = MouseButtonController.shared
        if controller.hasLoadedSettings {
            guard controller.isFeatureEnabled != on else { return }
            controller.setFeatureEnabled(on)
            return
        }
        guard on else { return }
        // 设置通常已在引导期间加载完成；仍给一个短窗口，避免首次启动时丢失选择。
        Task { @MainActor in
            for _ in 0..<30 {
                try? await Task.sleep(for: .milliseconds(100))
                guard controller.hasLoadedSettings else { continue }
                controller.setFeatureEnabled(true)
                return
            }
        }
    }

    private func applyMicrophone(_ on: Bool) {
        let sharing = MicrophoneSharing.shared
        if sharing.snapshot != nil {
            guard sharing.isFeatureEnabled != on else { return }
            sharing.setFeatureEnabled(on)
            return
        }
        guard on else { return }
        Task { @MainActor in
            for _ in 0..<40 {
                try? await Task.sleep(for: .milliseconds(100))
                guard sharing.snapshot != nil else { continue }
                sharing.setFeatureEnabled(true)
                return
            }
        }
    }
}
