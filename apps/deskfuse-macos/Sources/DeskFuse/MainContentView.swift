import SwiftUI
import DeskFuseEngine

/// Engine snapshots invalidate content without reconstructing the window chrome.
struct MainContentView: View {
    let store: AppStore
    let page: Page
    let historySearchRequest: Int
    let deviceSearchRequest: Int
    let localDeviceRequest: Int

    var body: some View {
        VStack(spacing: 0) {
            if page != .settings {
                DFTopBar(section: page.section, title: page.rawValue, badge: page.isBeta ? "Beta" : nil)
            }
            pageContent
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
            .dfWorkspace()
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .overlay(alignment: .bottomLeading) {
                ErrorNoticeView(notice: store.errorNotice)
                    .frame(width: 320)
                    .padding(.leading, 14)
                    .padding(.bottom, 14)
            }
    }

    @ViewBuilder private var pageContent: some View {
        if store.selfTest {
            DFEmptyState(symbol: "checkmark.shield", title: "正在确认更新启动",
                         message: "初始化原生界面与本地存储。")
                .onAppear { store.confirmSelfTest() }
        } else if store.isEngineLoading {
            // RootView owns the window-wide loading presentation.
            Color.clear
        } else if let snapshot = store.snapshot {
            switch page {
            case .clipboard: ClipboardView(store: store, snapshot: snapshot, searchRequest: historySearchRequest)
            case .input: InputView(store: store, snapshot: snapshot)
            case .mouse: MouseSettingsView(engine: store.engine)
            case .microphone: MicrophoneSettingsView(store: store).padding(.top, 16)
            case .devices:
                DevicesView(
                    store: store,
                    snapshot: snapshot,
                    searchRequest: deviceSearchRequest,
                    localSelectionRequest: localDeviceRequest
                )
            case .remoteApplications:
                DevicesView(
                    store: store,
                    snapshot: snapshot,
                    searchRequest: deviceSearchRequest,
                    localSelectionRequest: 0,
                    showsApplications: true
                )
            case .files: FileTransferView(store: store, snapshot: snapshot)
            case .settings: EmptyView() // Owned by the retained settings host in RootView.
            }
        } else {
            DFEmptyState(symbol: "exclamationmark.triangle",
                         title: "无法启动",
                         message: store.error.map(friendlyError) ?? "后台服务未就绪，请重新打开 DeskFuse。",
                         actionTitle: "重试",
                         action: { store.error = nil; store.start() })
        }
    }
}
