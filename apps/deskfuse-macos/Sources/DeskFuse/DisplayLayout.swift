import SwiftUI
import DeskFuseEngine

/// Screen coordinates come from the engine; this canvas never invents a topology.
/// Dashed screens are reserved but not live; reconnect dials the peer and starts
/// a fresh input activation so the other side can prepare the session.
struct DisplayLayout: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    var onRemoveDevice: ((String) -> Void)? = nil
    var onReconnectDevice: ((String) -> Void)? = nil
    var reconnecting: Set<String> = []
    var editingDisabled = false
    @State private var dragging: String?
    @State private var delta: CGSize = .zero
    @State private var dragDisplays: [Display] = []
    @State private var dragScale: CGFloat = 1
    @State private var snapped = false
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GeometryReader { geometry in
            let displays = dragging == nil ? snapshot.input.displays.sorted { $0.layoutID < $1.layoutID } : dragDisplays
            let minX = displays.map(\.x).min() ?? 0
            let minY = displays.map(\.y).min() ?? 0
            let maxX = displays.map { $0.x + $0.width }.max() ?? 1
            let maxY = displays.map { $0.y + $0.height }.max() ?? 1
            let scale = max(0.001, min(
                (geometry.size.width - 64) / max(1, maxX - minX),
                (geometry.size.height - 64) / max(1, maxY - minY)
            ))
            let offsetX = (geometry.size.width - (maxX - minX) * scale) / 2
            let offsetY = (geometry.size.height - (maxY - minY) * scale) / 2

            ZStack(alignment: .topLeading) {
                Color(nsColor: .controlBackgroundColor)
                Canvas { context, size in
                    var dots = Path()
                    for x in stride(from: 16.0, to: size.width, by: 24.0) {
                        for y in stride(from: 16.0, to: size.height, by: 24.0) {
                            dots.addEllipse(in: CGRect(x: x, y: y, width: 1.5, height: 1.5))
                        }
                    }
                    context.fill(dots, with: .color(.secondary.opacity(contrast == .increased ? 0.5 : 0.22)))
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)

                if displays.isEmpty {
                    ContentUnavailableView("等待显示器布局", systemImage: "display",
                                           description: Text("设备连接后显示真实布局。"))
                        .frame(width: geometry.size.width, height: geometry.size.height)
                }

                ForEach(Array(displays.enumerated()), id: \.element.layoutID) { index, display in
                    let outline = DisplayPalette.color(for: display.deviceId, displays: displays)
                    let reconnecting = isReconnecting(display.deviceId)
                    let canReconnect = showsReconnect(for: display)
                    ZStack {
                        // 显示屏幕壁纸
                        DisplayWallpaperView(display: display, outline: outline)
                        
                        RoundedRectangle(cornerRadius: 2)
                            .fill(outline.opacity(dragging == display.deviceId ? 0.09 : (display.ready ? 0.025 : 0.06)))
                            .overlay {
                                RoundedRectangle(cornerRadius: 2)
                                    .strokeBorder(outline, style: StrokeStyle(
                                        lineWidth: contrast == .increased || dragging == display.deviceId ? 3 : 2,
                                        dash: display.ready ? [] : [6, 4]
                                    ))
                            }
                            .contentShape(Rectangle())
                            .gesture(DragGesture(minimumDistance: 4)
                                .onChanged { value in
                                    guard !editingDisabled else { return }
                                    if dragging == nil {
                                        dragDisplays = displays
                                        dragScale = scale
                                        dragging = display.deviceId
                                    }
                                    guard dragging == display.deviceId else { return }
                                    updateDrag(value.translation, device: display.deviceId, attach: false)
                                }
                                .onEnded { value in
                                    guard dragging == display.deviceId else { return }
                                    defer {
                                        dragging = nil
                                        dragDisplays = []
                                        delta = .zero
                                        snapped = false
                                    }
                                    guard !editingDisabled else { return }
                                    if updateDrag(value.translation, device: display.deviceId, attach: true),
                                       let anchor = dragDisplays.first(where: { $0.layoutID == display.layoutID }) {
                                        store.send(.moveInputDevice(deviceId: display.deviceId, displayId: display.id,
                                            x: anchor.x + delta.width / dragScale,
                                            y: anchor.y + delta.height / dragScale))
                                    }
                                })
                        DisplayScreenLabel(
                            index: index + 1,
                            display: display,
                            outline: outline,
                            showReconnect: canReconnect,
                            reconnecting: reconnecting,
                            reduceMotion: reduceMotion,
                            action: { onReconnectDevice?(display.deviceId) }
                        )
                        .disabled(editingDisabled)
                        .padding(10)
                        .allowsHitTesting(canReconnect)
                    }
                    .frame(width: display.width * scale, height: display.height * scale)
                    .offset(
                        x: offsetX + (display.x - minX) * scale + (dragging == display.deviceId ? delta.width : 0),
                        y: offsetY + (display.y - minY) * scale + (dragging == display.deviceId ? delta.height : 0)
                    )
                    .contextMenu {
                        if !display.local {
                            if canReconnect {
                                Button(reconnecting ? "正在连接…" : "重新连接", systemImage: "arrow.clockwise") {
                                    onReconnectDevice?(display.deviceId)
                                }.disabled(editingDisabled || reconnecting)
                            }
                            if let onRemoveDevice {
                                Button("移除此设备的屏幕", systemImage: "minus.circle") {
                                    onRemoveDevice(display.deviceId)
                                }.disabled(editingDisabled)
                            }
                        }
                    }
                    .help(screenHelp(display, reconnecting: reconnecting))
                    .accessibilityElement(children: canReconnect ? .contain : .ignore)
                    .accessibilityLabel("屏幕 \(index + 1)，\(display.name)，\(display.layoutSizeLabel)，\(display.ready ? "已就绪" : "未就绪")")
                    .accessibilityHint(canReconnect ? "未正式连接。可重新连接并通知对端准备键鼠共享。" : "")
                }
            }
            .overlay(alignment: .topLeading) {
                if dragging != nil {
                    Label(snapped ? "已吸附边缘" : "松手自动贴合相邻屏幕", systemImage: "rectangle.connected.to.line.below")
                        .font(.caption).padding(8).background(.regularMaterial, in: Capsule()).padding(12)
                        .allowsHitTesting(false)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.separator.opacity(0.5)))
        }
    }

    private func showsReconnect(for display: Display) -> Bool {
        guard !display.ready, !display.local, onReconnectDevice != nil else { return false }
        let connected = snapshot.devices.contains { $0.id == display.deviceId && $0.connected }
        return snapshot.input.settings.enabled || !connected
    }

    private func isReconnecting(_ id: String) -> Bool {
        reconnecting.contains(id)
            || snapshot.devices.contains { $0.id == id && $0.detail.contains("正在连接") }
    }

    private func screenHelp(_ display: Display, reconnecting: Bool) -> String {
        let status: String
        if display.ready {
            status = "已就绪"
        } else if reconnecting {
            status = "正在重新连接"
        } else {
            status = "未就绪，可点中间的重连按钮通知对端准备连接"
        }
        return "\(display.name)\(display.local ? " · 本机" : "") · \(status)\n拖动自动吸附边缘；同一设备的屏幕整体移动，松手贴合相邻屏幕。"
    }

    @discardableResult
    private func updateDrag(_ translation: CGSize, device: String, attach: Bool) -> Bool {
        do {
            let offset = try previewDisplayMove(displays: dragDisplays, deviceId: device,
                deltaX: translation.width / dragScale, deltaY: translation.height / dragScale,
                snapDistance: 14 / dragScale, attach: attach)
            delta = CGSize(width: offset.x * dragScale, height: offset.y * dragScale)
            snapped = abs(delta.width - translation.width) > 0.1 || abs(delta.height - translation.height) > 0.1
            return true
        } catch {
            if attach { store.error = error.userFacingMessage }
            return false
        }
    }

}

