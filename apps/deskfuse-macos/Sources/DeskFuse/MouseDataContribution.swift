import AppKit
import CoreGraphics
import Foundation
import Observation
import SwiftUI
import UniformTypeIdentifiers

enum MouseConnection: String, CaseIterable, Identifiable {
    case wired
    case receiver
    case bluetooth

    var id: String { rawValue }
    var title: String {
        switch self {
        case .wired: "有线"
        case .receiver: "USB 接收器"
        case .bluetooth: "蓝牙"
        }
    }

    init(transport: String) {
        let value = transport.lowercased()
        if value.contains("bluetooth") { self = .bluetooth }
        else if value.contains("usb") { self = .receiver }
        else { self = .wired }
    }
}

struct MouseContributionSample: Identifiable, Equatable {
    var number: Int
    var name: String
    var count: Int

    var id: Int { number }
}

struct MouseContributionDraft {
    var modelName: String
    var connection: MouseConnection
    var vendorID: String
    var productID: String
    var detectedName: String?
    var isSupported: Bool
    var notes: String
    var samples: [MouseContributionSample]
    var appVersion: String
    var appBuild: String
    var systemVersion: String
    var capturedAt: Date

    func markdown() -> String {
        let title = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let detected = detectedName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        var lines = [
            "# 鼠标适配数据：\(title.isEmpty ? "未命名鼠标" : title)",
            "",
            "## 环境",
            "- DeskFuse：\(appVersion)\(appBuild.isEmpty ? "" : " (\(appBuild))")",
            "- macOS：\(systemVersion)",
            "- 采集时间：\(capturedAt.ISO8601Format())",
            "- 连接方式：\(connection.title)",
            "- Vendor ID：\(formattedID(vendorID))",
            "- Product ID：\(formattedID(productID))",
            "- 系统检测名称：\(detected.isEmpty ? "未提供" : detected)",
            "- 当前适配状态：\(isSupported ? "已适配" : "未适配（通用鼠标）")",
        ]
        let trimmedNotes = notes.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedNotes.isEmpty {
            lines += ["", "## 补充说明", "", trimmedNotes]
        }
        lines += ["", "## 按键识别", "", "编号为 macOS `mouseEventButtonNumber`，从 0 开始。", ""]
        if samples.isEmpty {
            lines.append("未记录到按键。")
        } else {
            lines += ["| 编号 | 名称 | 按下次数 |", "| ---: | --- | ---: |"]
            for sample in samples.sorted(by: { $0.number < $1.number }) {
                lines.append("| \(sample.number) | \(escaped(sample.name)) | \(sample.count) |")
            }
        }
        return lines.joined(separator: "\n") + "\n"
    }

    private func formattedID(_ raw: String) -> String {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "未提供" }
        let body = trimmed.lowercased().hasPrefix("0x") ? String(trimmed.dropFirst(2)) : trimmed
        if let value = UInt16(body, radix: 16) { return String(format: "0x%04X", value) }
        return trimmed
    }

    private func escaped(_ value: String) -> String {
        value
            .replacingOccurrences(of: "|", with: "\\|")
            .replacingOccurrences(of: "\n", with: " ")
    }
}

/// A listen-only event tap used by the contribution form. It stays independent
/// from the remapping tap so users can collect data while the feature is off.
@MainActor @Observable
final class MouseButtonProbe {
    private(set) var samples: [MouseContributionSample] = []
    private(set) var lastNumber: Int?
    private(set) var capturing = false
    private(set) var error: String?

    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    private var keyMonitor: Any?
    private static let marker: Int64 = 0x44464D42
    private static let nativeMarker: Int64 = 0x44464849

    func start() {
        guard !capturing else { return }
        error = nil
        lastNumber = nil
        let types: [CGEventType] = [.leftMouseDown, .rightMouseDown, .otherMouseDown]
        let mask = types.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
        guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .tailAppendEventTap,
                                          options: .listenOnly, eventsOfInterest: mask,
                                          callback: { _, _, event, info in
            guard let info else { return Unmanaged.passUnretained(event) }
            let probe = Unmanaged<MouseButtonProbe>.fromOpaque(info).takeUnretainedValue()
            MainActor.assumeIsolated { probe.record(event) }
            return Unmanaged.passUnretained(event)
        }, userInfo: Unmanaged.passUnretained(self).toOpaque()) else {
            error = "无法读取鼠标按键，请在系统设置中授予输入监控权限。"
            return
        }
        self.tap = tap
        source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        capturing = true
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.keyCode == 53 else { return event }
            MainActor.assumeIsolated { self?.stop() }
            return nil
        }
    }

    func stop() {
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        keyMonitor = nil
        if let tap {
            CGEvent.tapEnable(tap: tap, enable: false)
            CFMachPortInvalidate(tap)
        }
        if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        tap = nil
        source = nil
        capturing = false
    }

    func clear() {
        samples.removeAll()
        lastNumber = nil
        error = nil
    }

    func remove(_ number: Int) {
        samples.removeAll { $0.number == number }
    }

    func rename(_ number: Int, to name: String) {
        guard let index = samples.firstIndex(where: { $0.number == number }) else { return }
        samples[index].name = name
    }

    private func record(_ event: CGEvent) {
        let marker = event.getIntegerValueField(.eventSourceUserData)
        guard marker != Self.marker, marker != Self.nativeMarker else { return }
        let number = Int(event.getIntegerValueField(.mouseEventButtonNumber))
        guard (0...31).contains(number) else { return }
        lastNumber = number
        if let index = samples.firstIndex(where: { $0.number == number }) {
            samples[index].count += 1
        } else {
            samples.append(MouseContributionSample(number: number,
                                                   name: MouseSettings.buttonName(number), count: 1))
        }
    }
}

