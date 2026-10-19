import SwiftUI
import AppKit
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
    var onLayoutInteraction: (() -> Void)? = nil
    @State private var dragging: String?
    @State private var delta: CGSize = .zero
    @State private var dragDisplays: [Display] = []
    @State private var hoveredDisplay: String?
    @State private var dragViewport: DisplayLayoutViewport?
    @State private var pendingMove: UUID?
    @State private var snapped = false
    @State private var snapGuideX: CGFloat?
    @State private var snapGuideY: CGFloat?
    // The coordinate space belongs to the stationary canvas, never a moving tile.
    static let canvasSpace = "DeskFuse.DisplayLayout.canvas"
    static var moveGesture: DragGesture { DragGesture(minimumDistance: 4, coordinateSpace: .named(canvasSpace)) }
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GeometryReader { geometry in
            let displays = dragging == nil ? snapshot.input.displays.sorted { $0.layoutID < $1.layoutID } : dragDisplays
            let viewport = dragViewport ?? DisplayLayoutViewport(displays: displays, size: geometry.size)
            let scale = viewport.scale

            ZStack(alignment: .topLeading) {
                // 底色由外层 systemSettingsBlock 提供，这里只画点阵：
                // 画布再铺一层材质会和外层分组叠成两层灰底。
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
                    let offline = isOffline(display)
                    let position = viewport.position(of: display, translation: dragging == display.deviceId ? delta : .zero)
                    ZStack(alignment: .bottomLeading) {
                        DisplayWallpaperView(display: display, localWallpaper: store.localDisplayWallpapers[display.id],
                            outline: outline, isOffline: offline, size: CGSize(
                                width: display.width * scale, height: display.height * scale))
                        if offline {
                            Color.black.opacity(contrast == .increased ? 0.34 : 0.24)
                                .allowsHitTesting(false)
                            Label(display.ready ? "已断开" : "等待连接", systemImage: "wifi.slash")
                                .font(DFType.caption.weight(.semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 8).padding(.vertical, 5)
                                .background(.black.opacity(0.48), in: Capsule())
                                .padding(8)
                                .allowsHitTesting(false)
                        }
                        RoundedRectangle(cornerRadius: DFRadius.small)
                            .fill(outline.opacity(dragging == display.deviceId ? 0.09 : (display.ready ? 0.025 : 0.06)))
                            .overlay {
                                RoundedRectangle(cornerRadius: DFRadius.small)
                                    .strokeBorder(outline, style: StrokeStyle(
                                        lineWidth: contrast == .increased || dragging == display.deviceId ? 3 : 2,
                                        dash: display.ready ? [] : [6, 4]
                                    ))
                            }
                            // The interaction surface must fill the tile even
                            // when no wallpaper image has arrived.
                            .frame(width: display.width * scale, height: display.height * scale)
                            .contentShape(Rectangle())
                            .gesture(Self.moveGesture
                                .onChanged { value in
                                    guard !editingDisabled, pendingMove == nil else { return }
                                    if dragging == nil {
                                        dragDisplays = displays
                                        dragViewport = viewport
                                        dragging = display.deviceId
                                        onLayoutInteraction?()
                                        NSCursor.closedHand.set()
                                    }
                                    guard dragging == display.deviceId else { return }
                                    updateDrag(value.translation, device: display.deviceId, attach: false)
                                }
                                .onEnded { value in
                                    NSCursor.openHand.set()
                                    guard dragging == display.deviceId, pendingMove == nil else { return }
                                    guard !editingDisabled,
                                          updateDrag(value.translation, device: display.deviceId, attach: true),
                                          let anchor = dragDisplays.first(where: { $0.layoutID == display.layoutID }),
                                          let dragViewport else { resetDrag(); return }
                                    let movement = dragViewport.logicalTranslation(delta)
                                    let token = UUID()
                                    pendingMove = token
                                    store.send(.moveInputDevice(deviceId: display.deviceId, displayId: display.id,
                                        x: anchor.x + movement.width, y: anchor.y + movement.height), failed: { _ in
                                            guard pendingMove == token else { return }
                                            resetDrag()
                                        })
                                    confirmMove(using: snapshot.input.displays)
                                })
                        DisplayInfoBar(
                            name: displayDeviceName(display),
                            resolution: display.layoutSizeLabel,
                            symbol: displayDeviceSymbol(display),
                            status: displayStatus(display),
                            expanded: hoveredDisplay == display.layoutID,
                            compact: display.width * scale < 180 || display.height * scale < 96
                        )
                        .padding(7)
                        .allowsHitTesting(false)
                        if hoveredDisplay == display.layoutID && dragging == nil {
                            VStack(alignment: .leading, spacing: 3) {
                                Text("屏幕 \(String(format: "%02d", index + 1)) · \(display.local ? "本机" : "远端")")
                                    .font(DFType.caption.weight(.semibold))
                                Text(display.name).font(DFType.detail.weight(.medium))
                                Text("\(display.layoutSizeLabel) · \(display.ready ? "已就绪" : "未就绪")")
                                    .font(DFType.caption)
                            }
                            .foregroundStyle(.primary)
                            .padding(.horizontal, 10).padding(.vertical, 7)
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DFRadius.control))
                            .overlay(RoundedRectangle(cornerRadius: DFRadius.control).strokeBorder(outline.opacity(0.35)))
                            .padding(8)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                            .allowsHitTesting(false)
                        }
                        if canReconnect {
                            DisplayReconnectButton(
                                color: outline,
                                busy: reconnecting,
                                reduceMotion: reduceMotion,
                                action: { onReconnectDevice?(display.deviceId) }
                            )
                            .disabled(editingDisabled)
                            .padding(10)
                            .frame(width: display.width * scale, height: display.height * scale, alignment: .center)
                        }
                    }
                    .frame(width: display.width * scale, height: display.height * scale)
                    .clipShape(RoundedRectangle(cornerRadius: DFRadius.small))
                    .contentShape(Rectangle())
                    .scaleEffect(hoveredDisplay == display.layoutID && dragging == nil ? 1.01 : 1)
                    .shadow(color: .black.opacity(hoveredDisplay == display.layoutID ? 0.18 : 0.08),
                            radius: hoveredDisplay == display.layoutID ? 8 : 3, y: 2)
                    .onHover { inside in
                        if inside {
                            hoveredDisplay = display.layoutID
                            (dragging == nil ? NSCursor.openHand : NSCursor.closedHand).set()
                        } else {
                            if hoveredDisplay == display.layoutID { hoveredDisplay = nil }
                            NSCursor.arrow.set()
                        }
                    }
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
                    .accessibilityElement(children: canReconnect ? .contain : .ignore)
                    .accessibilityLabel("屏幕 \(index + 1)，\(display.name)，\(display.layoutSizeLabel)，\(display.ready ? "已就绪" : "未就绪")")
                    .accessibilityHint(canReconnect ? "未正式连接。可重新连接并通知对端准备键鼠共享。" : "")
                    .offset(x: position.x, y: position.y)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
            .coordinateSpace(name: Self.canvasSpace)
            .transaction { transaction in
                if dragging != nil { transaction.animation = nil; transaction.disablesAnimations = true }
            }
            .overlay(alignment: .topLeading) {
                if let snapGuideX, dragging != nil {
                    Rectangle()
                        .fill(DFPalette.accent.opacity(0.78))
                        .frame(width: 1, height: geometry.size.height)
                        .offset(x: snapGuideX)
                        .allowsHitTesting(false)
                }
                if let snapGuideY, dragging != nil {
                    Rectangle()
                        .fill(DFPalette.accent.opacity(0.78))
                        .frame(width: geometry.size.width, height: 1)
                        .offset(y: snapGuideY)
                        .allowsHitTesting(false)
                }
                if dragging != nil {
                    Label(pendingMove != nil ? "正在保存布局…" : snapped ? "已吸附边缘" : "松手自动贴合相邻屏幕", systemImage: "rectangle.connected.to.line.below")
                        .font(DFType.caption).padding(8).background(.regularMaterial, in: Capsule()).padding(12)
                        .allowsHitTesting(false)
                }
            }
            // 只用来裁掉拖动溢出，圆角与外层分组对齐；不再单独铺底或描边。
            .clipShape(RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous))
        }
        .onChange(of: snapshot.input.displays) { _, displays in confirmMove(using: displays) }
        .onDisappear { resetDrag() }
        .task(id: pendingMove) {
            guard let token = pendingMove else { return }
            do { try await Task.sleep(for: .seconds(3)) } catch { return }
            guard pendingMove == token else { return }
            resetDrag()
            store.error = "屏幕布局更新尚未确认，请重试"
        }
    }

    private func resetDrag() {
        dragging = nil; dragDisplays = []; dragViewport = nil
        delta = .zero; snapped = false; snapGuideX = nil; snapGuideY = nil; pendingMove = nil
    }

    private func confirmMove(using displays: [Display]) {
        guard pendingMove != nil, let dragging, let dragViewport else { return }
        if DisplayLayoutConfirmation.matches(before: dragDisplays, after: displays, deviceID: dragging,
            translation: dragViewport.logicalTranslation(delta)) {
            resetDrag()
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

    private func deviceName(for display: Display) -> String {
        if display.local { return snapshot.localName }
        return snapshot.devices.first(where: { $0.id == display.deviceId })?.name
            ?? "设备 \(display.deviceId.prefix(8))"
    }

    private func isOffline(_ display: Display) -> Bool {
        guard !display.local else { return !display.ready }
        let connected = snapshot.devices.first(where: { $0.id == display.deviceId })?.connected == true
        return !connected || !display.ready
    }

    private func displayDeviceName(_ display: Display) -> String {
        deviceName(for: display)
    }

    private func displayDeviceSymbol(_ display: Display) -> String {
        DeviceProductIconResolver.fallbackSymbol(for: deviceName(for: display))
    }

    private func displayStatus(_ display: Display) -> DisplayStatus {
        if display.local {
            return display.ready ? .local : .offline
        }
        let connected = snapshot.devices.first(where: { $0.id == display.deviceId })?.connected == true
        return connected && display.ready ? .connected : .offline
    }

    @discardableResult
    private func updateDrag(_ translation: CGSize, device: String, attach: Bool) -> Bool {
        guard let dragViewport else { return false }
        let movement = dragViewport.logicalTranslation(translation)
        do {
            let offset = try previewDisplayMove(displays: dragDisplays, deviceId: device,
                deltaX: movement.width, deltaY: movement.height,
                snapDistance: 14 / dragViewport.scale, attach: attach)
            delta = CGSize(width: offset.x * dragViewport.scale, height: offset.y * dragViewport.scale)
            snapped = abs(delta.width - translation.width) > 0.1 || abs(delta.height - translation.height) > 0.1
            updateSnapGuides(device: device, offset: CGSize(width: offset.x, height: offset.y))
            return true
        } catch {
            store.error = error.userFacingMessage
            return false
        }
    }

    private func updateSnapGuides(device: String, offset: CGSize) {
        guard snapped, let dragViewport else {
            snapGuideX = nil
            snapGuideY = nil
            return
        }
        let moving = dragDisplays.filter { $0.deviceId == device }
        let stationary = dragDisplays.filter { $0.deviceId != device }
        let tolerance = 0.5

        let verticalSeam = moving.lazy.compactMap { moved -> Double? in
            let left = moved.x + offset.width
            let right = left + moved.width
            return stationary.lazy.compactMap { fixed -> Double? in
                if abs(left - (fixed.x + fixed.width)) < tolerance { return left }
                if abs(right - fixed.x) < tolerance { return right }
                return nil
            }.first
        }.first
        let horizontalSeam = moving.lazy.compactMap { moved -> Double? in
            let top = moved.y + offset.height
            let bottom = top + moved.height
            return stationary.lazy.compactMap { fixed -> Double? in
                if abs(top - (fixed.y + fixed.height)) < tolerance { return top }
                if abs(bottom - fixed.y) < tolerance { return bottom }
                return nil
            }.first
        }.first

        snapGuideX = verticalSeam.map {
            dragViewport.origin.x + ($0 - dragViewport.layoutOrigin.x) * dragViewport.scale
        }
        snapGuideY = horizontalSeam.map {
            dragViewport.origin.y + ($0 - dragViewport.layoutOrigin.y) * dragViewport.scale
        }
    }


}

extension Display {
    var layoutID: String { deviceId + ":" + id }
    // The topology contains logical desktop bounds, not physical panel inches.
    var layoutSizeLabel: String { "\(Int(width)) × \(Int(height)) pt" }
}

private enum DisplayStatus {
    case local, connected, offline

    var color: Color {
        switch self {
        case .local: DFPalette.accent
        case .connected: DFPalette.success
        case .offline: Color.secondary
        }
    }

    var label: String {
        switch self {
        case .local: "本机"
        case .connected: "已连接"
        case .offline: "已断开"
        }
    }
}

private struct DisplayInfoBar: View {
    let name: String
    let resolution: String
    let symbol: String
    let status: DisplayStatus
    let expanded: Bool
    let compact: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol)
                .font(DFType.caption.weight(.medium))
            Circle()
                .fill(status.color)
                .frame(width: 6, height: 6)
            if !compact || expanded {
                VStack(alignment: .leading, spacing: 0) {
                    Text(name).lineLimit(1)
                    Text(resolution).font(DFType.caption).opacity(0.78).lineLimit(1)
                }
            } else {
                Text(name).lineLimit(1)
            }
            if expanded && !compact {
                Text(status.label).opacity(0.84).lineLimit(1)
            }
        }
        .font(DFType.caption.weight(.semibold))
        .foregroundStyle(.white)
        .padding(.horizontal, 8).padding(.vertical, 5)
        .background(.black.opacity(0.58), in: Capsule())
        .overlay { Capsule().strokeBorder(.white.opacity(0.18), lineWidth: 0.5) }
        .animation(.easeOut(duration: 0.15), value: expanded)
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
            .font(DFType.detail.weight(.semibold))
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
    private static let colors: [Color] = DFPalette.identity
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

/// Local desktop image or a bounded JPEG preview sent by the remote Mac.
private struct DisplayWallpaperView: View {
    let display: Display
    let localWallpaper: NSImage?
    let outline: Color
    let isOffline: Bool
    let size: CGSize
    @State private var remoteWallpaper: NSImage?

    var body: some View {
        Group {
            if let wallpaper = display.local ? localWallpaper : remoteWallpaper {
                Image(nsImage: wallpaper)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: size.width, height: size.height)
            } else {
                LinearGradient(
                    colors: [outline.opacity(0.3), outline.opacity(0.15)],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            }
        }
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: DFRadius.small))
        .saturation(isOffline ? 0 : 1)
        .allowsHitTesting(false)
        .task(id: display.wallpaperJpeg?.hashValue) {
            remoteWallpaper = display.wallpaperJpeg.flatMap(NSImage.init(data:))
        }
    }
}
