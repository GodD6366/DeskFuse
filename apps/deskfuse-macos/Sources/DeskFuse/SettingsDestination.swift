/// Settings navigation is shared by the sidebar, commands and in-app links.
struct SettingsDestination: Equatable {
    var category: SettingsCategory = .general

    init() {}

    init(tab: String?, previous: Self = Self()) {
        category = tab.map { SettingsCategory(legacy: $0) ?? .general } ?? previous.category
    }
}
