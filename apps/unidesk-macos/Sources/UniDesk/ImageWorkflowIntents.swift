import AppIntents
import AppKit
import Foundation
import UniformTypeIdentifiers
import UniDeskEngine

/// Stable identity is independent of a workflow's editable display name.
struct ImageWorkflowEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "图片工作流"
    static let defaultQuery = ImageWorkflowEntityQuery()
    let id: String
    let name: String
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

struct ImageWorkflowEntityQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [ImageWorkflowEntity] {
        let available = try await Self.available()
        return identifiers.compactMap { id in available.first { $0.id == id } }
    }

    func suggestedEntities() async throws -> [ImageWorkflowEntity] { try await Self.available() }

    @MainActor private static func available() async throws -> [ImageWorkflowEntity] {
        let engine = try await ImageWorkflowIntentRuntime.readyEngine()
        return try engine.imageWorkflows().filter(\.enabled).map { ImageWorkflowEntity(id: $0.id, name: $0.name) }
    }
}

struct RunImageWorkflowIntent: AppIntent {
    static let title: LocalizedStringResource = "运行 uniDesk 图片流程"
    static let description = IntentDescription("处理单张图片并返回结果。默认不修改剪贴板；可显式执行流程末尾的复制或粘贴。")
    static let openAppWhenRun = false

    @Parameter(title: "图片工作流") var workflow: ImageWorkflowEntity
    @Parameter(title: "图片", description: "省略时读取当前系统剪贴板。") var image: IntentFile?
    @Parameter(title: "执行流程末尾输出", default: false) var performOutput: Bool

    static var parameterSummary: some ParameterSummary {
        Summary("运行 \(\.$workflow)") {
            \.$image
            \.$performOutput
        }
    }

    @MainActor func perform() async throws -> some IntentResult & ReturnsValue<IntentFile> {
        let engine = try await ImageWorkflowIntentRuntime.readyEngine()
        guard try engine.imageWorkflows().contains(where: { $0.id == workflow.id && $0.enabled }) else {
            throw ImageWorkflowIntentError.unavailable
        }
        // Capture the receiving app before processing; never activate uniDesk here.
        let targetPid = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0
        let staging = FileManager.default.temporaryDirectory.appendingPathComponent("unidesk-intent-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: staging) }
        let input: WorkflowInput
        if let image {
            let staged = try await Task.detached(priority: .userInitiated) {
                try Task.checkCancellation()
                let bytes: Data
                if let url = image.fileURL {
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    bytes = try ImageWorkflowIntentRuntime.readBounded(url)
                } else {
                    bytes = image.data
                    guard !bytes.isEmpty, bytes.count <= ImageWorkflowIntentRuntime.maximumFileBytes else {
                        throw ImageWorkflowIntentError.fileSize
                    }
                }
                try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
                let ext = URL(fileURLWithPath: image.filename).pathExtension
                let url = staging.appendingPathComponent("input").appendingPathExtension(ext.isEmpty ? "image" : ext)
                try bytes.write(to: url, options: .atomic)
                return url.path
            }.value
            input = .file(path: staged)
        } else {
            input = .clipboard
        }
        try Task.checkCancellation()
        let result = try await ImageWorkflowStore.shared.run(
            workflowId: workflow.id, input: input,
            output: performOutput ? .configured : .returnFile, targetPid: targetPid)
        try Task.checkCancellation()
        let url = URL(fileURLWithPath: result.outputPath)
        let bytes = try await Task.detached(priority: .userInitiated) {
            try ImageWorkflowIntentRuntime.readBounded(url)
        }.value
        // Own the returned bytes: task/cache lease expiry cannot invalidate Shortcuts output.
        return .result(value: IntentFile(data: bytes, filename: url.lastPathComponent,
                                        type: UTType(filenameExtension: url.pathExtension)))
    }
}

struct UniDeskImageAppShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: RunImageWorkflowIntent(),
                    phrases: ["使用 \(.applicationName) 处理图片"],
                    shortTitle: "运行图片流程", systemImageName: "photo.on.rectangle.angled")
    }
}

private enum ImageWorkflowIntentError: LocalizedError {
    case unavailable, startup(String), fileSize
    var errorDescription: String? {
        switch self {
        case .unavailable: "图片工作流已删除或停用，请重新选择。"
        case .startup(let message): "uniDesk 引擎无法启动：\(message)"
        case .fileSize: "图片文件为空或超过 128 MiB，无法交给快捷指令处理。"
        }
    }
}

private enum ImageWorkflowIntentRuntime {
    static let maximumFileBytes = 128 * 1024 * 1024

    @MainActor static func readyEngine() async throws -> Engine {
        let app = AppStore.shared
        app.start()
        // Startup currently creates Engine synchronously; allow asynchronous startup too.
        for _ in 0..<200 {
            try Task.checkCancellation()
            if let engine = app.engine { return engine }
            if !app.starting { throw ImageWorkflowIntentError.startup(app.error ?? "未知错误") }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw ImageWorkflowIntentError.startup("启动超时")
    }

    static func readBounded(_ url: URL) throws -> Data {
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values.isRegularFile == true, let size = values.fileSize, size > 0, size <= maximumFileBytes else {
            throw ImageWorkflowIntentError.fileSize
        }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let bytes = try handle.read(upToCount: maximumFileBytes + 1) ?? Data()
        guard !bytes.isEmpty, bytes.count <= maximumFileBytes else { throw ImageWorkflowIntentError.fileSize }
        return bytes
    }
}
