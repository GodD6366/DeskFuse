import AppKit
import SwiftUI
import DeskFuseEngine

/// 关于与更新：版本、更新和反馈问题集中在一页。
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
            DFSection("反馈问题", symbol: "lifepreserver",
                      footer: "诊断文件只包含运行日志和设备状态，不包含剪贴板内容。导出后可附在问题反馈中。") {
                DFSettingRow(title: "导出诊断信息", detail: "遇到连接、同步或键鼠问题时，导出给开发者分析。") {
                    DiagnosticsExportButton(store: store)
                }
            }
        }
    }
}

/// 导出引擎诊断 + 本机内存与鼠标按键诊断，写入同一个 JSONL 文件。
struct DiagnosticsExportButton: View {
    let store: AppStore
    @State private var exporting = false

    var body: some View {
        Button {
            export()
        } label: {
            HStack(spacing: DFSpace.xs) {
                if exporting { ProgressView().controlSize(.small) }
                Text(exporting ? "正在导出…" : "导出…")
            }
        }
        .buttonStyle(.dfSecondary)
        .disabled(exporting || store.client == nil)
    }

    private func export() {
        guard !exporting, let client = store.client else { return }
        exporting = true
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "DeskFuse-diagnostics.jsonl"
        // Do not enter a nested AppKit run loop from a SwiftUI action.
        panel.begin { response in
            guard response == .OK, let url = panel.url else {
                exporting = false
                return
            }
            Task { @MainActor in
                defer { exporting = false }
                do {
                    try await client.exportDiagnostics(url.path)
                    let memory = await MemoryDiagnostics.capture(
                        ocr: ClipboardOCRIndexer.shared.diagnostics(),
                        selectedPage: store.selectedPage.rawValue,
                        visibleWindowCount: NSApp.windows.filter { $0.occlusionState.contains(.visible) }.count)
                    let handle = try FileHandle(forWritingTo: url)
                    handle.seekToEndOfFile()
                    handle.write(memory)
                    handle.write(MouseButtonController.shared.diagnostics())
                    try handle.close()
                }
                catch { store.error = error.userFacingMessage }
            }
        }
    }
}
