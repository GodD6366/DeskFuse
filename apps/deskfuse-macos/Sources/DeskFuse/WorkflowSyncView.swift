import SwiftUI

/// 工作流定义在已配对设备间的同步状态与并发修改冲突。
struct WorkflowSyncView: View {
    @Bindable private var workflows = WorkflowStore.shared
    @Bindable var store: AppStore
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            Label(syncTitle(workflows.catalog.sync_status), systemImage: "arrow.triangle.2.circlepath")
                .font(DFType.detail).dfSecondaryText()
            ForEach(workflows.catalog.peers, id: \.device_id) { peer in
                Text("\(store.snapshot?.devices.first { $0.id == peer.device_id }?.name ?? String(peer.device_id.prefix(8)))：\(syncTitle(peer.status))")
                    .font(DFType.detail).dfSecondaryText()
            }
            ForEach(workflows.catalog.conflicts) { conflict in
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: DFSpace.xxs) {
                        Text(conflict.name).font(DFType.detail.weight(.semibold))
                        Text(conflict.message).font(DFType.detail).dfSecondaryText()
                    }
                    Spacer()
                    Button("恢复保留版本") {
                        Task {
                            do { try await workflows.restoreConflict(conflict.id) }
                            catch { failure = error.userFacingMessage }
                        }
                    }
                    .buttonStyle(.dfSecondary)
                }
            }
            if let error = workflows.catalog.error { Text(error).font(DFType.detail).foregroundStyle(DFPalette.danger) }
            if let failure { Text(failure).font(DFType.detail).foregroundStyle(DFPalette.danger) }
        }
    }

    private func syncTitle(_ status: String) -> String {
        switch status {
        case "synced": "同步完成"
        case "pending": "等待同步"
        case "waiting_for_devices": "等待已配对设备连接"
        case "error": "同步失败，已保留本地修改"
        default: status
        }
    }
}
