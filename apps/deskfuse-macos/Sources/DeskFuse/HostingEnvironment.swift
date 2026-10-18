import SwiftUI

/// Copy only values needed across an AppKit hosting boundary. Retaining the
/// representable's entire EnvironmentValues also retains its PropertyList tracker;
/// repeated root updates can accumulate invalid tracked values indefinitely.
struct HostingEnvironment {
    let quietWorkspace: Bool
    let colorScheme: ColorScheme
    let controlSize: ControlSize
    let font: Font?
    let isEnabled: Bool
    let layoutDirection: LayoutDirection
    let locale: Locale
    let displayScale: CGFloat
    let dynamicTypeSize: DynamicTypeSize
    let scenePhase: ScenePhase

    init(_ environment: EnvironmentValues) {
        quietWorkspace = environment.quietWorkspace
        colorScheme = environment.colorScheme
        controlSize = environment.controlSize
        font = environment.font
        isEnabled = environment.isEnabled
        layoutDirection = environment.layoutDirection
        locale = environment.locale
        displayScale = environment.displayScale
        dynamicTypeSize = environment.dynamicTypeSize
        scenePhase = environment.scenePhase
    }

    // Read-only accessibility preferences are supplied by each native host.
    func wrap(_ content: some View) -> AnyView {
        AnyView(content
            .environment(\.quietWorkspace, quietWorkspace)
            .environment(\.colorScheme, colorScheme)
            .environment(\.controlSize, controlSize)
            .environment(\.font, font)
            .environment(\.isEnabled, isEnabled)
            .environment(\.layoutDirection, layoutDirection)
            .environment(\.locale, locale)
            .environment(\.displayScale, displayScale)
            .environment(\.dynamicTypeSize, dynamicTypeSize)
            .environment(\.scenePhase, scenePhase)
            .tint(DeskFuseBrand.accent)
            .buttonStyle(WorkspaceButtonStyle()))
    }
}
