import UniDeskEngine

/// A value projected from the current engine snapshot, never independently saved.
/// Peer heartbeats, clipboard history and input motion must not invalidate tabs.
struct SettingsPresentation: Equatable {
    let clipboardEnabled: Bool
    let debugLoggingEnabled: Bool
    let permissions: [Permission]
    let files: FileSettings?
    let cache: CacheStatus?
    let update: UpdateState?
    let canConfigureInput: Bool
    let inputSettings: InputSettings?

    init(_ snapshot: EngineSnapshot?) {
        clipboardEnabled = snapshot?.clipboardEnabled ?? true
        debugLoggingEnabled = snapshot?.debugLoggingEnabled ?? false
        permissions = snapshot?.permissions ?? []
        files = snapshot?.files
        cache = snapshot?.cache
        update = snapshot?.update
        canConfigureInput = snapshot?.input.canConfigure ?? false
        inputSettings = snapshot?.input.settings
    }
}
