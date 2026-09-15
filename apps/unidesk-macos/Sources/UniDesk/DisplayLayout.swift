import SwiftUI
import UniDeskEngine

/// Screen coordinates come from the engine; this canvas never invents a topology.
struct DisplayLayout: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    var onRemoveDevice: ((String) -> Void)? = nil
    var editingDisabled = false
    @State private var dragging: String?
    @State private var delta: CGSize = .zero
    @State private var dragDisplays: [Display] = []
    @State private var dragScale: CGFloat = 1
    @State private var snapped = false
    @Environment(\.colorSchemeContrast) private var contrast

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
                    // No device silhouette or wallpaper: adjoining screen edges stay visible.
                    RoundedRectangle(cornerRadius: 2)
                        .fill(outline.opacity(dragging == display.deviceId ? 0.09 : 0.025))
                        .overlay {
                            RoundedRectangle(cornerRadius: 2)
                                .strokeBorder(outline, style: StrokeStyle(
                                    lineWidth: contrast == .increased || dragging == display.deviceId ? 3 : 2,
                                    dash: display.ready ? [] : [6, 4]
                                ))
                        }
                        .overlay {
                            VStack(spacing: 6) {
                                Text(String(format: "%02d", index + 1))
                                    .font(.system(size: 22, weight: .semibold, design: .rounded))
                                    .foregroundStyle(outline)
                                Text(display.name + (display.local ? " · 本机" : "")).font(.system(size: 12, weight: .semibold))
                                    .lineLimit(2).multilineTextAlignment(.center)
                                Text(display.layoutSizeLabel)
                                    .font(.system(size: 10)).monospacedDigit().foregroundStyle(.secondary)
                                    .lineLimit(1).minimumScaleFactor(0.7)
                            }
                            .padding(8)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .allowsHitTesting(false)
                        }
                        .frame(width: display.width * scale, height: display.height * scale)
                        .contentShape(Rectangle())
                        .offset(
                            x: offsetX + (display.x - minX) * scale + (dragging == display.deviceId ? delta.width : 0),
                            y: offsetY + (display.y - minY) * scale + (dragging == display.deviceId ? delta.height : 0)
                        )
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
                        .contextMenu {
                            if !display.local, let onRemoveDevice {
                                Button("移除此设备的屏幕", systemImage: "minus.circle") {
                                    onRemoveDevice(display.deviceId)
                                }.disabled(editingDisabled)
                            }
                        }
                        .help("\(display.name)\(display.local ? " · 本机" : "") · \(display.ready ? "已就绪" : "未就绪")\n拖动自动吸附边缘；同一设备的屏幕整体移动，松手贴合相邻屏幕。")
                        .accessibilityLabel("屏幕 \(index + 1)，\(display.name)，\(display.layoutSizeLabel)，\(display.ready ? "已就绪" : "未就绪")")
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
