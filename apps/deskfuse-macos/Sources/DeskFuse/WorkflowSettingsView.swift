import SwiftUI
import AppKit
import DeskFuseEngine

struct WorkflowSettingsView: View {
    @Bindable var store: AppStore
    @State private var selectedID: String?
    @State private var draft: WorkflowDefinition?
    @State private var original: WorkflowDefinition?
    @State private var deleteRequested = false
    @State private var switchID: String?
    @State private var confirmSwitch = false
    private var workflows: [WorkflowDefinition] { store.workflowDefinitions }
    private var dirty: Bool { draft != original }

    var body: some View {
        // The settings tab owns the height. Avoid NSSplitView deriving its
        // vertical fitting size from an empty List or ContentUnavailableView.
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                HStack {
                    Text("工作流").font(DFType.section)
                    Spacer()
                    Text(String(workflows.count)).font(DFType.caption).foregroundStyle(.secondary)
                }.padding(12)
                ScrollView { WorkflowSyncView(store: store).padding(.horizontal, 12) }.frame(maxHeight: 115).padding(.bottom, 8)
                Divider()
                List(selection: Binding(get: { selectedID }, set: select)) {
                    ForEach(workflows) { flow in
                        HStack {
                            Image(systemName: "square.stack.3d.up")
                            VStack(alignment: .leading) {
                                Text(flow.name).lineLimit(1)
                                Text(flow.enabled ? "\(flow.steps.count) 个步骤 · \(flow.input_type.title)" : "已停用").font(DFType.caption).foregroundStyle(.secondary)
                                if let issue = WorkflowStore.shared.catalog.issues?.first(where: { $0.workflow_id == flow.id }) {
                                    Label(issue.message, systemImage: "exclamationmark.triangle").font(DFType.caption).foregroundStyle(DFPalette.warning)
                                }
                                if flow.steps.contains(where: { $0.kind == .systemShortcut && $0.local_shortcut_id?.isEmpty != false }) {
                                    Label("本机快捷指令待绑定", systemImage: "exclamationmark.triangle").font(DFType.caption).foregroundStyle(DFPalette.warning)
                                }
                            }
                        }.tag(flow.id)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay {
                    if workflows.isEmpty {
                        Text("尚未保存流程")
                            .font(DFType.detail).foregroundStyle(.secondary)
                            .allowsHitTesting(false)
                    }
                }
                Divider()
                HStack {
                    Menu {
                        Button("新建流程") { newDraft() }
                        Button("示例：聊天图片") {
                            var example = WorkflowDefinition(); example.name = "聊天图片"
                            example.input_type = .image
                            example.steps = [.init(kind: .compress), .init(kind: .paste)]
                            example.steps[0].target_bytes = 512_000
                            example.steps.insert(.init(kind: .convert), at: 1)
                            newDraft(example)
                        }
                        Button("示例：网站配图") {
                            var example = WorkflowDefinition(); example.name = "网站配图"; example.input_type = .image
                            var resize = WorkflowStep(kind: .resize); resize.max_edge = 1600
                            var format = WorkflowStep(kind: .convert); format.format = "Webp"
                            example.steps = [resize, format, .init(kind: .copy)]
                            newDraft(example)
                        }
                    } label: { Image(systemName: "plus") }.help("新建流程或使用示例")
                    Button { deleteRequested = true } label: { Image(systemName: "minus") }
                        .disabled(original == nil).help("删除流程")
                    Spacer()
                    Button("复制") {
                        guard var value = draft else { return }
                        value.id = UUID().uuidString; value.name += " 副本"; value.shortcut = nil
                        for index in value.steps.indices {
                            if let local = value.steps[index].local_shortcut_id, !local.isEmpty {
                                value.steps[index].shortcut_id = local
                            }
                        }
                        store.saveWorkflow(value) { load(value.id) }
                    }.disabled(draft == nil || dirty)
                }.padding(10)
            }
            .frame(width: 220)
            .frame(maxHeight: .infinity)
            Divider()
            if draft != nil {
                editor
            } else {
                VStack(spacing: 16) {
                    Image(systemName: "square.stack.3d.up")
                        .font(DFType.icon(.hero).weight(.light))
                        .foregroundStyle(.secondary)
                    VStack(spacing: 8) {
                        Text("组合你的内容处理动作").font(DFType.title.weight(.bold))
                        Text("整理文字、重命名文件、打包或处理图片，\n也可在共享给其他设备时自动运行。")
                            .font(DFType.detail).foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Button("新建流程") { newDraft() }
                        .buttonStyle(.dfPrimary)
                }
                .padding(32)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.background)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear { if draft == nil { load(workflows.first?.id) } }
        .onChange(of: workflows) { _, latest in
            if draft == nil { load(latest.first?.id) }
            else if !dirty { load(selectedID) }
        }
        .confirmationDialog("删除这个工作流？", isPresented: $deleteRequested) {
            Button("删除", role: .destructive) {
                if let id = original?.id { store.deleteWorkflow(id) }
                selectedID = nil; draft = nil; original = nil
            }
        }
        .confirmationDialog("舍弃未保存的修改？", isPresented: $confirmSwitch) {
            Button("舍弃修改", role: .destructive) { load(switchID) }
        }
    }
    private var editor: some View {
        VStack(spacing: 0) {
            DFScrollPage {
                if let original, workflows.first(where: { $0.id == original.id }) != original {
                    DFSection {
                        Text("其他设备已更新或删除此工作流。当前草稿已保留；保存会创建新修订。")
                            .foregroundStyle(DFPalette.warning)
                            .fixedSize(horizontal: false, vertical: true)
                        Button("放弃草稿并载入最新版本") { load(selectedID) }
                    }
                }

                DFSection("流程") {
                    TextField("名称", text: value(\.name, fallback: ""))
                    Picker("接受内容", selection: value(\.input_type, fallback: .any)) {
                        ForEach(WorkflowContent.allCases) { Text($0.title).tag($0) }
                    }
                    Toggle("启用流程", isOn: value(\.enabled, fallback: true))
                    WorkflowShortcutRecorder(store: store, shortcut: Binding(get: { draft?.shortcut }, set: { draft?.shortcut = $0 }), workflowID: draft?.id)
                }
                DFSection(
                    "依次处理",
                    footer: "中间结果不会写入剪贴板，源文件保持不变。图片步骤不遍历文件夹，混合非图片文件会明确失败；目标体积约束最终输出。用每行右侧的箭头调整步骤顺序，终点始终保留在末尾。"
                ) {
                    ForEach(Array((draft?.steps ?? []).indices), id: \.self) { index in
                        WorkflowStepEditor(step: Binding(get: { draft?.steps.indices.contains(index) == true ? draft!.steps[index] : .init(kind: .copy) }, set: { if draft?.steps.indices.contains(index) == true { draft?.steps[index] = $0 } }), index: index,
                            canMoveUp: index > 0 && !(draft?.steps[index].kind.terminal ?? true),
                            canMoveDown: index + 2 < (draft?.steps.count ?? 0),
                            move: { offset in draft?.steps.swapAt(index, index + offset) },
                            remove: { draft?.steps.remove(at: index) })
                    }
                    Menu("添加动作", systemImage: "plus") {
                        ForEach(WorkflowAction.allCases.filter { !$0.terminal }) { action in
                            Button(action.title, systemImage: action.icon) {
                                let insertion = max(0, (draft?.steps.count ?? 1) - 1)
                                draft?.steps.insert(.init(kind: action), at: insertion)
                            }
                        }
                    }.disabled((draft?.steps.count ?? 0) >= 32)
                    ForEach(draft?.incompatibilities ?? [], id: \.self) { issue in
                        Label(issue, systemImage: "exclamationmark.triangle").foregroundStyle(DFPalette.warning)
                    }
                }
            }
            if !WorkflowStore.shared.runs.isEmpty || WorkflowStore.shared.error != nil {
                Divider()
                ScrollView {
                    WorkflowRunStatusView()
                }.frame(maxHeight: 150)
            }
            Divider()
            HStack {
                if dirty { Text("未保存").font(DFType.caption).foregroundStyle(.secondary) }
                Spacer()
                Button("撤销修改") { draft = original }.disabled(!dirty || original == nil)
                Button("保存流程") {
                    guard let draft else { return }
                    store.saveWorkflow(draft) { load(draft.id) }
                }.buttonStyle(.dfPrimary).disabled(!dirty || !(draft?.incompatibilities.isEmpty ?? true) || draft?.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false)
            }.padding(12)
            if let error = store.error { Text(error).foregroundStyle(DFPalette.danger).font(DFType.caption).padding([.horizontal, .bottom], 12) }
        }.frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
    }
    private func value<T>(_ key: WritableKeyPath<WorkflowDefinition, T>, fallback: T) -> Binding<T> {
        Binding(get: { draft?[keyPath: key] ?? fallback }, set: { draft?[keyPath: key] = $0 })
    }
    private func select(_ id: String?) {
        if dirty { switchID = id; confirmSwitch = true } else { load(id) }
    }
    private func load(_ id: String?) {
        selectedID = id; original = workflows.first { $0.id == id }; draft = original
    }
    private func newDraft(_ template: WorkflowDefinition = WorkflowDefinition()) {
        guard !dirty else { store.error = "请先保存或撤销当前流程修改"; return }
        selectedID = template.id; original = nil; draft = template
    }
}
