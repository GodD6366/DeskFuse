import SwiftUI
import DeskFuseEngine

struct WorkflowRulesView: View {
    @Bindable var store: AppStore
    @Bindable private var workflows = WorkflowStore.shared
    @State private var rule: WorkflowRuleDraft?
    @State private var original: WorkflowRuleDraft?
    @State private var failure: String?
    @State private var saving = false
    private var rules: [WorkflowRuleDraft] {
        workflows.catalog.rules.sorted { ($0.position, $0.id) < ($1.position, $1.id) }
    }
    private var localID: String { workflows.catalog.local_device_id }
    private var peers: [Device] { (store.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked } }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("共享自动规则").font(.headline)
                Spacer()
                Button("添加规则", systemImage: "plus") {
                    var value = WorkflowRuleDraft()
                    value.sender_id = localID
                    value.workflow_id = workflows.definitions.first?.id ?? ""
                    value.position = min(rules.map(\.position).max() ?? 0, UInt32.max - 1) + 1
                    original = nil; rule = value; failure = nil
                }.disabled(rule != nil || localID.isEmpty || workflows.definitions.isEmpty)
            }
            Text("规则同步到已配对设备，只有指定发送设备运行；新规则默认关闭。手动选择优先，同级规则按顺序取第一条。")
                .font(.caption).foregroundStyle(.secondary)
            if rules.isEmpty { Text("尚未配置自动规则").foregroundStyle(.secondary) }
            ForEach(rules) { value in
                HStack(alignment: .top) {
                    Image(systemName: value.enabled ? "checkmark.circle" : "pause.circle")
                    VStack(alignment: .leading, spacing: 3) {
                        Text("\(deviceName(value.sender_id)) → \(value.target_id.isEmpty ? "所有目标设备" : deviceName(value.target_id))")
                        Text("\(value.content_type.title) · \(workflows.definitions.first { $0.id == value.workflow_id }?.name ?? "工作流已删除") · 顺序 \(value.position)")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("编辑") { original = value; rule = value; failure = nil }.disabled(rule != nil)
                    Button(role: .destructive) {
                        perform { try await workflows.deleteRule(value.id) }
                    } label: { Image(systemName: "trash") }.disabled(saving || rule != nil)
                }
            }
            if rule != nil { editor }
            if let failure { Text(failure).font(.caption).foregroundStyle(.red) }
        }
    }
    private var editor: some View {
        VStack(alignment: .leading, spacing: 10) {
            Divider()
            if let original, !workflows.catalog.rules.contains(original) {
                Text("此规则已由其他设备修改或删除。草稿已保留；保存将创建新修订。")
                    .font(.caption).foregroundStyle(.orange)
            }
            Toggle("启用规则", isOn: binding(\.enabled, false))
            LabeledContent("发送设备", value: deviceName(rule?.sender_id ?? localID))
            Picker("目标设备", selection: binding(\.target_id, "")) {
                Text("所有目标设备").tag("")
                ForEach(peers, id: \.id) { peer in Text(peer.name).tag(peer.id) }
                if let target = rule?.target_id, !target.isEmpty, !peers.contains(where: { $0.id == target }) {
                    Text(deviceName(target)).tag(target)
                }
            }
            Picker("内容类型", selection: binding(\.content_type, .any)) {
                ForEach(WorkflowContent.allCases) { Text($0.title).tag($0) }
            }
            Picker("工作流", selection: binding(\.workflow_id, "")) {
                Text("请选择…").tag("")
                ForEach(workflows.definitions) { Text($0.name).tag($0.id) }
            }
            TextField("匹配顺序（数值越小越优先）", value: binding(\.position, 0), format: .number)
            HStack {
                Spacer()
                Button("取消") { rule = nil; original = nil }.disabled(saving)
                Button("保存规则") {
                    guard let rule else { return }
                    perform { try await workflows.saveRule(rule); self.rule = nil; original = nil }
                }.disabled(saving || rule?.workflow_id.isEmpty != false)
            }
        }
    }
    private func binding<T>(_ key: WritableKeyPath<WorkflowRuleDraft, T>, _ fallback: T) -> Binding<T> {
        Binding(get: { rule?[keyPath: key] ?? fallback }, set: { rule?[keyPath: key] = $0 })
    }
    private func deviceName(_ id: String) -> String {
        if id == localID { return store.snapshot?.localName ?? "本机" }
        return peers.first { $0.id == id }?.name ?? "设备 \(id.prefix(8))"
    }
    private func perform(_ operation: @escaping @MainActor () async throws -> Void) {
        saving = true; failure = nil
        Task {
            defer { saving = false }
            do { try await operation() } catch { failure = error.userFacingMessage }
        }
    }
}

struct WorkflowSyncView: View {
    @Bindable private var workflows = WorkflowStore.shared
    @Bindable var store: AppStore
    @State private var failure: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(syncTitle(workflows.catalog.sync_status), systemImage: "arrow.triangle.2.circlepath")
                .font(.caption).foregroundStyle(.secondary)
            ForEach(workflows.catalog.peers, id: \.device_id) { peer in
                Text("\(store.snapshot?.devices.first { $0.id == peer.device_id }?.name ?? String(peer.device_id.prefix(8)))：\(syncTitle(peer.status))")
                    .font(.caption).foregroundStyle(.secondary)
            }
            ForEach(workflows.catalog.conflicts) { conflict in
                HStack(alignment: .top) {
                    VStack(alignment: .leading) {
                        Text(conflict.name).font(.caption.bold())
                        Text(conflict.message).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("恢复保留版本") {
                        Task {
                            do { try await workflows.restoreConflict(conflict.id) }
                            catch { failure = error.userFacingMessage }
                        }
                    }.controlSize(.small)
                }
            }
            if let error = workflows.catalog.error { Text(error).font(.caption).foregroundStyle(.red) }
            if let failure { Text(failure).font(.caption).foregroundStyle(.red) }
        }
    }
    private func syncTitle(_ status: String) -> String {
        switch status {
        case "synced": "同步完成"
        case "pending": "等待同步"
        case "waiting_for_devices": "等待已配对设备连接"
        case "error": "同步失败，保留本地修改"
        default: status
        }
    }
}
