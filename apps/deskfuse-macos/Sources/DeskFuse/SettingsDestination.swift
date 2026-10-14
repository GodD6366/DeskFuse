/// Settings navigation is shared by the sidebar, commands and deep links.
struct SettingsDestination: Equatable {
    var category: SettingsCategory = .general
    var clipboardSegment = "history"

    init() {}

    init(tab: String?, clipboardSegment: String?, previous: Self = Self()) {
        category = tab.map { SettingsCategory(rawValue: $0) ?? .general } ?? previous.category
        self.clipboardSegment = clipboardSegment.map {
            ["history", "workflows"].contains($0) ? $0 : "history"
        } ?? previous.clipboardSegment
    }
}
