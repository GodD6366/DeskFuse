import SwiftUI
import DeskFuseEngine

struct ShortcutSettingsPage: View {
    @Bindable var store: AppStore
    let manageWorkflows: () -> Void
    let openInput: () -> Void
    /// 键鼠快捷键直接可改：编辑后立即提交引擎，不需要额外保存。
    @State private var shortcutDraft: [Shortcut]?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: "keyboard").font(DFType.pageTitle).foregroundStyle(.secondary)
                Text("用一组按键，运行工作流或切换控制设备。")
                    .font(.callout).foregroundStyle(.secondary)
            }.padding(.horizontal, 26).padding(.top, 2).padding(.bottom, 12)
            DFScrollPage {
                if store.workflowsEnabled {
                    WorkflowShortcutSettings(store: store, manageWorkflows: manageWorkflows)
                }
                DFSection(
                    "键鼠控制",
                    symbol: "computermouse",
                    footer: "点右侧组合键即可修改，立即生效。方向切换与锁定遵循键鼠共享状态。"
                ) {
                    if let input = store.snapshot?.input {
                        ShortcutEditor(store: store, shortcuts: shortcutBinding(input.settings))
                        if !input.managed {
                            Button("打开键鼠共享", action: openInput)
                                .buttonStyle(.dfSecondary)
                        } else {
                            Text(input.summary).font(DFType.detail).dfSecondaryText()
                            if input.managed {
                                Button("停止被控") { store.reclaim() }
                                    .buttonStyle(.dfSecondary)
                                    .disabled(input.managedPaused)
                            }
                        }
                    } else {
                        Text("正在读取键鼠设置…").font(DFType.detail).dfSecondaryText()
                    }
                }
                DFSection(
                    "固定快捷键",
                    symbol: "lock",
                    footer: "这些组合键由 DeskFuse 保留，不能分配给工作流。"
                ) {
                    fixedRow("紧急收回（保底）", detail: "自定义按键之外，此组合始终可用", key: 53, modifiers: 0x1c0000)
                    fixedRow("文件暂存区", detail: "打开或收起暂存区", key: 2, modifiers: 0x120000)
                }
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
    /// 编辑结果立即写回引擎；失败时回到快照里的真实值。
    private func shortcutBinding(_ settings: InputSettings) -> Binding<[Shortcut]> {
        Binding(get: { shortcutDraft ?? settings.shortcuts },
                set: { newValue in
                    shortcutDraft = newValue
                    var updated = settings
                    updated.shortcuts = newValue
                    store.send(.setInput(settings: updated), failed: { _ in shortcutDraft = nil })
                })
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