extension Display {
    var layoutID: String { deviceId + ":" + id }
    // The topology contains logical desktop bounds, not physical panel inches.
    var layoutSizeLabel: String { "\(Int(width)) × \(Int(height)) pt" }
}

/// Identity stays centered; the reconnect control only occupies the middle when
/// the screen is dashed. Smaller tiles drop labels before dropping the button.
private struct DisplayScreenLabel: View {
    let index: Int
    let display: Display
    let outline: Color
    let showReconnect: Bool
    let reconnecting: Bool
    let reduceMotion: Bool
    let action: () -> Void

    var body: some View {
        if showReconnect {
            ViewThatFits(in: .vertical) {
                VStack(spacing: 10) {
                    identity
                    reconnectButton
                }
                VStack(spacing: 8) {
                    Text(display.name)
                        .font(.system(size: 12, weight: .semibold))
                        .lineLimit(1)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                    reconnectButton
                }
                reconnectButton
            }
        } else {
            identity
        }
    }

    private var identity: some View {
        VStack(spacing: 6) {
            Text(String(format: "%02d", index))
                .font(.system(size: 22, weight: .semibold, design: .rounded))
                .foregroundStyle(outline)
            Text(display.name + (display.local ? " · 本机" : ""))
                .font(.system(size: 12, weight: .semibold))
                .lineLimit(2).multilineTextAlignment(.center)
            Text(display.layoutSizeLabel)
                .font(.system(size: 10)).monospacedDigit().foregroundStyle(.secondary)
                .lineLimit(1).minimumScaleFactor(0.7)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    private var reconnectButton: some View {
        DisplayReconnectButton(color: outline, busy: reconnecting, reduceMotion: reduceMotion, action: action)
    }
}

private struct DisplayReconnectButton: View {
    let color: Color
    let busy: Bool
    let reduceMotion: Bool
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button {
            guard !busy else { return }
            action()
        } label: {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 6) {
                    reconnectGlyph
                    Text(busy ? "正在连接" : "重连")
                }
                reconnectGlyph
            }
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 12)
            .frame(height: 28)
            .fixedSize()
            .background(color.opacity(busy ? 0.78 : (hovering ? 0.96 : 0.9)), in: Capsule())
            .brightness(!busy && hovering ? 0.04 : 0)
            .animation(.easeInOut(duration: 0.15), value: hovering)
            .contentShape(Capsule())
        }
        .buttonStyle(DisplayReconnectButtonStyle(reduceMotion: reduceMotion))
        .allowsHitTesting(!busy)
        .onHover { hovering = $0 }
        .help(busy ? "正在重新连接，并通知对端准备输入会话" : "重新连接此设备，并通知对端准备键鼠共享")
        .accessibilityLabel(busy ? "正在重新连接" : "重新连接")
        .accessibilityHint("重新连接此设备，并通知对端准备键鼠共享")
        .accessibilityAddTraits(busy ? .updatesFrequently : .isButton)
    }

    private var reconnectGlyph: some View {
        ZStack {
            if busy {
                ProgressView()
                    .controlSize(.mini)
                    .tint(.white)
            } else {
                Image(systemName: "arrow.clockwise")
            }
        }
        .frame(width: 12, height: 12)
    }
}

