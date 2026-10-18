import AppKit
import SwiftUI

struct MousePhotoAnglePicker: View {
    @Binding var side: Bool
    var body: some View {
        HStack(spacing: 4) {
            angle("顶视", side: false)
            angle("侧视", side: true)
        }.padding(4).background(.primary.opacity(0.05), in: Capsule())
    }
    private func angle(_ title: String, side value: Bool) -> some View {
        Button { side = value } label: {
            Text(title).font(.callout.weight(.medium))
                .foregroundStyle(side == value ? Color.white : Color.primary)
                .padding(.horizontal, 15).padding(.vertical, 7)
                .background(side == value ? Color.accentColor : Color.clear, in: Capsule())
                .contentShape(Capsule())
        }.buttonStyle(.plain).accessibilityLabel(title)
            .accessibilityAddTraits(side == value ? .isSelected : [])
    }
}

private enum MXMasterPhotos {
    static let top = load("mx-master-3-top")
    static let side = load("mx-master-3-side")
    static func load(_ name: String) -> NSImage? {
        Bundle.main.url(forResource: name, withExtension: "png", subdirectory: "Mice")
            .flatMap(NSImage.init(contentsOf:))
    }
}

struct MXMasterMouseEditor<Editor: View>: View {
    let controller: MouseButtonController
    let onSelection: () -> Void
    @ViewBuilder let actionEditor: (Int) -> Editor
    @State private var side = false
    @State private var editing: Int?

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
            Text("普通按键直接设置 · 手势键与模式键需检测到 HID++ 支持 · 横向滚轮保留原功能")
                .font(.caption).foregroundStyle(.secondary).padding(16)
        }
        .background(.background, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(.primary.opacity(0.06)).allowsHitTesting(false))
        .onChange(of: side) { _, _ in close() }
        .onDisappear { close() }
    }

    private var canvas: some View {
        GeometryReader { proxy in
            let width = side ? min(proxy.size.width - 40, 740) : proxy.size.height * 2400 / 2061
            let height = width * 2061 / 2400
            ZStack {
                LinearGradient(colors: [.primary.opacity(0.015), Color.accentColor.opacity(0.045)], startPoint: .topLeading, endPoint: .bottomTrailing)
                    .allowsHitTesting(false)
                if let photo = side ? MXMasterPhotos.side : MXMasterPhotos.top {
                    Image(nsImage: photo).resizable().frame(width: width, height: height)
                        .position(x: proxy.size.width / 2, y: proxy.size.height / 2).allowsHitTesting(false)
                }
                ForEach(side ? [4, 3, 195] : [0, 1, 2, 196], id: \.self) { button in
                    let point = anchor(button)
                    Button { onSelection(); editing = button } label: {
                        Circle().fill(editing == button ? Color.accentColor : Color.white)
                            .overlay(Circle().strokeBorder(Color.accentColor, lineWidth: 2).allowsHitTesting(false))
                            .overlay {
                                if controller.settings.bindings.contains(where: { $0.button == button }) {
                                    Image(systemName: "checkmark").font(.system(size: 10, weight: .bold)).foregroundStyle(Color.accentColor)
                                        .allowsHitTesting(false)
                                }
                            }.frame(width: 25, height: 25).shadow(color: .black.opacity(0.2), radius: 4, y: 2)
                    }.buttonStyle(.plain).accessibilityLabel(MouseSettings.buttonName(button))
                        .help(MouseSettings.buttonName(button))
                        .popover(isPresented: Binding(get: { editing == button }, set: { if !$0 && editing == button { close() } })) {
                            VStack(alignment: .leading, spacing: 14) {
                                HStack {
                                    Text(MouseSettings.buttonName(button)).font(.headline)
                                    Spacer()
                                    Button { close() } label: { Image(systemName: "xmark") }
                                        .buttonStyle(.plain).accessibilityLabel("关闭按键设置")
                                }
                                if button < 195 || controller.availableNativeButtons.contains(button) {
                                    actionEditor(button)
                                    Text(controller.saveStatus).font(.caption).foregroundStyle(.secondary)
                                    if button >= 195 {
                                        Text("设置动作会接管此扩展键；选择保持原始功能可恢复原行为。")
                                            .font(.caption).foregroundStyle(.secondary)
                                    }
                                } else {
                                    Text("尚未检测到此扩展键。请连接 MX Master 3，并允许输入监控权限。")
                                        .font(.callout).foregroundStyle(.secondary)
                                    Button("检测扩展键") { controller.beginDetection() }
                                    Text(controller.hidStatus).font(.caption).foregroundStyle(.secondary)
                                }
                            }.padding(20).frame(width: 300)
                        }
                        .position(x: (proxy.size.width - width) / 2 + point.x * width,
                                  y: (proxy.size.height - height) / 2 + point.y * height)
                }
            }.contentShape(Rectangle()).clipped()
        }.frame(height: 420)
    }
    private func close() {
        editing = nil; controller.clearCapture(); onSelection()
    }
    private func anchor(_ button: Int) -> CGPoint {
        switch button {
        case 0: CGPoint(x: 0.46, y: 0.16)
        case 1: CGPoint(x: 0.65, y: 0.18)
        case 2: CGPoint(x: 0.56, y: 0.23)
        case 196: CGPoint(x: 0.56, y: 0.40)
        case 4: CGPoint(x: 0.47, y: 0.485)
        case 3: CGPoint(x: 0.58, y: 0.445)
        default: CGPoint(x: 0.61, y: 0.685)
        }
    }
}
