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
                    .overlay(alignment: .trailing) {
                        FeatureControls(store: store, page: page)
                            .id(page)
                            .padding(.trailing, DFMetrics.pageInset)
                            .padding(.bottom, DFSpace.s)
                    }
            }
            featureError
            pageContent
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
            .dfWorkspace()
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder private var featureError: some View {
        if page == .remoteApplications, #available(macOS 14.2, *),
           let message = RemoteApplicationManager.shared.error {
            DFNotice(tone: .warning, title: "远程应用暂不可用",
                     message: friendlyError(message),
                     actionTitle: RemoteApplicationManager.shared.enabled ? "重试" : nil,
                     action: { RemoteApplicationManager.shared.retry() },
                     onDismiss: { RemoteApplicationManager.shared.error = nil })
                .padding(.horizontal, DFMetrics.pageInset)
                .padding(.bottom, DFSpace.s)
        } else if page == .microphone, store.featureEnabled(for: page) == false,
                  let message = MicrophoneSharing.shared.error {
            DFNotice(tone: .warning, title: "麦克风操作未完成", message: friendlyError(message),
                     onDismiss: { MicrophoneSharing.shared.error = nil })
                .padding(.horizontal, DFMetrics.pageInset)
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
            if store.featureEnabled(for: page) == false {
                // 每个功能只有这一种“未开启”状态：总开关关闭时直接给出动画引导。
                FeatureOnboardingView(store: store, page: page)
            } else {
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
