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
                Text("DeskFuse 的全部全局快捷键都在这里，点组合键即可修改，立即生效。")
                    .font(DFType.detail).foregroundStyle(.secondary)
            }.padding(.horizontal, 26).padding(.top, 2).padding(.bottom, 12)
            DFScrollPage {
                ClipboardPanelShortcutSection(store: store)
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
                Text(detail).font(DFType.caption).foregroundStyle(.secondary)
            }
            Spacer()
            ShortcutKeycaps(shortcut: Shortcut(keyCode: key, modifiers: modifiers))
            Image(systemName: "lock.fill").font(DFType.caption).foregroundStyle(.tertiary).padding(.leading, 6)
        }.padding(.vertical, 3)
    }
}

/// 唤起剪贴板面板的快捷键：与其他全局快捷键放在同一页，修改后立即保存并生效。
private struct ClipboardPanelShortcutSection: View {
    @Bindable var store: AppStore
    @State private var shortcut: WorkflowKey?
    @State private var loaded = false
    @State private var saving = false
    @State private var message: String?
    @State private var failed = false

    var body: some View {
        DFSection("剪贴板", symbol: "doc.on.clipboard") {
            if loaded {
                ClipboardShortcutSettings(store: store, shortcut: Binding(get: { shortcut }, set: apply))
                    .disabled(saving)
            } else if failed, let message {
                HStack(spacing: DFSpace.s) {
                    Text(friendlyError(message)).font(DFType.detail).foregroundStyle(DFPalette.danger)
                    Button("重试") { Task { await load() } }.buttonStyle(.dfSecondary)
                }
            } else {
                Text("正在读取剪贴板快捷键…").font(DFType.detail).dfSecondaryText()
            }
            if loaded, let message {
                Text(failed ? friendlyError(message) : message)
                    .font(DFType.caption)
                    .foregroundStyle(failed ? DFPalette.danger : DFPalette.secondaryText(scheme))
            }
        }
        .task(id: store.client != nil) { if !loaded { await load() } }
    }

    @Environment(\.colorScheme) private var scheme

    private func load() async {
        guard store.client != nil else { return }
        do {
            let value = try await ClipboardLibraryStore.shared.loadSettings().shortcut ?? ClipboardLibraryShortcut()
            shortcut = value.enabled ? WorkflowKey(key_code: value.key_code, modifiers: value.modifiers) : nil
            loaded = true; failed = false; message = nil
        } catch {
            failed = true; message = error.userFacingMessage
        }
    }

    /// 录入或清除后立即提交；冲突时保留原值并说明原因。
    private func apply(_ key: WorkflowKey?) {
        if let conflict = ClipboardShortcutSettings.conflict(key, store: store) {
            failed = true; message = conflict; return
        }
        let previous = shortcut
        shortcut = key
        saving = true; failed = false; message = nil
        Task {
            defer { saving = false }
            do {
                // 只改快捷键：其余设置以引擎中的最新值为准。
                var settings = try await ClipboardLibraryStore.shared.loadSettings()
                settings.shortcut = ClipboardLibraryShortcut(enabled: key != nil,
                    key_code: key?.key_code ?? 9, modifiers: key?.modifiers ?? 0x180000)
                let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(settings))
                _ = try await ClipboardLibraryStore.shared.mutate(["op": "settings", "settings": object])
                ClipboardPanelController.shared.configure(key)
                if let error = ClipboardPanelController.shared.error {
                    failed = true; message = "快捷键已保存，但\(error)"
                } else {
                    message = key == nil ? "已关闭全局唤起" : "已生效"
                }
            } catch {
                shortcut = previous
                failed = true; message = error.userFacingMessage
            }
        }
    }
}
