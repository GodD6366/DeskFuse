import Foundation
import DeskFuseEngine

/// Editing drafts only; the Rust snapshot remains the source of saved workflows.
struct WorkflowDefinition: Codable, Identifiable, Equatable {
    var version = 2
    var id = UUID().uuidString
    var name = "新工作流"
    var enabled = true
    var shortcut: WorkflowKey?
    var input_type: WorkflowContent = .any
    var steps: [WorkflowStep] = [.init(kind: .copy)]

    func accepts(_ type: WorkflowContent) -> Bool {
        (input_type == .any || input_type == type || (input_type == .image && type == .files)) && incompatibilities(startingWith: type).isEmpty
    }
    var incompatibilities: [String] { incompatibilities(startingWith: input_type) }
    private func incompatibilities(startingWith input: WorkflowContent) -> [String] {
        var current = input
        var issues: [String] = []
        for (index, step) in steps.enumerated() {
            let required: [WorkflowContent]
            switch step.kind {
            case .compress, .resize, .convert: required = [.image, .files]
            case .textReplace, .textTrim: required = [.text]
            case .rename, .zip: required = [.files]
            default: required = []
            }
            if current != .any && !required.isEmpty && !required.contains(current) {
                issues.append("第 \(index + 1) 步「\(step.kind.title)」不接受\(current.title)")
            }
            if step.kind == .systemShortcut { current = step.shortcut_output }
            if step.kind == .zip || step.kind == .rename { current = .files }
            if step.kind == .textReplace || step.kind == .textTrim { current = .text }
        }
        return issues
    }
}
enum WorkflowContent: String, Codable, CaseIterable, Identifiable {
    case any = "Any", text = "Text", image = "Image", files = "Files"
    var id: String { rawValue }
    var title: String {
        switch self { case .any: "任意内容"; case .text: "文字"; case .image: "图片"; case .files: "文件与文件夹" }
    }
    init(native: WorkflowContentType) {
        switch native { case .any: self = .any; case .text: self = .text; case .image: self = .image; case .files: self = .files }
    }
    var native: WorkflowContentType {
        switch self { case .any: .any; case .text: .text; case .image: .image; case .files: .files }
    }
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
    var local_shortcut_id: String?
    var output_shape = "FollowSource"
    var find = ""
    var replacement = ""
    var prefix = ""
    var suffix = ""
    var shortcut_output: WorkflowContent = .any
    init(kind: WorkflowAction) {
        self.kind = kind
        if kind == .resize { max_edge = 2048 }
    }
}
enum WorkflowAction: String, Codable, CaseIterable, Identifiable {
    case compress = "Compress", resize = "Resize", convert = "Convert"
    case textReplace = "TextReplace", textTrim = "TextTrim", rename = "Rename", zip = "Zip"
    case systemShortcut = "SystemShortcut", copy = "Copy", paste = "Paste"
    var id: String { rawValue }
    var terminal: Bool { self == .copy || self == .paste }
    var title: String {
        switch self {
        case .compress: "压缩图片"
        case .resize: "调整尺寸"
        case .convert: "转换格式"
        case .textReplace: "查找替换文字"
        case .textTrim: "去除首尾空白"
        case .rename: "批量重命名"
        case .zip: "打包 ZIP"
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
        case .textReplace: "text.magnifyingglass"
        case .textTrim: "text.alignleft"
        case .rename: "pencil.line"
        case .zip: "doc.zipper"
        case .systemShortcut: "square.stack.3d.up"
        case .copy: "doc.on.doc"
        case .paste: "doc.on.clipboard"
        }
    }
}

extension WorkflowDefinition {
    init(native: Workflow) {
        version = Int(native.version); id = native.id; name = native.name; enabled = native.enabled
        shortcut = native.shortcut.map { WorkflowKey(key_code: $0.keyCode, modifiers: $0.modifiers) }
        input_type = WorkflowContent(native: native.inputType)
        steps = native.steps.map(WorkflowStep.init(native:))
    }
    var native: Workflow {
        Workflow(version: UInt32(clamping: version), id: id, name: name, enabled: enabled,
                 shortcut: shortcut?.native, steps: steps.map(\.native), inputType: input_type.native)
    }
}
extension WorkflowStep {
    init(native: DeskFuseEngine.WorkflowStep) {
        switch native.kind {
        case .compress: kind = .compress
        case .resize: kind = .resize
        case .convert: kind = .convert
        case .textReplace: kind = .textReplace
        case .textTrim: kind = .textTrim
        case .rename: kind = .rename
        case .zip: kind = .zip
        case .systemShortcut: kind = .systemShortcut
        case .copy: kind = .copy
        case .paste: kind = .paste
        }
        quality = Int(native.quality); target_bytes = native.targetBytes; max_edge = native.maxEdge
        scale_percent = native.scalePercent; shortcut_id = native.shortcutId; shortcut_name = native.shortcutName
        find = native.find; replacement = native.replacement; prefix = native.prefix; suffix = native.suffix
        shortcut_output = WorkflowContent(native: native.shortcutOutput)
        switch native.format { case .png: format = "Png"; case .jpeg: format = "Jpeg"; case .webp: format = "Webp"; case .heic: format = "Heic" }
        switch native.outputShape { case .followSource: output_shape = "FollowSource"; case .image: output_shape = "Image"; case .file: output_shape = "File" }
    }
    var native: DeskFuseEngine.WorkflowStep {
        let type: WorkflowStepKind
        switch kind {
        case .compress: type = .compress
        case .resize: type = .resize
        case .convert: type = .convert
        case .textReplace: type = .textReplace
        case .textTrim: type = .textTrim
        case .rename: type = .rename
        case .zip: type = .zip
        case .systemShortcut: type = .systemShortcut
        case .copy: type = .copy
        case .paste: type = .paste
        }
        let imageFormat: ImageFormat
        switch format { case "Png": imageFormat = .png; case "Webp": imageFormat = .webp; case "Heic": imageFormat = .heic; default: imageFormat = .jpeg }
        let shape: ImageOutputShape
        switch output_shape { case "Image": shape = .image; case "File": shape = .file; default: shape = .followSource }
        return DeskFuseEngine.WorkflowStep(kind: type, quality: UInt8(clamping: quality), targetBytes: target_bytes,
                                 maxEdge: max_edge, scalePercent: scale_percent, format: imageFormat,
                                 shortcutId: shortcut_id, shortcutName: shortcut_name, outputShape: shape,
                                 find: find, replacement: replacement, prefix: prefix, suffix: suffix,
                                 shortcutOutput: shortcut_output.native)
    }
}
