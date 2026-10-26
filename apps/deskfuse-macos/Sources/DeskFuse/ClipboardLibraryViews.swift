import SwiftUI
import AppKit
import UniformTypeIdentifiers

struct ClipboardLibraryTabs: View {
    @Environment(\.dfOpaque) private var quiet
    @Environment(\.colorScheme) private var scheme
    @Bindable var library = ClipboardLibraryStore.shared
    @Binding var selectedListID: String?
    var spacious = false
    var vertical = false
    @State private var editing: ClipboardLibraryList?
    var body: some View {
        Group {
            if vertical {
                VStack(alignment: .leading, spacing: 4) {
                    verticalTab("全部", id: nil, symbol: "rectangle.stack", count: library.counts["all"])
                    sectionLabel("类型")
                    ForEach(library.lists.filter(\.isDefaultTypeGroup)) { list in
                        verticalList(list)
                    }
                    Divider().padding(.vertical, 6)
                    sectionLabel("我的列表")
                    ForEach(library.lists.filter { !$0.isDefaultTypeGroup && $0.kind != "smart" }) { list in
                        verticalList(list)
                    }
                    Divider().padding(.vertical, 8)
                    newListButton(label: "新建列表", vertical: true)
                }
            } else {
                HStack(spacing: 6) {
                    HorizontalWheelScrollView {
                        HStack(spacing: 6) {
                            tab("全部", id: nil, count: library.counts["all"])
                            ForEach(library.lists) { list in
                                tab(list.name, id: list.id, count: library.counts[list.id])
                                    .contextMenu {
                                        Button("重命名…") { editing = list }
                                        Button("向前移动") { move(list, by: -1) }.disabled(library.lists.first?.id == list.id)
                                        Button("向后移动") { move(list, by: 1) }.disabled(library.lists.last?.id == list.id)
                                        Divider()
                                        Button("删除列表", role: .destructive) {
                                            if selectedListID == list.id { selectedListID = nil }
                                            library.perform(["op": "delete_list", "id": list.id])
                                        }
                                    }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 34, maxHeight: 36)
                    newListButton(label: nil, vertical: false)
                }
            }
        }
        .sheet(item: $editing) { list in
            ClipboardLibraryEditor(initial: list, onSaved: { selectedListID = $0 })
        }
        .onChange(of: library.lists) { _, lists in
            if let selectedListID, !lists.contains(where: { $0.id == selectedListID }) { self.selectedListID = nil }
        }
    }
    private func sectionLabel(_ title: String) -> some View {
        Text(title).font(DFType.caption.weight(.medium)).dfSecondaryText()
            .padding(.horizontal, 8).padding(.top, 8).padding(.bottom, 4)
            .accessibilityAddTraits(.isHeader)
    }

    private func verticalList(_ list: ClipboardLibraryList) -> some View {
        verticalTab(list.name, id: list.id, symbol: symbol(for: list), count: library.counts[list.id])
            .contextMenu {
                if list.kind != "smart" { Button("重命名…") { editing = list } }
                Button("向前移动") { move(list, by: -1) }.disabled(library.lists.first?.id == list.id)
                Button("向后移动") { move(list, by: 1) }.disabled(library.lists.last?.id == list.id)
                Divider()
                Button("删除列表", role: .destructive) {
                    if selectedListID == list.id { selectedListID = nil }
                    library.perform(["op": "delete_list", "id": list.id])
                }
            }
    }

    private func tab(_ name: String, id: String?, count: Int? = nil) -> some View {
        Button { selectedListID = id } label: {
            HStack(spacing: 6) {
                Text(name).lineLimit(1).font(DFType.detail)
                if let count { Text("\(count)").font(DFType.caption).monospacedDigit() }
            }
                // 横向滚动条里的标签按理想宽度排版，名称不被压缩成省略号。
                .fixedSize()
                .padding(.horizontal, spacious ? 12 : 11).padding(.vertical, spacious ? 0 : 9)
                .frame(minHeight: spacious ? DFMetrics.controlHeight : 34)
                .foregroundStyle(selectedListID == id ? DFPalette.accent : .secondary)
                .background(selectedListID == id ? DFPalette.selection
                            : spacious ? Color(nsColor: .quaternaryLabelColor).opacity(0.3) : .clear,
                            in: RoundedRectangle(cornerRadius: spacious ? 9 : 100))
                .contentShape(RoundedRectangle(cornerRadius: spacious ? 9 : 100))
        }.buttonStyle(.plain).accessibilityAddTraits(selectedListID == id ? .isSelected : [])
    }

    private func verticalTab(_ name: String, id: String?, symbol: String, count: Int?) -> some View {
        let selected = selectedListID == id
        let shape = RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        return Button { selectedListID = id } label: {
            HStack(spacing: 9) {
                Image(systemName: symbol)
                    .font(DFType.icon(.regular).weight(.medium))
                    .frame(width: 18)
                Text(name).lineLimit(1)
                Spacer(minLength: 0)
                if let count {
                    Text("\(count)").font(DFType.caption).monospacedDigit()
                        .foregroundStyle(selected && !quiet ? Color.white.opacity(0.8) : DFPalette.secondaryText(scheme))
                }
            }
            .font(DFType.body)
            .padding(.horizontal, 8)
            .frame(height: DFMetrics.rowHeight)
            .foregroundStyle(selected && !quiet ? Color.white : count == 0 && !selected ? DFPalette.secondaryText(scheme) : Color.primary)
            .background(selected ? AnyShapeStyle(quiet ? DFPalette.surface(scheme) : DFPalette.accent)
                                 : AnyShapeStyle(.clear), in: shape)
            .animation(DFMotion.snappy, value: selected)
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func symbol(for list: ClipboardLibraryList) -> String {
        switch list.name {
        case "图片": "photo"
        case "文件": "folder"
        case "纯文本", "文字": "text.alignleft"
        default: list.kind == "smart" ? "line.3.horizontal.decrease.circle" : "list.bullet"
        }
    }

    @ViewBuilder private func newListButton(label: String?, vertical: Bool) -> some View {
        Menu {
            Button("新建列表…") { editing = ClipboardLibraryList() }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "plus.circle")
                    .font(DFType.icon(.circled, weight: .medium))
                if let label { Text(label).font(DFType.detail) }
            }
            .frame(maxWidth: vertical ? .infinity : nil, alignment: .leading)
            .padding(.horizontal, vertical ? 10 : 6)
            .frame(minWidth: vertical ? nil : 32, minHeight: vertical ? 34 : (spacious ? DFMetrics.controlHeight : 32))
            .contentShape(Rectangle())
        }
        .menuStyle(.borderlessButton).menuIndicator(.hidden)
        .fixedSize(horizontal: !vertical, vertical: false)
        .help("新建列表")
        .background(spacious && !vertical ? Color(nsColor: .quaternaryLabelColor).opacity(0.3) : .clear,
                    in: RoundedRectangle(cornerRadius: DFRadius.control))
    }
    private func move(_ list: ClipboardLibraryList, by step: Int) {
        var ids = library.lists.map(\.id)
        guard let index = ids.firstIndex(of: list.id), ids.indices.contains(index + step) else { return }
        ids.swapAt(index, index + step)
        library.perform(["op": "reorder", "ids": ids])
    }
}

private struct ClipboardLibraryEditor: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable private var library = ClipboardLibraryStore.shared
    @State private var draft: ClipboardLibraryList
    @State private var saving = false
    @State private var error: String?
    let onSaved: (String) -> Void
    init(initial: ClipboardLibraryList, onSaved: @escaping (String) -> Void) {
        _draft = State(initialValue: initial)
        self.onSaved = onSaved
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(draft.kind == "smart" ? "编辑智能列表" : "编辑手动列表").font(DFType.pageTitle.weight(.bold))
            TextField("列表名称", text: $draft.name)
            if draft.kind == "smart" {
                Picker("包含匹配下列条件的内容", selection: $draft.match_mode) {
                    Text("所有条件").tag("all"); Text("任一条件").tag("any")
                }.pickerStyle(.segmented)
                ScrollView {
                    VStack(spacing: 12) {
                        ForEach($draft.rules) { $rule in
                            HStack(alignment: .top) {
                                ClipboardLibraryRuleEditor(rule: $rule, apps: library.apps, devices: library.devices)
                                Button { draft.rules.removeAll { $0.id == rule.id } } label: { Image(systemName: "minus.circle") }
                                    .buttonStyle(.borderless).disabled(draft.rules.count == 1)
                            }
                        }
                    }
                }.frame(minHeight: 160, maxHeight: 300)
                Button("增加条件", systemImage: "plus") { draft.rules.append(ClipboardLibraryRule()) }
            } else {
                Text("加入此列表的内容会长期保留，并在已配对设备间同步。删除列表不会删除其他列表的成员。").font(DFType.detail).foregroundStyle(.secondary)
            }
            if let error { Text(error).foregroundStyle(DFPalette.danger).font(DFType.detail) }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(saving ? "正在保存…" : "保存") { save() }
                    .keyboardShortcut(.defaultAction).disabled(saving || draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || invalidRules)
            }
        }.padding(24).frame(width: 620).disabled(saving)
    }
    private var invalidRules: Bool {
        draft.kind == "smart" && draft.rules.contains {
            !["today", "yesterday"].contains($0.op) && ($0.value.isEmpty || ($0.op == "last_days" && (Int($0.value) ?? 0) < 1) || ($0.op == "range" && $0.value2.isEmpty))
        }
    }
    private func save() {
        saving = true
        Task {
            do {
                draft.name = draft.name.trimmingCharacters(in: .whitespacesAndNewlines)
                try await library.save(draft)
                onSaved(draft.id)
                dismiss()
            }
            catch { self.error = error.userFacingMessage; saving = false }
        }
    }
}

