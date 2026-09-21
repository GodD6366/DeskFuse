import SwiftUI
import DeskFuseEngine

/// Engine snapshots invalidate content without reconstructing the window chrome.
struct MainContentView: View {
    let store: AppStore
    let historySearchRequest: Int
    let deviceSearchRequest: Int
    let localDeviceRequest: Int

    var body: some View {
        VStack(spacing: 0) {
            if let error = store.error ?? store.snapshot?.error {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "exclamationmark.triangle")
                    Text(error).font(.callout).textSelection(.enabled)
                    Spacer(minLength: 8)
                    if store.error != nil {
                        Button("关闭") { store.error = nil }
                    }
                }
                .padding(12)
                .background(.yellow.opacity(0.12))
            }
            pageContent
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private var pageContent: some View {
        if store.selfTest {
            ContentUnavailableView("正在确认更新启动", systemImage: "checkmark.shield", description: Text("初始化原生界面与本地存储"))
                .onAppear { store.confirmSelfTest() }
        } else if store.starting {
            ProgressView("正在启动 DeskFuse…")
        } else if let snapshot = store.snapshot {
            switch store.selectedPage {
            case .clipboard: ClipboardView(store: store, snapshot: snapshot, searchRequest: historySearchRequest)
            case .input: InputView(store: store, snapshot: snapshot)
            case .devices:
                DevicesView(
                    store: store,
                    snapshot: snapshot,
                    searchRequest: deviceSearchRequest,
                    localSelectionRequest: localDeviceRequest
                )
            case .transfers: TransfersView(store: store, transfers: snapshot.transfers)
            case .lanTransfer: ShareAndTransferView(store: store, snapshot: snapshot)
            }
        } else {
            ContentUnavailableView("无法启动", systemImage: "exclamationmark.triangle", description: Text(store.error ?? "引擎未就绪"))
        }
    }
}
