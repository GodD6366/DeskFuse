import Foundation
import Observation

/// Transient presentation only: dismissing a notice must not mark the engine healthy.
@MainActor @Observable final class ErrorNotice {
    private(set) var message: String?
    private var localError: String?
    private var runtimeError: String?
    private var dismissedLocalError: String?
    private var dismissedRuntimeError: String?
    @ObservationIgnored private var dismissalTask: Task<Void, Never>?
    private let displayDuration: Duration

    init(displayDuration: Duration = .seconds(8)) {
        self.displayDuration = displayDuration
    }

    func updateLocalError(_ error: String?) {
        localError = error
        // An explicit local error assignment is a new failure, even if its text repeats.
        dismissedLocalError = nil
        refresh(restartTimer: true)
    }

    func updateRuntimeError(_ error: String?) {
        // Repeated snapshots are observations, not new failures. They must neither
        // restart the timer nor resurrect an already dismissed sticky engine error.
        guard runtimeError != error else { return }
        runtimeError = error
        dismissedRuntimeError = nil
        refresh()
    }

    func dismiss() {
        dismissalTask?.cancel()
        dismissalTask = nil
        dismissedLocalError = localError
        dismissedRuntimeError = runtimeError
        message = nil
    }

    private func refresh(restartTimer: Bool = false) {
        let next = (localError != dismissedLocalError ? localError : nil)
            ?? (runtimeError != dismissedRuntimeError ? runtimeError : nil)
        guard next != message || restartTimer else { return }
        dismissalTask?.cancel()
        dismissalTask = nil
        message = next
        guard next != nil else { return }
        let duration = displayDuration
        dismissalTask = Task { [weak self] in
            do { try await Task.sleep(for: duration) }
            catch { return }
            guard !Task.isCancelled else { return }
            self?.dismiss()
        }
    }
}
