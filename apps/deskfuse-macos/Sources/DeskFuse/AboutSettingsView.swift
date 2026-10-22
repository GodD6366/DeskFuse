import AppKit
import SwiftUI
import DeskFuseEngine

/// 关于与更新：版本、更新和帮助反馈集中在一页。
struct AboutSettingsView: View {
    let store: AppStore
    let update: UpdateState?

    var body: some View {
        DFScrollPage {
            HStack(spacing: DFSpace.l) {
                DeskFuseBrandMark(size: 48)
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text("DeskFuse").font(DFType.pageTitle)
                    Text("一套键鼠、一个剪贴板，在多台电脑之间无缝工作。")
                        .font(DFType.detail).dfSecondaryText()
                }
            }
            DFSection("更新", symbol: "arrow.down.circle",
                      footer: UpdateInstallationChannel.isManual ? nil : "DeskFuse 每 12 小时自动检查一次更新。") {
                if let update {
                    UpdateControls(store: store, update: update)
                } else {
                    Text("正在读取版本信息…").font(DFType.detail).dfSecondaryText()
                }
            }
            DFSection("帮助与反馈", symbol: "lifepreserver",
                      footer: "诊断文件只包含运行日志和设备状态，不包含剪贴板内容。随时可以在菜单「帮助」或侧栏底部的问号找到这些入口。") {
                DFSettingRow(title: "反馈问题",
                             detail: "自动导出诊断并打开反馈页，版本和系统信息已经填好，只需描述发生了什么。") {
                    Button("反馈问题…") { SupportCenter.reportProblem(store) }
                        .buttonStyle(.dfPrimary)
                }
                Divider()
                DFSettingRow(title: "导出诊断信息", detail: "保存到指定位置，方便通过其他渠道发给开发者。") {
                    DiagnosticsExportButton(store: store)
                }
                Divider()
                DFSettingRow(title: "已知问题与发行说明", detail: "先看看别人是否遇到过同样的问题，或新版本是否已修复。") {
                    HStack(spacing: DFSpace.s) {
                        Button("已报告的问题") { SupportCenter.open(SupportCenter.issuesURL) }
                        Button("发行说明") { SupportCenter.open(SupportCenter.releasesURL) }
                    }
                    .buttonStyle(.dfSecondary)
                }
            }
        }
    }
}

/// 让用户自选位置导出诊断；完成后在访达中选中文件。
struct DiagnosticsExportButton: View {
    let store: AppStore
    @State private var exporting = false

    var body: some View {
        Button {
            guard !exporting else { return }
            exporting = true
            SupportCenter.exportDiagnosticsInteractively(store) { exporting = false }
        } label: {
            HStack(spacing: DFSpace.xs) {
                if exporting { ProgressView().controlSize(.small) }
                Text(exporting ? "正在导出…" : "导出…")
            }
        }
        .buttonStyle(.dfSecondary)
        .disabled(exporting || store.client == nil)
    }
}
