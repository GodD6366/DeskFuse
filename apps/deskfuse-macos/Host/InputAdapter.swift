import Foundation

/// Native input posting stays in the signed input subprocess. The app host only
/// forwards lease commands through the collaboration API.
final class MacOSInputExecution {
    func releaseAll() {}
}
