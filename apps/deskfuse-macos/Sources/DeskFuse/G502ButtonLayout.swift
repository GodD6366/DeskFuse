import Foundation

/// Physical locations are independent of the firmware's Button Spy bit order.
enum G502ButtonPosition: String, CaseIterable, Identifiable, Codable {
    case left, right, middle, tiltLeft, tiltRight, dpiUp, dpiDown, top, back, forward, thumb
    var genericButton: Int? {
        switch self {
        case .left: 0
        case .right: 1
        case .middle: 2
        case .back: 3
        case .forward: 4
        default: nil
        }
    }
    var id: String { rawValue }
    var sideView: Bool { [.back, .forward, .thumb].contains(self) }
    var title: String {
        switch self {
        case .left: "左键"
        case .right: "右键"
        case .middle: "滚轮按下"
        case .tiltLeft: "滚轮左拨"
        case .tiltRight: "滚轮右拨"
        case .dpiUp: "DPI +"
        case .dpiDown: "DPI −"
        case .top: "顶部 G9"
        case .back: "后退侧键"
        case .forward: "前进侧键"
        case .thumb: "拇指键"
        }
    }
    var detail: String {
        switch self {
        case .left: "G1 · 食指主键"
        case .right: "G2 · 右侧主键"
        case .middle: "G3 · 向下按压滚轮"
        case .tiltLeft: "向左轻拨滚轮"
        case .tiltRight: "向右轻拨滚轮"
        case .dpiUp: "G8 · 左键旁，靠前的小键"
        case .dpiDown: "G7 · 左键旁，靠后的小键"
        case .top: "G9 · 滚轮模式键后方"
        case .back: "G4 · 拇指上方，靠后的小键"
        case .forward: "G5 · 拇指上方，靠前的小键"
        case .thumb: "G6 · 拇指前方的 DPI Shift 键"
        }
    }
}

struct G502ButtonLocations: Codable, Equatable {
    private(set) var buttons: [String: Int] = [:]

    init(json: String = "{}") {
        guard let data = json.data(using: .utf8),
              let decoded = try? JSONDecoder().decode([String: Int].self, from: data) else { return }
        // Ignore corrupt/unknown entries and keep one physical location per event.
        var used: Set<Int> = []
        for position in G502ButtonPosition.allCases {
            if let button = decoded[position.rawValue], (208...218).contains(button), used.insert(button).inserted {
                buttons[position.rawValue] = button
            }
        }
    }
    func button(at position: G502ButtonPosition) -> Int? { buttons[position.rawValue] }
    func position(for button: Int) -> G502ButtonPosition? {
        G502ButtonPosition.allCases.first { buttons[$0.rawValue] == button }
    }
    mutating func locate(_ button: Int, at position: G502ButtonPosition) {
        guard (208...218).contains(button) else { return }
        buttons = buttons.filter { $0.value != button }
        buttons[position.rawValue] = button
    }
    var json: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        return (try? encoder.encode(buttons)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }
}
