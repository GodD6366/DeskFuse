import SwiftUI
import DeskFuseEngine

/// The pane's lifetime owns its catalog connection. Opening an app delegates to
/// the existing manager so launched windows survive device/tab changes.
@available(macOS 14.2, *)
struct RemoteApplicationCatalog: View {
    let device: Device
    @State private var session: RemoteApplicationSession?
    @State private var rdpSession: RemoteRdpSession?

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                DeviceProductIconView(displayName: device.name, size: 42)
                VStack(alignment: .leading, spacing: 4) {
                    Text(device.name).font(.title2.bold())
                    Text("应用列表").font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    if let rdpSession { rdpSession.refresh() }
                    else { session?.refreshApplications() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .disabled(device.remoteAppRdp
                    ? (rdpSession == nil || rdpSession?.phase == .connecting)
                    : (session == nil || session?.busy == true))
                .help("刷新应用列表")
                .accessibilityLabel("刷新应用列表")
            }
            .padding(24)
            Divider()
            if let rdpSession {
                RemoteRdpPicker(session: rdpSession) { application, username, password in
                    RemoteApplicationManager.shared.openWindowsApplication(application,
                        session: rdpSession, username: username, password: password)
                }
            } else if let session {
                RemoteApplicationPicker(session: session, embedded: true) { application in
                    RemoteApplicationManager.shared.openApplication(application, from: session)
                }
            } else {
                ContentUnavailableView("远程应用尚未就绪", systemImage: "square.grid.2x2",
                    description: Text("请等待引擎就绪，或重新选择设备。"))
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .task(id: "\(device.id):\(device.remoteAppRdp):\(device.connected)") {
            let manager = RemoteApplicationManager.shared
            if device.remoteAppRdp {
                guard let catalog = manager.makeWindowsCatalog(for: device) else { return }
                rdpSession = catalog
                // RemoteRdpPicker owns connect(); this task owns teardown only.
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(3600)) }
                    catch { break }
                }
                catalog.stop()
                if rdpSession === catalog { rdpSession = nil }
                return
            }
            guard let catalog = manager.makeCatalog(for: device) else { return }
            session = catalog
            await catalog.start()
            // Keep the task alive until SwiftUI removes this selected device.
            // Cancellation closes only the browsing channel, not its viewers.
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(3600)) }
                catch { break }
            }
            await manager.closeCatalog(catalog)
            if session === catalog { session = nil }
        }
    }
}