struct MouseDataContributionView: View {
    @Environment(\.dismiss) private var dismiss
    let controller: MouseButtonController
    @State private var probe = MouseButtonProbe()
    @State private var modelName: String
    @State private var connection: MouseConnection
    @State private var vendorID: String
    @State private var productID: String
    @State private var notes = ""
    @State private var detectedName: String?
    @State private var isSupported: Bool
    @State private var selectedDeviceID: String
    @State private var statusMessage: String?
    private let capturedAt: Date

    @MainActor
    init(controller: MouseButtonController, initialDevice: DetectedMouseDevice?) {
        self.controller = controller
        let device = initialDevice ?? controller.detectedMice.first
        _modelName = State(initialValue: device?.name ?? "")
        _connection = State(initialValue: MouseConnection(transport: device?.transport ?? ""))
        _vendorID = State(initialValue: device.map { String(format: "%04X", UInt16(clamping: $0.vendorID)) } ?? "")
        _productID = State(initialValue: device.map { String(format: "%04X", UInt16(clamping: $0.productID)) } ?? "")
        _detectedName = State(initialValue: device?.name)
        _isSupported = State(initialValue: device?.isSupported ?? false)
        _selectedDeviceID = State(initialValue: device?.id ?? "manual")
        capturedAt = .now
    }

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "开发版"
    }
    private var appBuild: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? ""
    }
    private var draft: MouseContributionDraft {
        MouseContributionDraft(
            modelName: modelName,
            connection: connection,
            vendorID: vendorID,
            productID: productID,
            detectedName: detectedName,
            isSupported: isSupported,
            notes: notes,
            samples: probe.samples,
            appVersion: appVersion,
            appBuild: appBuild,
            systemVersion: ProcessInfo.processInfo.operatingSystemVersionString,
            capturedAt: capturedAt)
    }
    private var markdown: String { draft.markdown() }
    private var canExport: Bool {
        !modelName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header.padding(DFSpace.xl)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: DFSpace.l) {
                    identitySection
                    captureSection
                    markdownSection
                }
                .padding(DFSpace.xl)
            }
            Divider()
            footer.padding(DFSpace.xl)
        }
        .frame(width: 680, height: 720)
        .dfFloating(opaque: true)
        .interactiveDismissDisabled(probe.capturing)
        .onAppear(perform: stopMissingCapture)
        .onDisappear { probe.stop() }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: DFSpace.m) {
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                Text("提供鼠标数据").font(DFType.pageTitle.weight(.bold))
                Text("记录型号和 macOS 按键编号，生成可粘贴到适配 Issue 的 Markdown。")
                    .font(DFType.detail).dfSecondaryText()
            }
            Spacer(minLength: DFSpace.s)
            Button { close() } label: { Image(systemName: "xmark") }
                .buttonStyle(.dfIcon).accessibilityLabel("关闭")
        }
    }

    private var identitySection: some View {
        DFSection("鼠标信息", symbol: "computermouse") {
            if !controller.detectedMice.isEmpty {
                HStack(spacing: DFSpace.m) {
                    Text("检测到的设备").frame(width: 92, alignment: .leading).dfSecondaryText()
                    Picker("检测到的设备", selection: $selectedDeviceID) {
                        Text("手动填写").tag("manual")
                        ForEach(controller.detectedMice) { device in
                            Text(device.isSupported ? "\(device.name)（已适配）" : device.name)
                                .tag(device.id)
                        }
                    }
                    .labelsHidden()
                    Spacer(minLength: 0)
                }
                .onChange(of: selectedDeviceID) { _, value in applyDevice(value) }
            }
            HStack(spacing: DFSpace.m) {
                Text("鼠标型号").frame(width: 92, alignment: .leading).dfSecondaryText()
                TextField("例如 MX Master 4", text: $modelName)
                    .textFieldStyle(.roundedBorder)
            }
            HStack(spacing: DFSpace.m) {
                Text("连接方式").frame(width: 92, alignment: .leading).dfSecondaryText()
                Picker("连接方式", selection: $connection) {
                    ForEach(MouseConnection.allCases) { item in
                        Text(item.title).tag(item)
                    }
                }
                .pickerStyle(.segmented).labelsHidden()
                Spacer(minLength: 0)
            }
            HStack(spacing: DFSpace.m) {
                Text("Vendor ID").frame(width: 92, alignment: .leading).dfSecondaryText()
                TextField("046D", text: $vendorID).textFieldStyle(.roundedBorder).frame(width: 110)
                Text("Product ID").dfSecondaryText()
                TextField("C548", text: $productID).textFieldStyle(.roundedBorder).frame(width: 110)
                Spacer(minLength: 0)
            }
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                Text("补充说明").dfSecondaryText()
                TextEditor(text: $notes)
                    .font(DFType.body)
                    .scrollContentBackground(.hidden)
                    .frame(height: 64)
                    .padding(DFSpace.s)
                    .dfControlSurface()
            }
        }
        .disabled(probe.capturing)
    }

    private var captureSection: some View {
        DFSection("按键识别", symbol: "cursorarrow.click") {
            HStack(alignment: .center, spacing: DFSpace.m) {
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text(probe.capturing ? "正在识别按键" : "依次记录鼠标按键")
                        .font(DFType.bodyMedium)
                    Text(probe.capturing
                         ? "按下每个按键，DeskFuse 会记录编号；按 Esc 结束。"
                         : "开始后只记录标准鼠标事件；未被 macOS 识别的按键可能不会出现。")
                        .font(DFType.caption).dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: DFSpace.s)
                Button(probe.capturing ? "正在识别…" : "开始识别按键") { probe.start() }
                    .buttonStyle(probe.capturing ? DFButtonStyle.dfSecondary : DFButtonStyle.dfPrimary)
                    .disabled(probe.capturing)
            }
            if let error = probe.error {
                Label(error, systemImage: "exclamationmark.circle")
                    .font(DFType.caption).foregroundStyle(DFPalette.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let last = probe.lastNumber {
                Text("最近按下：编号 \(last) · \(MouseSettings.buttonName(last))")
                    .font(DFType.caption).foregroundStyle(DFPalette.accent)
            }
            if probe.samples.isEmpty {
                Text("尚未记录按键").font(DFType.detail).dfSecondaryText()
            } else {
                ForEach(probe.samples) { sample in
                    HStack(spacing: DFSpace.m) {
                        Text("编号 \(sample.number)")
                            .font(DFType.caption.weight(.medium)).monospacedDigit()
                            .frame(width: 66, alignment: .leading)
                        TextField("按键名称", text: Binding(
                            get: { sample.name },
                            set: { probe.rename(sample.number, to: $0) }))
                            .textFieldStyle(.roundedBorder)
                        Text("\(sample.count) 次")
                            .font(DFType.caption).dfSecondaryText()
                            .frame(width: 40, alignment: .trailing)
                        Button { probe.remove(sample.number) } label: { Image(systemName: "trash") }
                            .buttonStyle(.dfIcon)
                            .accessibilityLabel("删除编号 \(sample.number)")
                    }
                    .disabled(probe.capturing)
                }
                HStack {
                    Spacer()
                    Button("清空记录") { probe.clear() }
                        .buttonStyle(.borderless).font(DFType.caption)
                        .disabled(probe.capturing)
                }
            }
        }
    }

    private var markdownSection: some View {
        DFSection("Markdown 预览", symbol: "doc.text") {
            ScrollView {
                Text(markdown)
                    .font(DFType.mono)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(height: 200)
            .padding(DFSpace.m)
            .dfControlSurface()
        }
    }

    private var footer: some View {
        HStack(spacing: DFSpace.s) {
            if let statusMessage {
                Text(statusMessage).font(DFType.caption).dfSecondaryText()
            } else if !canExport {
                Text("填写型号后可导出；建议至少记录一个按键。").font(DFType.caption).dfSecondaryText()
            }
            Spacer(minLength: DFSpace.s)
            Button("关闭") { close() }
            Button("复制 Markdown") { copy() }.disabled(!canExport)
            Button("保存 .md…") { save() }.buttonStyle(.dfPrimary).disabled(!canExport)
        }
    }

    private func applyDevice(_ id: String) {
        guard id != "manual", let device = controller.detectedMice.first(where: { $0.id == id }) else { return }
        modelName = device.name
        connection = MouseConnection(transport: device.transport)
        vendorID = String(format: "%04X", UInt16(clamping: device.vendorID))
        productID = String(format: "%04X", UInt16(clamping: device.productID))
        detectedName = device.name
        isSupported = device.isSupported
        statusMessage = nil
    }

    private func stopMissingCapture() {
        if probe.capturing { probe.stop() }
    }

    private func close() {
        probe.stop()
        dismiss()
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(markdown, forType: .string)
        statusMessage = "已复制 Markdown"
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = fileName
        panel.allowedContentTypes = [UTType(filenameExtension: "md") ?? .plainText]
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try markdown.write(to: url, atomically: true, encoding: .utf8)
                statusMessage = "已保存 \(url.lastPathComponent)"
            } catch {
                statusMessage = "保存失败：\(error.localizedDescription)"
            }
        }
    }

    private var fileName: String {
        let raw = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        let invalid = CharacterSet(charactersIn: "/:\\?%*|\"<>")
        let safe = raw.components(separatedBy: invalid).joined(separator: "-")
        return "DeskFuse-鼠标数据-\(safe.isEmpty ? "未命名" : safe).md"
    }
}
