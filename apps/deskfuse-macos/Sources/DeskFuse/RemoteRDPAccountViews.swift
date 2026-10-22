import SwiftUI

/// 预先配置 Windows 登录账号：列表、新增、编辑与删除在同一个面板内切换。
struct RemoteRDPAccountManager: View {
    private enum Mode: Equatable {
        case list
        case edit(RemoteRDPAccount?)
    }

    var store = RemoteRDPAccountStore.shared
    @Environment(\.dismiss) private var dismiss
    @State private var mode = Mode.list
    @State private var pendingRemoval: RemoteRDPAccount?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            switch mode {
            case .list: list
            case .edit(let account):
                RemoteRDPAccountEditor(store: store, account: account) { mode = .list }
            }
        }
        .padding(20)
        .frame(width: 440)
        .frame(minHeight: 300, alignment: .top)
        .confirmationDialog("删除账号“\(pendingRemoval?.title ?? "")”？", isPresented: Binding(
            get: { pendingRemoval != nil }, set: { if !$0 { pendingRemoval = nil } }
        ), presenting: pendingRemoval) { account in
            Button("删除", role: .destructive) { store.remove(account.id) }
        } message: { _ in
            Text("钥匙串中保存的密码会一并删除。")
        }
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Windows 登录账号").font(DFType.title)
                Spacer()
                DFActionButton("添加账号", systemImage: "plus") { mode = .edit(nil) }
            }
            if store.accounts.isEmpty {
                ContentUnavailableView("还没有保存的账号", systemImage: "person.crop.circle.badge.plus",
                    description: Text("添加后可在打开 Windows 应用时直接选择，无需每次输入密码。"))
                    .frame(maxWidth: .infinity, minHeight: 160)
            } else {
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(store.accounts) { account in
                            row(account)
                            if account.id != store.accounts.last?.id { Divider() }
                        }
                    }
                }
                .frame(maxHeight: 280)
                .dfCard()
            }
            Text("账号在所有 Windows 设备间共用；密码仅保存在本机 macOS 钥匙串，不会发送到 DeskFuse 服务端。")
                .font(DFType.caption)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("完成") { dismiss() }
                    .buttonStyle(.dfPrimary)
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    private func row(_ account: RemoteRDPAccount) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "person.crop.circle")
                .font(DFType.icon(.medium))
                .dfSecondaryText()
            VStack(alignment: .leading, spacing: 2) {
                Text(account.name).font(DFType.bodyMedium).lineLimit(1)
                Text(account.username).font(DFType.detail).dfSecondaryText().lineLimit(1)
            }
            Spacer()
            Button { mode = .edit(account) } label: { Image(systemName: "pencil") }
                .buttonStyle(.dfIcon)
                .help("编辑")
                .accessibilityLabel("编辑 \(account.title)")
            Button { pendingRemoval = account } label: { Image(systemName: "trash") }
                .buttonStyle(.dfIcon)
                .help("删除")
                .accessibilityLabel("删除 \(account.title)")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

/// 新增或编辑单个账号。编辑时密码留空表示保留钥匙串中的原密码。
private struct RemoteRDPAccountEditor: View {
    let store: RemoteRDPAccountStore
    let account: RemoteRDPAccount?
    let done: () -> Void
    @State private var name = ""
    @State private var username = ""
    @State private var password = ""
    @State private var message: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(account == nil ? "添加账号" : "编辑账号").font(DFType.title)
            TextField("备注名（可选，如“工作账号”）", text: $name)
                .textFieldStyle(.roundedBorder)
            TextField("Windows 用户名", text: $username)
                .textFieldStyle(.roundedBorder)
            SecureField(account == nil ? "Windows 账户密码" : "新密码（留空则保留原密码）", text: $password)
                .textFieldStyle(.roundedBorder)
            Text("使用 Windows 账户密码，而不是 PIN；Microsoft 账户填写完整邮箱。")
                .font(DFType.caption)
                .dfSecondaryText()
            if let message {
                Text(message).font(DFType.caption).foregroundStyle(DFPalette.warning)
            }
            Spacer(minLength: 0)
            HStack {
                Spacer()
                Button("取消", action: done)
                    .buttonStyle(.dfSecondary)
                    .keyboardShortcut(.cancelAction)
                Button("保存", action: save)
                    .buttonStyle(.dfPrimary)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSave)
            }
        }
        .onAppear {
            name = account.map { $0.name == $0.username ? "" : $0.name } ?? ""
            username = account?.username ?? ""
        }
    }

    private var canSave: Bool {
        !username.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (account != nil || !password.isEmpty)
    }

    private func save() {
        do {
            try store.save(id: account?.id, name: name, username: username, password: password)
            done()
        } catch {
            message = error.userFacingMessage
        }
    }
}
