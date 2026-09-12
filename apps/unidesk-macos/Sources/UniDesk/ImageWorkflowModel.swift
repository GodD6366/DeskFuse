import Foundation
import UniDeskEngine

/// Editing drafts only; the Rust snapshot remains the source of saved workflows.
struct WorkflowDefinition: Codable, Identifiable, Equatable {
    var version = 1
    var id = UUID().uuidString
    var name = "新图片流程"
    var enabled = true
    var shortcut: WorkflowKey?
    var steps: [WorkflowStep] = [.init(kind: .compress), .init(kind: .paste)]
}
struct WorkflowKey: Codable, Equatable {
    var key_code: UInt16
    var modifiers: UInt64
    var native: Shortcut { Shortcut(keyCode: key_code, modifiers: modifiers) }
}
struct WorkflowStep: Codable, Equatable {
    var kind: WorkflowAction
    var quality = 85
    var target_bytes: UInt64?
    var max_edge: UInt32?
    var scale_percent: UInt32?
    var format = "Jpeg"
    var shortcut_id = ""
    var shortcut_name = ""
    var output_shape = "FollowSource"
    init(kind: WorkflowAction) {
        self.kind = kind
        if kind == .resize { max_edge = 2048 }
    }
}
enum WorkflowAction: String, Codable, CaseIterable, Identifiable {
    case compress = "Compress", resize = "Resize", convert = "Convert"
    case systemShortcut = "SystemShortcut", copy = "Copy", paste = "Paste"
    var id: String { rawValue }
    var terminal: Bool { self == .copy || self == .paste }
    var title: String {
        switch self {
        case .compress: "压缩图片"
        case .resize: "调整尺寸"
        case .convert: "转换格式"
        case .systemShortcut: "运行系统快捷指令"
        case .copy: "复制到剪贴板"
        case .paste: "粘贴到当前应用"
        }
    }
    var icon: String {
        switch self {
        case .compress: "arrow.down.right.and.arrow.up.left"
        case .resize: "arrow.up.left.and.arrow.down.right"
        case .convert: "photo.on.rectangle.angled"
        case .systemShortcut: "square.stack.3d.up"
        case .copy: "doc.on.doc"
        case .paste: "doc.on.clipboard"
        }
    }
}

extension WorkflowDefinition {
    init(native: ImageWorkflow) {
        version = Int(native.version); id = native.id; name = native.name; enabled = native.enabled
        shortcut = native.shortcut.map { WorkflowKey(key_code: $0.keyCode, modifiers: $0.modifiers) }
        steps = native.steps.map(WorkflowStep.init(native:))
    }
    var native: ImageWorkflow {
        ImageWorkflow(version: UInt32(clamping: version), id: id, name: name, enabled: enabled,
                      shortcut: shortcut?.native, steps: steps.map(\.native))
    }
}
extension WorkflowStep {
    init(native: ImageWorkflowStep) {
        switch native.kind {
        case .compress: kind = .compress
        case .resize: kind = .resize
        case .convert: kind = .convert
        case .systemShortcut: kind = .systemShortcut
        case .copy: kind = .copy
        case .paste: kind = .paste
        }
        quality = Int(native.quality); target_bytes = native.targetBytes; max_edge = native.maxEdge
        scale_percent = native.scalePercent; shortcut_id = native.shortcutId; shortcut_name = native.shortcutName
        switch native.format { case .png: format = "Png"; case .jpeg: format = "Jpeg"; case .webp: format = "Webp"; case .heic: format = "Heic" }
        switch native.outputShape { case .followSource: output_shape = "FollowSource"; case .image: output_shape = "Image"; case .file: output_shape = "File" }
    }
    var native: ImageWorkflowStep {
        let type: WorkflowStepKind
        switch kind {
        case .compress: type = .compress
        case .resize: type = .resize
        case .convert: type = .convert
        case .systemShortcut: type = .systemShortcut
        case .copy: type = .copy
        case .paste: type = .paste
        }
        let imageFormat: ImageFormat
        switch format { case "Png": imageFormat = .png; case "Webp": imageFormat = .webp; case "Heic": imageFormat = .heic; default: imageFormat = .jpeg }
        let shape: ImageOutputShape
        switch output_shape { case "Image": shape = .image; case "File": shape = .file; default: shape = .followSource }
        return ImageWorkflowStep(kind: type, quality: UInt8(clamping: quality), targetBytes: target_bytes,
                                 maxEdge: max_edge, scalePercent: scale_percent, format: imageFormat,
                                 shortcutId: shortcut_id, shortcutName: shortcut_name, outputShape: shape)
    }
}
