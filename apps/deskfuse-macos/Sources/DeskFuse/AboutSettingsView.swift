import SwiftUI

struct AboutSettingsView: View {
    private var version: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let value = info["CFBundleShortVersionString"] as? String ?? "—"
        let build = info["CFBundleVersion"] as? String
        return build.map { "v\(value) · build \($0)" } ?? "v\(value)"
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack(spacing: 14) {
                    Image(systemName: "rectangle.on.rectangle")
                        .font(.system(size: 34, weight: .light))
                        .foregroundStyle(DeskFuseBrand.accent)
                        .frame(width: 52, height: 52)
                        .modifier(NativeGlassBackdrop(cornerRadius: 14))
                    VStack(alignment: .leading, spacing: 4) {
                        Text("DeskFuse").font(.title2.weight(.semibold))
                        Text("跨设备工作区").font(.callout).foregroundStyle(.secondary)
                    }
                }

                SettingsGroupCard("版本") {
                    LabeledContent("当前版本", value: version)
                    LabeledContent("更新通道", value: UpdateInstallationChannel.isManual ? "手动安装" : "可信局域网")
                }

                SettingsGroupCard("关于 DeskFuse") {
                    Text("用一套键鼠在多台设备之间移动，保持剪贴板和文件工作流连续。")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text("版本信息与更新入口集中在这里，侧栏底部只保留运行状态。")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.bottom, SystemSettingsMetrics.formPageBottomInset)
        }
    }
}
