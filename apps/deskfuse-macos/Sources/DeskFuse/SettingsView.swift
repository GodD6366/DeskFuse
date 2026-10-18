import SwiftUI
import AppKit
import DeskFuseEngine

struct SettingsView: View, Equatable {
    let store: AppStore
    let presentation: SettingsPresentation
    let destination: SettingsDestination

    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.store === rhs.store && lhs.presentation == rhs.presentation && lhs.destination == rhs.destination
    }
    @State private var prefetch = false
    @State private var threshold = 10.0
    @State private var maximum = 1024.0
    @State private var downloadDirectory = ""
    @State private var savedFiles: FileSettings?
    @State private var savingFiles = false
    @State private var fileSaveStatus: String?
    @State private var fileSaveFailure: String?
    @State private var clearHistory = false
    @State private var clearCache = false
    @State private var exportingDiagnostics = false
    private var selectedTab: String { destination.category.rawValue }
    private var clipboardSegment: Binding<String> {
        Binding(get: { store.settingsDestination.clipboardSegment },
                set: { store.settingsDestination.clipboardSegment = $0 })
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            QuietPageHeader(section: "设置", title: destination.category.title,
                badge: destination.category == .microphone ? "Beta" : nil)
            SettingsPageContainer(selection: selectedTab, pages: SettingsCategory.allCases.map { category in
                SettingsPageContainer.Page(id: category.rawValue,
                    content: AnyView(settingsPage(category).scrollContentBackground(.hidden)))
            })
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .quietWorkspaceStyle()
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
        .onChange(of: downloadDirectory) { _, _ in if filesDirty { fileSaveStatus = nil; fileSaveFailure = nil } }
        .confirmationDialog("清空所有剪贴板历史？", isPresented: $clearHistory) { Button("清空历史", role: .destructive) { store.send(.clearHistory) } }
        .confirmationDialog("清除可回收的传输缓存？", isPresented: $clearCache) { Button("清除缓存", role: .destructive) { store.send(.clearCache) } }
    }

    @ViewBuilder private func settingsPage(_ category: SettingsCategory) -> some View {
        switch category {
        case .general:
            GeneralSettingsView(
                clipboardEnabled: Binding(get: { presentation.clipboardEnabled }, set: { store.send(.setClipboard(enabled: $0)) }),
                inputSharingEnabled: Binding(
                    get: { presentation.inputSettings?.enabled ?? false },
                    set: { enabled in
                        guard var settings = presentation.inputSettings else { return }
                        settings.enabled = enabled
                        store.send(.setInput(settings: settings))
                    }
                ),
                remoteApplicationEnabled: remoteApplicationBinding,
                remoteApplicationBeta: true,
                lanWebClient: store.client,
                debugLoggingEnabled: Binding(get: { presentation.debugLoggingEnabled }, set: { store.send(.setDebugLogging(enabled: $0)) }),
                hideDockIcon: Binding(get: { store.hideDockIcon }, set: { store.setHideDockIcon($0) }),
                keepAwake: Binding(get: { store.keepAwake }, set: { store.keepAwake = $0 }),
                ready: store.client != nil, exportingDiagnostics: exportingDiagnostics, exportDiagnostics: exportDiagnostics)
        case .permissions:
            SettingsPermissionsView(store: store, permissions: presentation.permissions)
        case .storage:
            VStack(spacing: 0) {
                SystemSettingsFormPage {
                    SystemSettingsFormBlock(title: "文件传输") {
                        QuietSettingsToggle("小文件提前传输",
                            detail: "默认在目标设备粘贴时传输；开启后，低于整批阈值的文件会提前传输。", isOn: $prefetch)
                        Divider()
                        QuietSettingsRow(title: "整批阈值") {
                            HStack { TextField("MiB", value: $threshold, format: .number).frame(width: 90); Text("MiB") }
                        }.disabled(!prefetch)
                        Divider()
                        QuietSettingsRow(title: "缓存上限") {
                            HStack { TextField("MiB", value: $maximum, format: .number).frame(width: 90); Text("MiB") }
                        }
                    }
                    .disabled(savingFiles || savedFiles == nil)
                    SystemSettingsFormBlock(title: "暂存区发送") {
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("接收位置")
                                Text("从文件暂存区发送到此设备的文件会直接保存到这里。")
                                    .font(.system(size: 12)).foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                                Text(downloadDirectory.isEmpty ? defaultDownloadDirectory.path : downloadDirectory)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                                    .truncationMode(.middle)
                                    .textSelection(.enabled)
                            }
                            Spacer(minLength: 12)
                            Button("选择…", action: chooseDownloadDirectory)
                                .disabled(savingFiles || savedFiles == nil)
                        }
                        if !downloadDirectory.isEmpty {
                            Button("恢复下载文件夹") { downloadDirectory = "" }
                                .buttonStyle(.link)
                                .font(.caption)
                        }
                    }
                    .disabled(savingFiles || savedFiles == nil)
                    SystemSettingsFormBlock(title: "本地存储") {
                        if let cache = presentation.cache {
                            QuietSettingsRow(title: "已使用") {
                                Text("\(byteCount(cache.used)) / \(byteCount(cache.maximum))")
                                    .foregroundStyle(.secondary)
                            }
                            Divider()
                            QuietSettingsRow(title: "活跃任务") {
                                Text("\(cache.active)").foregroundStyle(.secondary)
                            }
                            if !cache.message.isEmpty { Text(cache.message).foregroundStyle(.secondary) }
                        }
                        Divider()
                        QuietSettingsRow(title: "历史传输文件",
                            detail: "保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。") {
                            Button("清除…", role: .destructive) { clearCache = true }
                        }
                        Divider()
                        QuietSettingsRow(title: "剪贴板历史") {
                            Button("清空…", role: .destructive) { clearHistory = true }
                        }
                    }
                }
                SettingsSaveFooter(dirty: filesDirty, saving: savingFiles,
                    canSave: savedFiles != nil && (filesDirty || fileSaveFailure != nil),
                    canRevert: filesDirty, status: fileSaveStatus, error: fileSaveFailure,
                    revert: { if let savedFiles { loadFiles(savedFiles) } }, save: saveFiles)
            }
        case .clipboard:
            VStack(spacing: 0) {
                Picker("", selection: clipboardSegment) {
                    Text("历史").tag("history")
                    Text("工作流").tag("workflows")
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 320)
                .padding(.horizontal, 36).padding(.bottom, 12)
                SettingsPageContainer(selection: destination.clipboardSegment, pages: [
                    .init(id: "history", content: AnyView(ClipboardLibrarySettingsView())),
                    .init(id: "workflows", content: AnyView(WorkflowSettingsView(store: store)))
                ])
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        case .shortcuts:
            ShortcutSettingsPage(store: store,
                manageWorkflows: {
                    store.settingsDestination.clipboardSegment = "workflows"
                    store.settingsDestination.category = .clipboard
                },
                openInput: { store.selectedPage = .input })
        case .microphone:
            MicrophoneSettingsView(store: store)
        case .updates:
            SystemSettingsFormPage {
                SystemSettingsFormBlock {
                    if let update = presentation.update {
                        UpdateControls(store: store, update: update)
                    }
                    Text("可随时手动检查；后台每 12 小时检查一次。GitHub 签名包自动下载安装，内部签名版沿用局域网下载和空闲安装流程。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        case .about:
            AboutSettingsView()
        }
    }
    private var remoteApplicationBinding: Binding<Bool>? {
        if #available(macOS 14.2, *) {
            return Binding(
                get: { RemoteApplicationManager.shared.enabled },
                set: { RemoteApplicationManager.shared.enabled = $0 }
            )
        }
        return nil
    }

    private var filesDirty: Bool {
        guard let savedFiles else { return false }
        return prefetch != savedFiles.prefetch || threshold != Double(savedFiles.threshold) / 1_048_576
            || maximum != Double(savedFiles.maximum) / 1_048_576
            || downloadDirectory != savedFiles.downloadDirectory
    }
    private func loadFiles(_ files: FileSettings) {
        savedFiles = files; prefetch = files.prefetch
        threshold = Double(files.threshold) / 1_048_576; maximum = Double(files.maximum) / 1_048_576
        downloadDirectory = files.downloadDirectory
        fileSaveStatus = nil; fileSaveFailure = nil
    }
    private func saveFiles() {
        guard !savingFiles else { return }
        guard threshold.isFinite, maximum.isFinite, threshold > 0, maximum >= 1,
              threshold <= 1_048_576, maximum <= 16_777_216 else {
            fileSaveFailure = "请输入有效的缓存容量与阈值"; return
        }
        let submitted = FileSettings(prefetch: prefetch, threshold: UInt64(threshold * 1_048_576),
            maximum: UInt64(maximum * 1_048_576), downloadDirectory: downloadDirectory)
        savingFiles = true; fileSaveFailure = nil; fileSaveStatus = nil
        store.send(.setFiles(settings: submitted), applied: {
            loadFiles(submitted); savingFiles = false; fileSaveStatus = "已保存"
        }, failed: { message in
            savingFiles = false; fileSaveFailure = message
        })
    }

    private var defaultDownloadDirectory: URL {
        FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Downloads")
    }

    private func chooseDownloadDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "选择"
        panel.message = "选择暂存区发送文件的保存位置"
        panel.directoryURL = downloadDirectory.isEmpty ? defaultDownloadDirectory : URL(fileURLWithPath: downloadDirectory)
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            Task { @MainActor in
                downloadDirectory = url.standardizedFileURL.path
                fileSaveStatus = nil
                fileSaveFailure = nil
            }
        }
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