private struct ClipboardLibraryRuleEditor: View {
    @Binding var rule: ClipboardLibraryRule
    let apps: [ClipboardLibraryApp]
    let devices: [ClipboardLibraryChoice]
    private var operators: [(String, String)] {
        switch rule.field {
        case "text": [("contains", "包含"), ("not_contains", "不包含")]
        case "date": [("today", "今天"), ("yesterday", "昨天"), ("last_days", "最近天数"), ("range", "日期范围")]
        default: [("is", "是"), ("is_not", "不是")]
        }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Picker("条件", selection: $rule.field) {
                    Text("来自 App").tag("app"); Text("创建日期").tag("date"); Text("内容类型").tag("kind")
                    Text("包含文本").tag("text"); Text("来源设备").tag("device")
                }.labelsHidden().frame(width: 140)
                Picker("匹配方式", selection: $rule.op) {
                    ForEach(operators, id: \.0) { Text($0.1).tag($0.0) }
                }.labelsHidden().frame(width: 130)
            }
            if rule.field == "kind" {
                Picker("内容类型", selection: $rule.value) {
                    Text("请选择类型").tag(""); Text("文字").tag("text"); Text("图片").tag("image")
                    Text("文件").tag("files"); Text("其他格式").tag("other")
                }.labelsHidden()
            } else if rule.field == "device" {
                Picker("来源设备", selection: $rule.value) {
                    Text("请选择设备").tag("")
                    ForEach(devices) { Text($0.name).tag($0.id) }
                    if !rule.value.isEmpty && !devices.contains(where: { $0.id == rule.value }) { Text("不可用设备 · \(rule.value)").tag(rule.value) }
                }.labelsHidden()
            } else if rule.field == "app" {
                HStack {
                    TextField("App bundle ID", text: $rule.value)
                    Menu("选择 App") {
                        ForEach(apps) { app in Button(app.name) { rule.value = app.id } }
                        Button("其他应用…") { if let app = ClipboardLibraryAppPicker.choose() { rule.value = app } }
                    }.fixedSize()
                }
            } else if rule.field == "date" {
                if rule.op == "last_days" { TextField("天数", text: $rule.value) }
                if rule.op == "range" {
                    HStack {
                        DatePicker("从", selection: dateBinding(second: false), displayedComponents: .date)
                        DatePicker("至", selection: dateBinding(second: true), displayedComponents: .date)
                    }
                }
            } else { TextField("要匹配的完整文字", text: $rule.value) }
        }
        .onChange(of: rule.field) { _, _ in rule.op = operators.first?.0 ?? "is"; rule.value = ""; rule.value2 = "" }
        .onChange(of: rule.op) { _, value in
            if value == "range" { rule.value = Self.format(Date()); rule.value2 = Self.format(Date()) }
            if value == "last_days" { rule.value = "7" }
        }
    }
    private func dateBinding(second: Bool) -> Binding<Date> {
        Binding(get: {
            let formatter = DateFormatter(); formatter.dateFormat = "yyyy-MM-dd"
            return formatter.date(from: second ? rule.value2 : rule.value) ?? Date()
        }, set: { if second { rule.value2 = Self.format($0) } else { rule.value = Self.format($0) } })
    }
    private static func format(_ date: Date) -> String {
        let formatter = DateFormatter(); formatter.dateFormat = "yyyy-MM-dd"; return formatter.string(from: date)
    }
}

enum ClipboardLibraryAppPicker {
    @MainActor static func choose() -> String? {
        let panel = NSOpenPanel(); panel.canChooseFiles = true; panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false; panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications"); panel.prompt = "选择应用"
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return Bundle(url: url)?.bundleIdentifier
    }
}
