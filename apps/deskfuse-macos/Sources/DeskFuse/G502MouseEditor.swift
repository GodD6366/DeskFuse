import SwiftUI
import AppKit

private enum G502Photos {
    static let top = load("g502-lightspeed-top")
    static let side = load("g502-lightspeed-side")
    private static func load(_ name: String) -> NSImage? {
        Bundle.main.url(forResource: name, withExtension: "png", subdirectory: "Mice")
            .flatMap(NSImage.init(contentsOf:))
    }
}

struct G502MouseEditor<ActionEditor: View>: View {
    let controller: MouseButtonController
    let onSelection: () -> Void
    @ViewBuilder let actionEditor: (Int) -> ActionEditor
    @AppStorage("mouse.g502.physicalLocations.v1") private var savedLocations = "{}"
    @State private var selection: G502ButtonPosition = .middle
    @State private var side = false
    @State private var editing: G502ButtonPosition?
    @State private var locating: G502ButtonPosition?
    @State private var candidate: Int?
    private var locations: G502ButtonLocations { G502ButtonLocations(json: savedLocations) }
    private var wheelSelected: Bool { [.middle, .tiltLeft, .tiltRight].contains(selection) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Label("点击按键，直接分配功能", systemImage: "cursorarrow")
                    .font(.callout).foregroundStyle(.secondary)
                Spacer()
                MousePhotoAnglePicker(side: $side)
            }.padding(18).zIndex(1)
            canvas
            Divider()
            HStack(spacing: 8) {
                Image(systemName: "checkmark.shield")
                Text("保留原生功能，只附加快捷键")
                Spacer()
                Text("\(locations.buttons.count) / 11 个位置已确认")
            }.font(.caption).foregroundStyle(.secondary).padding(16)
        }
        .background(.background, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.primary.opacity(0.06)).allowsHitTesting(false))
        .onChange(of: side) { _, value in
            editing = nil
            select(value ? .back : .middle)
        }
        .onChange(of: controller.captureReleased) { _, released in
            guard released, let position = locating, let button = controller.lastButton,
                  (208...218).contains(button) else { return }
            _ = position
            candidate = button
        }
        .onDisappear { cancelCapture() }
    }

    private var canvas: some View {
        GeometryReader { proxy in
            let size = proxy.size
            let scale = side ? min(size.width - 40, 740) / 1384 : (size.height - 35) / 1038
            let photoSize = CGSize(width: 1384 * scale, height: 1038 * scale)
            ZStack {
                LinearGradient(colors: [Color.primary.opacity(0.015), Color.accentColor.opacity(0.045)], startPoint: .topLeading, endPoint: .bottomTrailing).allowsHitTesting(false)
                if let photo = side ? G502Photos.side : G502Photos.top {
                    Image(nsImage: photo).resizable().frame(width: photoSize.width, height: photoSize.height)
                        .position(x: size.width / 2, y: size.height / 2).allowsHitTesting(false)
                }
                ForEach(side ? [.forward, .back, .thumb] : [.left, .right, .middle, .dpiUp, .dpiDown, .top], id: \.self) { (position: G502ButtonPosition) in
                    let point = anchor(position)
                    let x = (size.width - photoSize.width) / 2 + point.x * scale
                    let y = (size.height - photoSize.height) / 2 + point.y * scale
                    let active = editing == position
                    Button { select(position); editing = position } label: {
                        ZStack {
                            Circle().fill(active ? Color.accentColor : Color.white)
                            Circle().strokeBorder(active ? Color.white : Color.accentColor, lineWidth: 2)
                            if active { Circle().fill(.white).frame(width: 6, height: 6) }
                            else if locations.button(at: position) != nil {
                                Image(systemName: "checkmark").font(.system(size: 9, weight: .bold)).foregroundStyle(Color.accentColor)
                            }
                        }.frame(width: 23, height: 23)
                            .shadow(color: .black.opacity(0.22), radius: 4, y: 2)
                            .contentShape(Circle())
                    }.buttonStyle(.plain)
                        .popover(isPresented: Binding(get: { editing == position }, set: { presented in
                            if !presented && editing == position {
                                editing = nil
                                cancelCapture()
                                onSelection()
                            }
                        }), arrowEdge: .trailing) {
                            inspector.frame(width: 280).padding(20)
                        }
                        .position(x: x, y: y)
                        .help("\(position.title) · 点击设置功能").accessibilityLabel(position.title)
                }
                VStack {
                    Spacer()
                    Text(side ? "拇指侧 · 3 个按键" : "滚轮支持按下、左拨与右拨")
                        .font(.caption).foregroundStyle(.secondary).padding(10)
                        .background(.regularMaterial, in: Capsule()).padding(.bottom, 12).allowsHitTesting(false)
                }
            }.contentShape(Rectangle()).clipped()
        }.frame(height: 420)
    }

    private var inspector: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 7) {
                HStack {
                    Text(selection.title).font(.system(size: 18, weight: .semibold))
                    Spacer()
                    Button {
                        editing = nil; cancelCapture(); onSelection()
                    } label: { Image(systemName: "xmark") }
                        .buttonStyle(.plain).foregroundStyle(.secondary).accessibilityLabel("关闭按键设置")
                }
                Text(selection.detail).font(.caption).foregroundStyle(.secondary)
            }
            if wheelSelected {
                Picker("滚轮动作", selection: Binding(get: { selection }, set: { select($0) })) {
                    Text("按下").tag(G502ButtonPosition.middle)
                    Text("左拨").tag(G502ButtonPosition.tiltLeft)
                    Text("右拨").tag(G502ButtonPosition.tiltRight)
                }.pickerStyle(.segmented).labelsHidden()
            }
            Divider()
            if let button = locations.button(at: selection), locating == nil {
                Text("附加动作").font(.caption).foregroundStyle(.secondary)
                actionEditor(button)
                    .padding(10).background(Color.accentColor.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
                Button("重新确认位置") { startCapture() }.buttonStyle(.link).font(.caption)
            } else if let candidate, let position = locating {
                Label("已收到按键", systemImage: "checkmark.circle").foregroundStyle(Color.accentColor)
                Text("刚才按下的是「\(position.title)」吗？确认后才会记住位置。")
                    .font(.callout).foregroundStyle(.secondary)
                Button("确认这个位置") {
                    var updated = locations
                    updated.locate(candidate, at: position)
                    savedLocations = updated.json
                    cancelCapture()
                }.buttonStyle(.borderedProminent)
                Button("重新识别") { startCapture() }
                Button("取消") { cancelCapture() }.buttonStyle(.link)
            } else if locating != nil {
                Label(controller.captureReleased ? "请重试识别" : controller.detecting ? "正在监听…" : controller.lastButton != nil ? "请松开按键" : "未收到按键", systemImage: "waveform")
                    .foregroundStyle(Color.accentColor)
                Text("按下并松开「\(selection.title)」。只确认位置，不改变鼠标原有设置。")
                    .font(.callout).foregroundStyle(.secondary)
                HStack {
                    Button("重新监听") { startCapture() }
                    Button("取消") { cancelCapture() }
                }
            } else {
                Text("第一次，确认一下位置")
                    .font(.system(size: 13, weight: .medium))
                Text("按一下对应的实体键，让 DeskFuse 记住它。已有快捷键不会丢失。")
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                Button("识别这个位置", systemImage: "cursorarrow.click") { startCapture() }
                    .buttonStyle(.borderedProminent)
            }
        }.fixedSize(horizontal: false, vertical: true)
    }

    private func select(_ position: G502ButtonPosition) {
        cancelCapture(); onSelection(); selection = position
    }
    private func startCapture() {
        onSelection(); candidate = nil; locating = selection; controller.beginDetection(g502: true)
    }
    private func cancelCapture() {
        if locating != nil { controller.endDetection(); controller.clearCapture() }
        locating = nil; candidate = nil
    }
    private func anchor(_ position: G502ButtonPosition) -> CGPoint {
        switch position {
        case .left: CGPoint(x: 615, y: 190)
        case .right: CGPoint(x: 850, y: 205)
        case .middle, .tiltLeft, .tiltRight: CGPoint(x: 735, y: 270)
        case .dpiUp: CGPoint(x: 515, y: 240)
        case .dpiDown: CGPoint(x: 515, y: 325)
        case .top: CGPoint(x: 740, y: 490)
        case .forward: CGPoint(x: 630, y: 515)
        case .back: CGPoint(x: 840, y: 480)
        case .thumb: CGPoint(x: 540, y: 620)
        }
    }
}
