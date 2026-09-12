import SwiftUI
import AppKit
import UniDeskEngine

struct SettingsView: View {
    @Bindable var store: AppStore
    @State private var prefetch = false
    @State private var threshold = 10.0
    @State private var maximum = 1024.0
    @State private var clearHistory = false
    @State private var clearCache = false
    @State private var inputSettings = false
    @State private var selectedTab = "general"
    var body: some View {
        TabView(selection: $selectedTab) {
            Form {
                Section("共享") {
                    Toggle("自动同步剪贴板", isOn: Binding(get: { store.snapshot?.clipboardEnabled ?? true }, set: { store.send(.setClipboard(enabled: $0)) }))
                    Text("拖拽文件时晃动鼠标可呼出暂存区。")
                    Text("关闭主窗口后继续共享；从菜单退出 uniDesk 时停止。").foregroundStyle(.secondary)
                }
                Section("连接") {
                    Toggle("公网连接", isOn: Binding(get: { store.snapshot?.publicNetworkEnabled ?? false }, set: { store.send(.setPublicNetwork(enabled: $0)) }))
                    Text("默认只使用局域网；设备发现不代表已获得信任。").font(.caption).foregroundStyle(.secondary)
                }
                Section("诊断") { Button("导出诊断…", action: exportDiagnostics) }
            }.formStyle(.grouped).tabItem { Label("通用", systemImage: "gearshape") }.tag("general")
            Form {
                ForEach(store.snapshot?.permissions ?? [], id: \.name) { permission in
                    LabeledContent(permission.name) {
                        Text(permission.status).foregroundStyle(.secondary)
                        Button(permission.granted ? "系统设置" : "授权…") {
                            do { try requestPermission(kind: permission.kind); store.send(.refreshPermissions) }
                            catch { store.error = error.userFacingMessage }
                            if let url = URL(string: permission.settingsUrl) { NSWorkspace.shared.open(url) }
                        }
                    }
                }
                Text("键鼠共享依据真实辅助功能权限和会话状态判断。输入监控为诊断信息，不增加额外门槛。").font(.caption).foregroundStyle(.secondary)
                Button("刷新权限") { store.send(.refreshPermissions) }
            }.formStyle(.grouped).tabItem { Label("权限", systemImage: "hand.raised") }.tag("permissions")
            Form {
                Section("文件传输") {
                    Text("默认在目标设备粘贴时传输。")
                    Toggle("小文件提前传输", isOn: $prefetch)
                    HStack { Text("整批阈值"); TextField("MiB", value: $threshold, format: .number).frame(width: 90); Text("MiB") }.disabled(!prefetch)
                    HStack { Text("缓存上限"); TextField("MiB", value: $maximum, format: .number).frame(width: 90); Text("MiB") }
                    Button("保存传输设置") {
                        guard threshold.isFinite, maximum.isFinite, threshold > 0, maximum >= 1, threshold <= 1_048_576, maximum <= 16_777_216 else { store.error = "请输入有效的缓存容量与阈值"; return }
                        store.send(.setFiles(settings: FileSettings(prefetch: prefetch, threshold: UInt64(threshold * 1_048_576), maximum: UInt64(maximum * 1_048_576))))
                    }
                }
                Section("本地存储") {
                    if let cache = store.snapshot?.cache {
                        LabeledContent("已使用", value: "\(byteCount(cache.used)) / \(byteCount(cache.maximum))")
                        LabeledContent("活跃任务", value: "\(cache.active)")
                        if !cache.message.isEmpty { Text(cache.message).foregroundStyle(.secondary) }
                    }
                    Button("清除历史传输文件…", role: .destructive) { clearCache = true }
                    Button("清空剪贴板历史…", role: .destructive) { clearHistory = true }
                    Text("清理保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。").font(.caption).foregroundStyle(.secondary)
                }
            }.formStyle(.grouped).tabItem { Label("传输与存储", systemImage: "externaldrive") }.tag("storage")
            ImageWorkflowSettingsView(store: store)
                .tabItem { Label("图片工作流", systemImage: "square.stack.3d.up") }.tag("workflows")
            ShortcutSettingsPage(store: store,
                manageWorkflows: { selectedTab = "workflows" },
                configureInput: { inputSettings = true })
                .tabItem { Label("快捷键", systemImage: "keyboard") }.tag("shortcuts")
            Form {
                if let update = store.snapshot?.update {
                    UpdateControls(store: store, update: update)
                }
                Text("不主动操作时，仍会自动下载并在空闲后安装。").font(.caption).foregroundStyle(.secondary)
            }.formStyle(.grouped).tabItem { Label("更新", systemImage: "arrow.down.circle") }.tag("updates")
        }
        .padding(12)
        .onAppear { if let files = store.snapshot?.files { prefetch = files.prefetch; threshold = Double(files.threshold) / 1_048_576; maximum = Double(files.maximum) / 1_048_576 }; store.send(.refreshCache); store.send(.refreshPermissions) }
        .confirmationDialog("清空所有剪贴板历史？", isPresented: $clearHistory) { Button("清空历史", role: .destructive) { store.send(.clearHistory) } }
        .confirmationDialog("清除可回收的传输缓存？", isPresented: $clearCache) { Button("清除缓存", role: .destructive) { store.send(.clearCache) } }
        .onChange(of: store.snapshot?.input.canConfigure) { _, allowed in if allowed != true { inputSettings = false } }
        .sheet(isPresented: $inputSettings) { if store.snapshot?.input.canConfigure == true, let settings = store.snapshot?.input.settings { InputSettingsEditor(store: store, initial: settings).frame(width: 640, height: 610) } }
    }
    private func exportDiagnostics() {
        let panel = NSSavePanel(); panel.nameFieldStringValue = "uniDesk-diagnostics.jsonl"
        guard panel.runModal() == .OK, let url = panel.url, let client = store.client else { return }
        Task { do { try await client.exportDiagnostics(url.path) } catch { store.error = error.userFacingMessage } }
    }
}
