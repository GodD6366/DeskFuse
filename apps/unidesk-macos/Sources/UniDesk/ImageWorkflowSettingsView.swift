import SwiftUI
import AppKit
import UniDeskEngine

struct ImageWorkflowSettingsView: View {
    @Bindable var store: AppStore
    @State private var selectedID: String?
    @State private var draft: WorkflowDefinition?
    @State private var original: WorkflowDefinition?
    @State private var deleteRequested = false
    @State private var switchID: String?
    @State private var confirmSwitch = false
    private var workflows: [WorkflowDefinition] { store.imageWorkflowDefinitions }
    private var dirty: Bool { draft != original }

    var body: some View {
        // The settings tab owns the height. Avoid NSSplitView deriving its
        // vertical fitting size from an empty List or ContentUnavailableView.
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                HStack {
                    Text("图片流程").font(.headline)
                    Spacer()
                    Text(String(workflows.count)).font(.caption).foregroundStyle(.secondary)
                }.padding(12)
                Divider()
                List(selection: Binding(get: { selectedID }, set: select)) {
                    ForEach(workflows) { flow in
                        HStack {
                            Image(systemName: "square.stack.3d.up")
                            VStack(alignment: .leading) {
                                Text(flow.name).lineLimit(1)
                                Text(flow.enabled ? "\(flow.steps.count) 个步骤" : "已停用").font(.caption).foregroundStyle(.secondary)
                            }
                        }.tag(flow.id)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay {
                    if workflows.isEmpty {
                        Text("尚未保存流程")
                            .font(.callout).foregroundStyle(.secondary)
                            .allowsHitTesting(false)
                    }
                }
                Divider()
                HStack {
                    Menu {
                        Button("新建流程") { newDraft() }
                        Button("示例：聊天图片") {
                            var example = WorkflowDefinition(); example.name = "聊天图片"
                            example.steps[0].target_bytes = 512_000
                            example.steps.insert(.init(kind: .convert), at: 1)
                            newDraft(example)
                        }
                        Button("示例：网站配图") {
                            var example = WorkflowDefinition(); example.name = "网站配图"
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
                        store.saveImageWorkflow(value) { load(value.id) }
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
                        .font(.system(size: 40, weight: .light))
                        .foregroundStyle(.secondary)
                    VStack(spacing: 8) {
                        Text("组合你的图片动作").font(.title3.bold())
                        Text("添加压缩、尺寸或格式转换，\n再以复制或粘贴结束。")
                            .font(.callout).foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Button("新建流程") { newDraft() }
                        .buttonStyle(.borderedProminent)
                }
                .padding(32)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.background)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear { if draft == nil { load(workflows.first?.id) } }
        .onChange(of: workflows.map(\.id)) { _, ids in if draft == nil { load(ids.first) } }
        .confirmationDialog("删除这个图片流程？", isPresented: $deleteRequested) {
            Button("删除", role: .destructive) {
                if let id = original?.id { store.deleteImageWorkflow(id) }
                selectedID = nil; draft = nil; original = nil
            }
        }
        .confirmationDialog("舍弃未保存的修改？", isPresented: $confirmSwitch) {
            Button("舍弃修改", role: .destructive) { load(switchID) }
        }
    }
    private var editor: some View {
        VStack(spacing: 0) {
            Form {
                Section("流程") {
                    TextField("名称", text: value(\.name, fallback: ""))
                    Toggle("启用流程", isOn: value(\.enabled, fallback: true))
                    WorkflowShortcutRecorder(store: store, shortcut: Binding(get: { draft?.shortcut }, set: { draft?.shortcut = $0 }), workflowID: draft?.id)
                }
                Section {
                    ForEach(Array((draft?.steps ?? []).indices), id: \.self) { index in
                        WorkflowStepEditor(step: Binding(get: { draft?.steps.indices.contains(index) == true ? draft!.steps[index] : .init(kind: .copy) }, set: { if draft?.steps.indices.contains(index) == true { draft?.steps[index] = $0 } }), index: index,
                            canMoveUp: index > 0 && !(draft?.steps[index].kind.terminal ?? true),
                            canMoveDown: index + 2 < (draft?.steps.count ?? 0),
                            move: { offset in draft?.steps.swapAt(index, index + offset) },
                            remove: { draft?.steps.remove(at: index) })
                    }
                    .onMove { source, destination in
                        guard let steps = draft?.steps, destination < steps.count,
                              !source.contains(steps.count - 1) else { return }
                        draft?.steps.move(fromOffsets: source, toOffset: destination)
                    }
                    Menu("添加动作", systemImage: "plus") {
                        ForEach(WorkflowAction.allCases.filter { !$0.terminal }) { action in
                            Button(action.title, systemImage: action.icon) {
                                let insertion = max(0, (draft?.steps.count ?? 1) - 1)
                                draft?.steps.insert(.init(kind: action), at: insertion)
                            }
                        }
                    }.disabled((draft?.steps.count ?? 0) >= 32)
                } header: { Text("依次处理") } footer: {
                    Text("中间结果不会写入剪贴板；目标体积约束最终输出。上下移动或拖动步骤可调整顺序，终点始终保留在末尾。")
                }
            }.formStyle(.grouped)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            if !ImageWorkflowStore.shared.runs.isEmpty || ImageWorkflowStore.shared.error != nil {
                Divider()
                ScrollView {
                    WorkflowRunStatusView()
                }.frame(maxHeight: 150)
            }
            Divider()
            HStack {
                if dirty { Text("未保存").font(.caption).foregroundStyle(.secondary) }
                Spacer()
                Button("撤销修改") { draft = original }.disabled(!dirty || original == nil)
                Button("保存流程") {
                    guard let draft else { return }
                    store.saveImageWorkflow(draft) { original = draft; selectedID = draft.id }
                }.buttonStyle(.borderedProminent).disabled(!dirty || draft?.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false)
            }.padding(12)
            if let error = store.error { Text(error).foregroundStyle(.red).font(.caption).padding([.horizontal, .bottom], 12) }
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
