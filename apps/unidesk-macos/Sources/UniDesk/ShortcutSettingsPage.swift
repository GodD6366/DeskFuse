import SwiftUI
import UniDeskEngine

struct ShortcutSettingsPage: View {
    @Bindable var store: AppStore
    let manageWorkflows: () -> Void
    let configureInput: () -> Void
    private let directions = ["向左切换", "向右切换", "向上切换", "向下切换", "锁定当前设备"]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 14) {
                Image(systemName: "keyboard").font(.system(size: 28)).foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 4) {
                    Text("快捷键").font(.title2.bold())
                    Text("用一组按键，运行图片流程或切换控制设备。")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }.padding(.horizontal, 24).padding(.top, 20).padding(.bottom, 8)
            Form {
                WorkflowShortcutSettings(store: store, manageWorkflows: manageWorkflows)
                Section {
                    if let input = store.snapshot?.input {
                        ForEach(Array(input.settings.shortcuts.prefix(5).enumerated()), id: \.offset) { index, shortcut in
                            HStack {
                                Text(directions[index])
                                Spacer()
                                ShortcutKeycaps(shortcut: shortcut)
                            }.padding(.vertical, 3)
                        }
                        if input.canConfigure {
                            Button("编辑键鼠快捷键…", action: configureInput)
                        } else {
                            Text(input.summary).font(.caption).foregroundStyle(.secondary)
                            if input.managed {
                                Button("停止被控") { store.reclaim() }.disabled(input.managedPaused)
                            }
                        }
                    } else {
                        Text("正在读取键鼠设置…").foregroundStyle(.secondary)
                    }
                } header: { Label("键鼠控制", systemImage: "computermouse") }
                  footer: { Text("方向切换与锁定遵循键鼠共享状态；显示组合键不代表当前可用。") }
                Section {
                    fixedRow("紧急收回", detail: "立即收回键鼠控制", key: 53, modifiers: 0x1c0000)
                    fixedRow("文件暂存区", detail: "打开或收起暂存区", key: 2, modifiers: 0x120000)
                } header: { Label("固定快捷键", systemImage: "lock") }
                  footer: { Text("这些组合键由 uniDesk 保留，不能分配给图片流程。") }
            }.formStyle(.grouped)
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
    private func fixedRow(_ title: String, detail: String, key: UInt16, modifiers: UInt64) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            ShortcutKeycaps(shortcut: Shortcut(keyCode: key, modifiers: modifiers))
            Image(systemName: "lock.fill").font(.caption2).foregroundStyle(.tertiary).padding(.leading, 6)
        }.padding(.vertical, 3)
    }
}
