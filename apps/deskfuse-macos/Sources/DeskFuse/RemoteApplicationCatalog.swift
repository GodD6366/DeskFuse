import SwiftUI
import DeskFuseEngine

/// The pane's lifetime owns its catalog connection. Opening an app delegates to
/// the existing manager so launched windows survive device/tab changes.
@available(macOS 14.2, *)
struct RemoteApplicationCatalog: View {
    let device: Device
    @State private var session: RemoteApplicationSession?

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
                    session?.refreshApplications()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .disabled(session == nil || session?.busy == true)
                .help("刷新应用列表")
                .accessibilityLabel("刷新应用列表")
            }
            .padding(24)
            Divider()
            if let session {
                RemoteApplicationPicker(session: session, embedded: true) { application in
                    RemoteApplicationManager.shared.openApplication(application, from: session)
                }
            } else {
                ContentUnavailableView("远程应用尚未就绪", systemImage: "square.grid.2x2",
                    description: Text("请等待引擎就绪，或重新选择设备。"))
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .task {
            let manager = RemoteApplicationManager.shared
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
