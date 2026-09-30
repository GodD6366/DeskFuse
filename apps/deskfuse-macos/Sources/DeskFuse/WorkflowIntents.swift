import AppIntents
import AppKit
import Foundation
import UniformTypeIdentifiers
import DeskFuseEngine

/// Stable identity is independent of a workflow's editable display name.
struct WorkflowEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "工作流"
    static let defaultQuery = WorkflowEntityQuery()
    let id: String
    let name: String
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

struct WorkflowEntityQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [WorkflowEntity] {
        let available = try await Self.available()
        return identifiers.compactMap { id in available.first { $0.id == id } }
    }

    func suggestedEntities() async throws -> [WorkflowEntity] { try await Self.available() }

    @MainActor private static func available() async throws -> [WorkflowEntity] {
        let engine = try await WorkflowIntentRuntime.readyEngine()
        return try engine.workflows().filter(\.enabled).map { WorkflowEntity(id: $0.id, name: $0.name) }
    }
}

struct RunWorkflowIntent: AppIntent {
    static let title: LocalizedStringResource = "运行 DeskFuse 工作流"
    static let description = IntentDescription("处理文字、图片、多个文件或文件夹并返回结果。省略输入时读取剪贴板；默认不修改剪贴板。文字结果返回 UTF-8 文本文件。")
    static let openAppWhenRun = false

    @Parameter(title: "工作流") var workflow: WorkflowEntity
    @Parameter(title: "文件或图片", description: "支持多个文件和文件夹；与文字二选一。") var files: [IntentFile]?
    @Parameter(title: "文字", description: "省略全部输入时读取当前系统剪贴板。") var text: String?
    @Parameter(title: "执行流程末尾输出", default: false) var performOutput: Bool

    static var parameterSummary: some ParameterSummary {
        Summary("运行 \(\.$workflow)") {
            \.$files
            \.$text
            \.$performOutput
        }
    }

    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<[IntentFile]> {
        let engine = try await WorkflowIntentRuntime.readyEngine()
        guard try engine.workflows().contains(where: { $0.id == workflow.id && $0.enabled }) else {
            throw WorkflowIntentError.unavailable
        }
        if let files, !files.isEmpty, text != nil { throw WorkflowIntentError.ambiguousInput }
        let targetPid = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0
        let staging = FileManager.default.temporaryDirectory.appendingPathComponent("deskfuse-intent-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: staging) }
        let input: WorkflowInput
        if let files, !files.isEmpty {
            let paths = try await Task.detached(priority: .userInitiated) {
                try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
                var paths: [String] = []
                for (index, file) in files.enumerated() {
                    try Task.checkCancellation()
                    // Keep each original name while preventing equal names overwriting one another.
                    let directory = staging.appendingPathComponent(String(index), isDirectory: true)
                    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                    let name = URL(fileURLWithPath: file.filename).lastPathComponent
                    let destination = directory.appendingPathComponent(name.isEmpty ? "input" : name)
                    if let url = file.fileURL {
                        let scoped = url.startAccessingSecurityScopedResource()
                        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                        try FileManager.default.copyItem(at: url, to: destination)
                    } else {
                        let bytes = file.data
                        guard bytes.count <= WorkflowIntentRuntime.maximumFileBytes else { throw WorkflowIntentError.fileSize }
                        try bytes.write(to: destination, options: .atomic)
                    }
                    paths.append(destination.path)
                }
                return paths
            }.value
            input = .files(paths: paths)
        } else if let text {
            input = .text(value: text)
        } else {
            input = .clipboard
        }
        try Task.checkCancellation()
        let result = try await WorkflowStore.shared.run(
            workflowId: workflow.id, input: input,
            output: performOutput ? .configured : .returnResult, targetPid: targetPid)
        try Task.checkCancellation()
        if result.outputKind == .text {
            return .result(value: [IntentFile(data: Data(result.outputText.utf8), filename: "result.txt", type: .plainText)])
        }
        let paths = result.outputPaths.isEmpty && !result.outputPath.isEmpty ? [result.outputPath] : result.outputPaths
        let outputs = try await Task.detached(priority: .userInitiated) {
            try paths.map { path -> IntentFile in
                let url = URL(fileURLWithPath: path)
                let metadata = try url.resourceValues(forKeys: [.isDirectoryKey, .fileSizeKey])
                if metadata.isDirectory == true {
                    return IntentFile(fileURL: url, type: .folder)
                }
                if let size = metadata.fileSize, size <= WorkflowIntentRuntime.maximumFileBytes {
                    return IntentFile(data: try Data(contentsOf: url), filename: url.lastPathComponent,
                                      type: UTType(filenameExtension: url.pathExtension))
                }
                return IntentFile(fileURL: url, type: UTType(filenameExtension: url.pathExtension))
            }
        }.value
        return .result(value: outputs)
    }
}

struct DeskFuseImageAppShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: RunWorkflowIntent(),
                    phrases: ["使用 \(.applicationName) 运行工作流"],
                    shortTitle: "运行工作流", systemImageName: "square.stack.3d.up")
    }
}

private enum WorkflowIntentError: LocalizedError {
    case unavailable, startup(String), fileSize, ambiguousInput
    var errorDescription: String? {
        switch self {
        case .unavailable: "工作流已删除或停用，请重新选择。"
        case .startup(let message): "DeskFuse 引擎无法启动：\(message)"
        case .fileSize: "内存文件超过 128 MiB，请通过文件 URL 输入。"
        case .ambiguousInput: "请选择文字或文件输入，不要同时提供两种输入。"
        }
    }
}

private enum WorkflowIntentRuntime {
    static let maximumFileBytes = 128 * 1024 * 1024

    @MainActor static func readyEngine() async throws -> Engine {
        let app = AppStore.shared
        app.start()
        // Startup currently creates Engine synchronously; allow asynchronous startup too.
        for _ in 0..<200 {
            try Task.checkCancellation()
            if let engine = app.engine { return engine }
            if !app.starting { throw WorkflowIntentError.startup(app.error ?? "未知错误") }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw WorkflowIntentError.startup("启动超时")
    }

    static func readBounded(_ url: URL) throws -> Data {
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values.isRegularFile == true, let size = values.fileSize, size > 0, size <= maximumFileBytes else {
            throw WorkflowIntentError.fileSize
        }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let bytes = try handle.read(upToCount: maximumFileBytes + 1) ?? Data()
        guard !bytes.isEmpty, bytes.count <= maximumFileBytes else { throw WorkflowIntentError.fileSize }
        return bytes
    }
}
