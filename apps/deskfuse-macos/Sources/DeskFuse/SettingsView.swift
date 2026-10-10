import SwiftUI
import AppKit
import DeskFuseEngine

struct SettingsView: View, Equatable {
    let store: AppStore
    let presentation: SettingsPresentation

    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.store === rhs.store && lhs.presentation == rhs.presentation
    }
    @State private var prefetch = false
    @State private var threshold = 10.0
    @State private var maximum = 1024.0
    @State private var savedFiles: FileSettings?
    @State private var savingFiles = false
    @State private var fileSaveStatus: String?
    @State private var fileSaveFailure: String?
    @State private var clearHistory = false
    @State private var clearCache = false
    @State private var exportingDiagnostics = false
    @SceneStorage("settings.selectedTab") private var selectedTab = "general"
    var body: some View {
        HStack(spacing: 0) {
            SettingsCategoryBar(selection: $selectedTab)
                .frame(width: 196)
            VStack(alignment: .leading, spacing: 0) {
                Text((SettingsCategory(rawValue: selectedTab) ?? .general).title)
                    .font(.system(size: 28, weight: .bold))
                    .padding(.horizontal, 36).padding(.top, 24).padding(.bottom, 20)
                SettingsPageContainer(selection: selectedTab, pages: SettingsCategory.allCases.map { category in
                    SettingsPageContainer.Page(id: category.rawValue,
                        content: AnyView(settingsPage(category).scrollContentBackground(.hidden)))
                })
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .modifier(WorkspaceContentSurface())
            .padding(12)
        }
        .background(Color(nsColor: .windowBackgroundColor).ignoresSafeArea())
        .onChange(of: store.client != nil, initial: true) { _, ready in
            guard ready else { return }
            store.send(.refreshCache)
            store.send(.refreshPermissions)
        }
        .onChange(of: presentation.files, initial: true) { _, files in
            if let files, files != savedFiles, savedFiles == nil || (!filesDirty && !savingFiles) { loadFiles(files) }
        }
        .onChange(of: prefetch) { _, _ in if filesDirty { fileSaveStatus = nil; fileSaveFailure = nil } }
        .onChange(of: threshold) { _, _ in if filesDirty { fileSaveStatus = nil; fileSaveFailure = nil } }
        .onChange(of: maximum) { _, _ in if filesDirty { fileSaveStatus = nil; fileSaveFailure = nil } }
        .confirmationDialog("清空所有剪贴板历史？", isPresented: $clearHistory) { Button("清空历史", role: .destructive) { store.send(.clearHistory) } }
        .confirmationDialog("清除可回收的传输缓存？", isPresented: $clearCache) { Button("清除缓存", role: .destructive) { store.send(.clearCache) } }
    }

    @ViewBuilder private func settingsPage(_ category: SettingsCategory) -> some View {
        switch category {
        case .general:
            GeneralSettingsView(
                clipboardEnabled: Binding(get: { presentation.clipboardEnabled }, set: { store.send(.setClipboard(enabled: $0)) }),
                debugLoggingEnabled: Binding(get: { presentation.debugLoggingEnabled }, set: { store.send(.setDebugLogging(enabled: $0)) }),
                hideDockIcon: Binding(get: { store.hideDockIcon }, set: { store.setHideDockIcon($0) }),
                keepAwake: Binding(get: { store.keepAwake }, set: { store.keepAwake = $0 }),
                ready: store.client != nil, exportingDiagnostics: exportingDiagnostics, exportDiagnostics: exportDiagnostics)
        case .permissions:
            SettingsPermissionsView(store: store, permissions: presentation.permissions)
        case .input:
            if let settings = presentation.inputSettings {
                InputSharingSettingsView(store: store, initial: settings)
            } else {
                Text("正在读取键鼠共享设置…").foregroundStyle(.secondary)
            }
        case .storage:
            VStack(spacing: 0) {
                Form {
                    Section("文件传输") {
                        Text("默认在目标设备粘贴时传输。")
                        Toggle("小文件提前传输", isOn: $prefetch)
                        HStack { Text("整批阈值"); TextField("MiB", value: $threshold, format: .number).frame(width: 90); Text("MiB") }.disabled(!prefetch)
                        HStack { Text("缓存上限"); TextField("MiB", value: $maximum, format: .number).frame(width: 90); Text("MiB") }
                    }.disabled(savingFiles || savedFiles == nil)
                    Section("本地存储") {
                        if let cache = presentation.cache {
                            LabeledContent("已使用", value: "\(byteCount(cache.used)) / \(byteCount(cache.maximum))")
                            LabeledContent("活跃任务", value: "\(cache.active)")
                            if !cache.message.isEmpty { Text(cache.message).foregroundStyle(.secondary) }
                        }
                        Button("清除历史传输文件…", role: .destructive) { clearCache = true }
                        Button("清空剪贴板历史…", role: .destructive) { clearHistory = true }
                        Text("清理保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。").font(.caption).foregroundStyle(.secondary)
                    }
                }.formStyle(.grouped)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                SettingsSaveFooter(dirty: filesDirty, saving: savingFiles,
                    canSave: savedFiles != nil && (filesDirty || fileSaveFailure != nil),
                    canRevert: filesDirty, status: fileSaveStatus, error: fileSaveFailure,
                    revert: { if let savedFiles { loadFiles(savedFiles) } }, save: saveFiles)
            }
        case .clipboard:
            ClipboardLibrarySettingsView()
        case .workflows:
            WorkflowSettingsView(store: store)
        case .shortcuts:
            ShortcutSettingsPage(store: store,
                manageWorkflows: { selectedTab = "workflows" },
                configureInput: { selectedTab = SettingsCategory.input.rawValue })
        case .updates:
            Form {
                if let update = presentation.update {
                    UpdateControls(store: store, update: update)
                }
                Text("可随时手动检查；后台每 12 小时检查一次。GitHub 签名包自动下载安装，内部签名版沿用局域网下载和空闲安装流程。").font(.caption).foregroundStyle(.secondary)
            }.formStyle(.grouped)
        }
    }
    private var filesDirty: Bool {
        guard let savedFiles else { return false }
        return prefetch != savedFiles.prefetch || threshold != Double(savedFiles.threshold) / 1_048_576
            || maximum != Double(savedFiles.maximum) / 1_048_576
    }
    private func loadFiles(_ files: FileSettings) {
        savedFiles = files; prefetch = files.prefetch
        threshold = Double(files.threshold) / 1_048_576; maximum = Double(files.maximum) / 1_048_576
        fileSaveStatus = nil; fileSaveFailure = nil
    }
    private func saveFiles() {
        guard !savingFiles else { return }
        guard threshold.isFinite, maximum.isFinite, threshold > 0, maximum >= 1,
              threshold <= 1_048_576, maximum <= 16_777_216 else {
            fileSaveFailure = "请输入有效的缓存容量与阈值"; return
        }
        let submitted = FileSettings(prefetch: prefetch, threshold: UInt64(threshold * 1_048_576),
            maximum: UInt64(maximum * 1_048_576))
        savingFiles = true; fileSaveFailure = nil; fileSaveStatus = nil
        store.send(.setFiles(settings: submitted), applied: {
            loadFiles(submitted); savingFiles = false; fileSaveStatus = "已保存"
        }, failed: { message in
            savingFiles = false; fileSaveFailure = message
        })
    }
    private func exportDiagnostics() {
        guard !exportingDiagnostics, let client = store.client else { return }
        exportingDiagnostics = true
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "DeskFuse-diagnostics.jsonl"
        // Do not enter a nested AppKit run loop from a SwiftUI action.
        panel.begin { response in
            guard response == .OK, let url = panel.url else {
                exportingDiagnostics = false
                return
            }
            Task { @MainActor in
                defer { exportingDiagnostics = false }
                do {
                    try await client.exportDiagnostics(url.path)
                    let memory = await MemoryDiagnostics.capture(ocr: ClipboardOCRIndexer.shared.diagnostics())
                    let handle = try FileHandle(forWritingTo: url)
                    handle.seekToEndOfFile()
                    handle.write(memory)
                    try handle.close()
                }
                catch { store.error = error.userFacingMessage }
            }
        }
    }
}
