import SwiftUI
import AppKit
import DeskFuseEngine

/// 设置按「日常 → 进阶」排列：通用、剪贴板、快捷键、文件、权限、关于与更新，最后是高级。
/// 需要重启的权限提示只在该权限确实被用到时出现，不在这里重复列出全部权限。
struct SettingsView: View, Equatable {
    let store: AppStore
    let presentation: SettingsPresentation
    let destination: SettingsDestination

    nonisolated static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.store === rhs.store && lhs.presentation == rhs.presentation && lhs.destination == rhs.destination
    }
    @State private var cacheLimit = 1024.0
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
            DFTopBar(section: Page.settings.section, title: destination.category.title)
            SettingsPageContainer(selection: selectedTab, pages: SettingsCategory.allCases.map { category in
                SettingsPageContainer.Page(id: category.rawValue,
                    content: AnyView(settingsPage(category).scrollContentBackground(.hidden)))
            })
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .dfWorkspace()
        .onChange(of: store.client != nil, initial: true) { _, ready in
            guard ready else { return }
            store.send(.refreshCache)
            store.send(.refreshPermissions)
        }
        .onChange(of: presentation.files, initial: true) { _, files in
            if let files, files != savedFiles, savedFiles == nil || (!filesDirty && !savingFiles) { loadFiles(files) }
        }
        .onChange(of: cacheLimit) { _, _ in if filesDirty { fileSaveStatus = nil; fileSaveFailure = nil } }
        .onChange(of: downloadDirectory) { _, _ in if filesDirty { fileSaveStatus = nil; fileSaveFailure = nil } }
        .confirmationDialog("清空所有剪贴板历史？", isPresented: $clearHistory) {
            Button("清空历史", role: .destructive) { store.send(.clearHistory) }
        } message: {
            Text("手动列表成员也会被清空。剪贴板内容不会重新同步回来。")
        }
        .confirmationDialog("清除可回收的传输缓存？", isPresented: $clearCache) {
            Button("清除缓存", role: .destructive) { store.send(.clearCache) }
        } message: {
            Text("只清理已完成的传输副本，不删除原始文件或已粘贴的副本。")
        }
    }

    @ViewBuilder private func settingsPage(_ category: SettingsCategory) -> some View {
        switch category {
        case .general:
            GeneralSettingsView(
                hideDockIcon: Binding(get: { store.hideDockIcon }, set: { store.setHideDockIcon($0) }),
                keepAwake: Binding(get: { store.keepAwake }, set: { store.keepAwake = $0 }))
        case .clipboard:
            clipboardSettings
        case .shortcuts:
            ShortcutSettingsPage(store: store,
                manageWorkflows: {
                    store.settingsDestination.clipboardSegment = "workflows"
                    store.settingsDestination.category = .clipboard
                },
                openInput: { store.selectedPage = .input })
        case .files:
            fileSettings
        case .permissions:
            SettingsPermissionsView(store: store, permissions: presentation.permissions)
        case .about:
            AboutSettingsView(store: store, update: presentation.update)
        case .advanced:
            advancedSettings
        }
    }

    // MARK: - 剪贴板

    private var clipboardSettings: some View {
        VStack(spacing: 0) {
            Picker("", selection: clipboardSegment) {
                Text("历史").tag("history")
                if store.workflowsEnabled { Text("工作流").tag("workflows") }
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 320)
            .padding(.horizontal, DFMetrics.pageInset)
            .padding(.bottom, DFSpace.m)
            SettingsPageContainer(selection: store.workflowsEnabled ? destination.clipboardSegment : "history", pages: [
                .init(id: "history", content: AnyView(ClipboardLibrarySettingsView())),
                .init(id: "workflows", content: AnyView(WorkflowSettingsView(store: store)))
            ])
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    // MARK: - 文件

    private var fileSettings: some View {
        VStack(spacing: 0) {
            DFScrollPage {
                DFSection("接收位置",
                          footer: "从文件暂存区发送到此设备的文件会直接保存到该位置。") {
                    HStack(alignment: .firstTextBaseline, spacing: DFSpace.m) {
                        VStack(alignment: .leading, spacing: DFSpace.xxs) {
                            Text(downloadDirectory.isEmpty ? defaultDownloadDirectory.path : downloadDirectory)
                                .font(DFType.detail)
                                .dfSecondaryText()
                                .lineLimit(2)
                                .truncationMode(.middle)
                                .textSelection(.enabled)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        Button("选择…", action: chooseDownloadDirectory)
                            .buttonStyle(.dfSecondary)
                            .disabled(savingFiles || savedFiles == nil)
                    }
                    if !downloadDirectory.isEmpty {
                        Button("恢复下载文件夹") { downloadDirectory = "" }
                            .buttonStyle(.dfPlain)
                            .disabled(savingFiles || savedFiles == nil)
                    }
                }
                DFSection("缓存") {
                    if let cache = presentation.cache {
                        DFValueRow(title: "已使用", value: "\(byteCount(cache.used)) / \(byteCount(cache.maximum))")
                        if !cache.message.isEmpty {
                            Text(cache.message).font(DFType.detail).dfSecondaryText()
                        }
                        Divider()
                    }
                    DFSettingRow(title: "缓存上限",
                                 detail: "传输过程中临时占用的空间上限；超出后复用最早的文件。") {
                        HStack(spacing: DFSpace.xs) {
                            TextField("MiB", value: $cacheLimit, format: .number).frame(width: 90)
                            Text("MiB").dfSecondaryText()
                        }
                    }
                    Divider()
                    DFSettingRow(title: "历史传输文件",
                                 detail: "保护正在接收、粘贴和导出的内容，不删除用户源文件或已粘贴副本。") {
                        Button("清除…", role: .destructive) { clearCache = true }
                    }
                }
                .disabled(savingFiles || savedFiles == nil)
            }
            SettingsSaveFooter(dirty: filesDirty, saving: savingFiles,
                canSave: savedFiles != nil && (filesDirty || fileSaveFailure != nil),
                canRevert: filesDirty, status: fileSaveStatus, error: fileSaveFailure,
                revert: { if let savedFiles { loadFiles(savedFiles) } }, save: saveFiles)
        }
    }

    // MARK: - 高级

    private var advancedSettings: some View {
        DFScrollPage {
            DFSection("功能", footer: "工作流属于进阶功能；关闭后所有入口隐藏，已注册的全局快捷键也会停用。") {
                DFToggleRow("启用工作流",
                            detail: "在剪贴板里串联多个动作并绑定全局快捷键。",
                            isOn: Binding(get: { store.workflowsEnabled },
                                          set: { store.workflowsEnabled = $0 }))
            }
            DFSection("剪贴板") {
                DFSettingRow(title: "跨设备索引与排除 App",
                             detail: "同步标题与识别文字、管理不保存历史的应用。") {
                    Button("打开剪贴板设置") {
                        store.settingsDestination.category = .clipboard
                    }
                    .buttonStyle(.dfSecondary)
                }
                Divider()
                DFSettingRow(title: "清空剪贴板历史",
                             detail: "删除本机保存的全部剪贴板历史。") {
                    Button("清空…", role: .destructive) { clearHistory = true }
                }
            }
            DFSection("诊断") {
                DFSettingRow(title: "导出诊断信息",
                             detail: "附带进程、OCR helper 的内存占用与鼠标按键状态；不包含剪贴板内容。") {
                    Button(exportingDiagnostics ? "正在导出…" : "导出…") { exportDiagnostics() }
                        .buttonStyle(.dfSecondary)
                        .disabled(store.client == nil || exportingDiagnostics)
                }
                Divider()
                DFValueRow(title: "更新通道", value: UpdateInstallationChannel.current.title)
                if UpdateInstallationChannel.current.isInternal {
                    Divider()
                    DFValueRow(title: "构建号", value: Self.buildLabel)
                }
            }
        }
    }

    private static let buildLabel: String = {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
        return version ?? "未知"
    }()

    private var filesDirty: Bool {
        guard let savedFiles else { return false }
        return cacheLimit != Double(savedFiles.maximum) / 1_048_576
            || downloadDirectory != savedFiles.downloadDirectory
    }
    private func loadFiles(_ files: FileSettings) {
        savedFiles = files
        cacheLimit = Double(files.maximum) / 1_048_576
        downloadDirectory = files.downloadDirectory
        fileSaveStatus = nil; fileSaveFailure = nil
    }
    /// 传输阈值不再暴露给用户：沿用引擎的默认提前传输策略，这里只提交缓存与接收位置。
    private func saveFiles() {
        guard !savingFiles, let savedFiles else { return }
        guard cacheLimit.isFinite, cacheLimit >= 1, cacheLimit <= 16_777_216 else {
            fileSaveFailure = "请输入有效的缓存上限（1–16 777 216 MiB）"; return
        }
        let submitted = FileSettings(prefetch: savedFiles.prefetch, threshold: savedFiles.threshold,
            maximum: UInt64(cacheLimit * 1_048_576), downloadDirectory: downloadDirectory)
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
                    let memory = await MemoryDiagnostics.capture(
                        ocr: ClipboardOCRIndexer.shared.diagnostics(),
                        selectedPage: store.selectedPage.rawValue,
                        visibleWindowCount: NSApp.windows.filter { $0.occlusionState.contains(.visible) }.count)
                    let handle = try FileHandle(forWritingTo: url)
                    handle.seekToEndOfFile()
                    handle.write(memory)
                    handle.write(MouseButtonController.shared.diagnostics())
                    try handle.close()
                }
                catch { store.error = error.userFacingMessage }
            }
        }
    }
}
