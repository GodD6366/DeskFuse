import SwiftUI
import AppKit
import DeskFuseEngine

struct WorkflowStepEditor: View {
    @Binding var step: WorkflowStep
    let index: Int
    let canMoveUp: Bool
    let canMoveDown: Bool
    let move: (Int) -> Void
    let remove: () -> Void
    @State private var shortcuts: [SystemShortcut] = []
    @State private var loading = false
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("\(index + 1)").monospacedDigit().foregroundStyle(.secondary)
                Label(step.kind.title, systemImage: step.kind.icon).font(.headline)
                Spacer()
                if !step.kind.terminal {
                    Button { move(-1) } label: { Image(systemName: "arrow.up") }.disabled(!canMoveUp).help("上移")
                    Button { move(1) } label: { Image(systemName: "arrow.down") }.disabled(!canMoveDown).help("下移")
                    Button(role: .destructive, action: remove) { Image(systemName: "minus.circle") }.help("移除步骤")
                }
            }.buttonStyle(.borderless)
            parameters
        }.padding(.vertical, 8)
            .task { if step.kind == .systemShortcut { refresh() } }
    }
    @ViewBuilder private var parameters: some View {
        switch step.kind {
        case .compress:
            Picker("模式", selection: Binding(get: { step.target_bytes != nil }, set: { step.target_bytes = $0 ? 512_000 : nil })) {
                Text("质量").tag(false); Text("目标体积").tag(true)
            }
            if step.target_bytes != nil {
                HStack {
                    Text("最终输出上限")
                    TextField("KiB", value: Binding(get: { (step.target_bytes ?? 512_000) / 1024 }, set: { step.target_bytes = min($0, 131_072) * 1024 }), format: .number).frame(width: 100)
                    Text("KiB")
                }
                Text("在后续转换和快捷指令完成后检查；无法达标时不会复制或粘贴。多个上限取最严格值。")
                    .font(.caption).foregroundStyle(.secondary)
            } else { quality }
        case .resize:
            Picker("方式", selection: Binding(get: { step.max_edge != nil }, set: { edge in step.max_edge = edge ? 2048 : nil; step.scale_percent = edge ? nil : 75 })) {
                Text("最长边").tag(true); Text("比例").tag(false)
            }
            if step.max_edge != nil {
                TextField("最长边（px）", value: Binding(get: { step.max_edge ?? 2048 }, set: { step.max_edge = $0 }), format: .number)
            } else {
                Stepper("缩放比例：\(step.scale_percent ?? 75)%", value: Binding(get: { Int(step.scale_percent ?? 75) }, set: { step.scale_percent = UInt32($0) }), in: 1...100)
            }
            Text("保持比例，不放大图片。") .font(.caption).foregroundStyle(.secondary)
        case .convert:
            Picker("格式", selection: $step.format) {
                Text("PNG").tag("Png"); Text("JPEG").tag("Jpeg"); Text("WebP").tag("Webp"); Text("HEIC").tag("Heic")
            }
            if step.format != "Png" { quality }
            Text(step.format == "Jpeg" || step.format == "Heic" ? "透明区域使用白色背景；保持当前尺寸。" : "保留透明度及当前尺寸。")
                .font(.caption).foregroundStyle(.secondary)
        case .textReplace:
            TextField("查找", text: $step.find)
            TextField("替换为", text: $step.replacement)
            Text("区分大小写，按原文匹配，不使用正则表达式。")
                .font(.caption).foregroundStyle(.secondary)
        case .textTrim:
            Text("仅去除整段文字首尾的空格、制表符和换行，保留段落内部格式。")
                .font(.caption).foregroundStyle(.secondary)
        case .rename:
            TextField("名称中查找（可留空）", text: $step.find)
            TextField("替换为", text: $step.replacement)
            TextField("添加前缀", text: $step.prefix)
            TextField("添加后缀", text: $step.suffix)
            Text("只更改所选顶层文件及文件夹的输出名称；保留扩展名和内部结构。重名自动编号，不修改源文件。")
                .font(.caption).foregroundStyle(.secondary)
        case .zip:
            Text("将所选文件与文件夹打成一个 ZIP，保留目录层级，不修改源文件。")
                .font(.caption).foregroundStyle(.secondary)
        case .systemShortcut:
            Picker("快捷指令", selection: Binding(get: { step.local_shortcut_id ?? step.shortcut_id }, set: { id in
                step.local_shortcut_id = id; step.shortcut_id = id; step.shortcut_name = shortcuts.first { $0.id == id }?.name ?? ""
            })) {
                Text("请选择…").tag("")
                if let current = step.local_shortcut_id ?? (step.shortcut_id.isEmpty ? nil : step.shortcut_id), !current.isEmpty, !shortcuts.contains(where: { $0.id == current }) {
                    Text(step.shortcut_name.isEmpty ? "已保存的快捷指令（请刷新核对）" : step.shortcut_name).tag(current)
                }
                ForEach(shortcuts, id: \.id) { shortcut in Text(shortcut.name).tag(shortcut.id) }
            }
            if step.local_shortcut_id?.isEmpty == true {
                Label("此设备尚未绑定快捷指令，请从本机列表选择。", systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
            }
            HStack {
                Button(loading ? "读取中…" : "刷新列表", action: refresh).disabled(loading)
                Button("在快捷指令中编辑") {
                    var components = URLComponents(); components.scheme = "shortcuts"; components.host = "open-shortcut"
                    components.queryItems = [URLQueryItem(name: "name", value: step.shortcut_name)]
                    if let url = components.url { NSWorkspace.shared.open(url) }
                }.disabled(step.shortcut_name.isEmpty)
            }
            Picker("返回内容类型", selection: $step.shortcut_output) {
                ForEach(WorkflowContent.allCases) { Text($0.title).tag($0) }
            }
            Text("快捷指令绑定只保存在本机；其他设备需要选择自己的快捷指令。文件以工作副本传入；返回内容按声明类型校验。交互步骤需要你响应。")
                .font(.caption).foregroundStyle(.secondary)
            if let failure { Text(failure).font(.caption).foregroundStyle(.red) }
        case .copy, .paste:
            Picker("最后执行", selection: $step.kind) {
                Text("复制到剪贴板").tag(WorkflowAction.copy)
                Text("粘贴到当前应用").tag(WorkflowAction.paste)
            }
            Picker("图片输出形式", selection: $step.output_shape) {
                Text("跟随来源").tag("FollowSource"); Text("图片内容").tag("Image"); Text("图片文件").tag("File")
            }
            Text("历史记录入口只复制处理结果；共享入口只发送结果，不执行本机复制或粘贴。图片输出形式不影响文字和文件。") .font(.caption).foregroundStyle(.secondary)
        }
    }
    private var quality: some View {
        HStack {
            Text("质量")
            Slider(value: Binding(get: { Double(step.quality) }, set: { step.quality = Int($0) }), in: 1...100, step: 1)
            Text("\(step.quality)%").monospacedDigit().frame(width: 44)
        }
    }
    private func refresh() {
        loading = true; failure = nil
        Task {
            do { shortcuts = try await WorkflowStore.shared.systemShortcuts() }
            catch { failure = error.localizedDescription }
            loading = false
        }
    }
}