private struct DisplayReconnectButtonStyle: ButtonStyle {
    let reduceMotion: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(!reduceMotion && configuration.isPressed ? 0.97 : 1)
            .animation(.timingCurve(0.23, 1, 0.32, 1, duration: 0.16), value: configuration.isPressed)
    }
}

/// Deterministic across machines; local identity is a label, not a different color.
enum DisplayPalette {
    private static let colors: [Color] = [.blue, .teal, .purple, .orange, .pink, .green, .indigo, .brown]
    static func color(for device: String, displays: [Display]) -> Color {
        let ids = Set(displays.map(\.deviceId)).sorted()
        let slots = max(colors.count, ids.count)
        var occupied = Set<Int>()
        for id in ids {
            let hash = id.utf8.reduce(UInt64(14695981039346656037)) { ($0 ^ UInt64($1)) &* 1099511628211 }
            var index = Int(hash % UInt64(slots))
            while occupied.contains(index) { index = (index + 1) % slots }
            occupied.insert(index)
            if id == device {
                return index < colors.count ? colors[index] : Color(
                    hue: (Double(index) * 0.61803398875).truncatingRemainder(dividingBy: 1),
                    saturation: 0.75, brightness: 0.75)
            }
        }
        return .secondary
    }
}

/// 显示屏幕壁纸的视图
private struct DisplayWallpaperView: View {
    let display: Display
    let outline: Color
    
    var body: some View {
        ZStack {
            // 获取当前屏幕的壁纸
            if let wallpaper = getWallpaper(for: display) {
                Image(nsImage: wallpaper)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .clipped()
            } else {
                // 如果无法获取壁纸，显示渐变背景
                LinearGradient(
                    colors: [
                        outline.opacity(0.3),
                        outline.opacity(0.15)
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 2))
    }
    
    private func getWallpaper(for display: Display) -> NSImage? {
        let workspace = NSWorkspace.shared
        let screens = NSScreen.screens
        
        // 如果是本地屏幕，尝试获取对应屏幕的壁纸
        if display.local {
            // 遍历所有屏幕，尝试找到匹配的屏幕
            for screen in screens {
                // 尝试通过屏幕编号或位置匹配
                let screenFrame = screen.frame
                if abs(screenFrame.width - CGFloat(display.width)) < 10 &&
                   abs(screenFrame.height - CGFloat(display.height)) < 10 {
                    if let imageURL = workspace.desktopImageURL(for: screen),
                       let image = NSImage(contentsOf: imageURL) {
                        return image
                    }
                }
            }
            
            // 如果没有精确匹配，使用第一个屏幕的壁纸
            if let screen = screens.first,
               let imageURL = workspace.desktopImageURL(for: screen),
               let image = NSImage(contentsOf: imageURL) {
                return image
            }
        }
        
        return nil
    }
}
