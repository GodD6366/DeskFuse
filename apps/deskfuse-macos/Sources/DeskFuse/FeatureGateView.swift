import AppKit
import SwiftUI

/// 可选功能未启用时的页内引导空状态：说明价值、列出缺失权限、一键启用。
/// 视觉对齐被控空态，标题与动作沿用工作区的公共规格，
/// 用玻璃圆形徽标区分"功能未启用"与"等待对端"两种语义。
struct FeatureGateView: View {
    let symbol: String
    /// 引导图标的语义色调；未启用功能用中性强调色，恢复入口用品牌色。
    var tone: DFTone = .accent
    let title: String
    let message: String
    let bullets: [String]
    /// 关联的系统权限；未授权时显示可点击的授权行。
    let permission: LaunchPermission?
    let actionTitle: String
    let action: () -> Void
    var secondaryTitle: String? = nil
    var secondaryAction: (() -> Void)? = nil

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var appeared = false
    @State private var permissionGranted = false
    /// 启用动作已提交：按钮短暂呈现完成态，直到页面切换到真实功能视图。
    @State private var confirming = false
    private let didBecomeActive = NotificationCenter.default
        .publisher(for: NSApplication.didBecomeActiveNotification)

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: symbol)
                .font(DFType.title)
                .foregroundStyle(tone.color)
                .frame(width: 56, height: 56)
                .modifier(NativeGlassSurface(cornerRadius: DFRadius.panel))
                .accessibilityHidden(true)
            Text(title)
                .font(DFType.title)
            Text(message)
                .font(DFType.detail)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 8) {
                ForEach(bullets, id: \.self) { bullet in
                    Label(bullet, systemImage: "checkmark")
                        .font(DFType.detail)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .center)
            if let permission, !permissionGranted {
                permissionRow(permission)
            }
            Button {
                confirming = true
                action()
            } label: {
                if confirming {
                    Label("已启用", systemImage: "checkmark")
                } else {
                    Text(actionTitle)
                }
            }
            .buttonStyle(.dfPrimary)
            .disabled(confirming)
            if let secondaryTitle, let secondaryAction {
                Button(secondaryTitle, action: secondaryAction)
                    .buttonStyle(.link)
            }
        }
        .frame(maxWidth: 520)
        .opacity(appeared ? 1 : 0)
        .offset(y: appeared || reduceMotion ? 0 : 8)
        .onAppear {
            refreshPermission()
            withAnimation(.spring(response: 0.5, dampingFraction: 0.8)) {
                appeared = true
            }
        }
        .onReceive(didBecomeActive) { _ in refreshPermission() }
    }

    private func permissionRow(_ permission: LaunchPermission) -> some View {
        HStack(spacing: 12) {
            Image(systemName: permission.symbol)
                .font(DFType.title)
                .foregroundStyle(.secondary)
                .frame(width: 24)
                .accessibilityHidden(true)
            Text(permission.title)
                .font(DFType.detail)
            Spacer(minLength: 12)
            Button {
                AppStore.shared.permissionPrompt.authorize(permission)
            } label: {
                Label("前往授权", systemImage: "arrow.up.forward.app")
                    .font(DFType.detail)
            }
            .buttonStyle(.dfSecondary)
            .controlSize(.small)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: 340)
        .background(
            Color(nsColor: .controlBackgroundColor).opacity(0.52),
            in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        )
    }

    private func refreshPermission() {
        guard let permission else { return }
        permissionGranted = AppStore.shared.permissionPrompt.isGranted(permission)
    }
}
