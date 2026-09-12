import AppKit
import Foundation
import Observation
import UniDeskEngine

/// Observable projections only. Rust owns definitions, processing and run state.
@MainActor @Observable final class ImageWorkflowStore {
    static let shared = ImageWorkflowStore()
    private(set) var definitions: [WorkflowDefinition] = []
    private(set) var runs: [ImageWorkflowRun] = []
    private(set) var error: String?
    @ObservationIgnored private var engine: Engine?
    @ObservationIgnored private var reloadRevision = 0

    /// Root startup hook: `Task { await ImageWorkflowStore.shared.reload(engine: engine) }`.
    func reload(engine: Engine) async {
        self.engine = engine
        reloadRevision += 1
        let requested = reloadRevision
        do {
            let values = try await Task.detached { try engine.imageWorkflows() }.value
            guard requested == reloadRevision else { return }
            definitions = values.map(WorkflowDefinition.init(native:)); error = nil
            UniDeskImageAppShortcuts.updateAppShortcutParameters()
        } catch { if requested == reloadRevision { self.error = error.userFacingMessage } }
    }
    private func readyEngine() throws -> Engine {
        guard let engine = engine ?? AppStore.shared.engine else { throw WorkflowExecutionError(message: "引擎尚未就绪") }
        return engine
    }
    func save(_ draft: WorkflowDefinition) async throws {
        let engine = try readyEngine(); let definition = draft.native
        try await Task.detached { try engine.saveImageWorkflow(workflow: definition) }.value
        await reload(engine: engine)
        if let error { throw WorkflowExecutionError(message: error) }
    }
    func delete(_ id: String) async throws {
        let engine = try readyEngine()
        try await Task.detached { try engine.deleteImageWorkflow(id: id) }.value
        await reload(engine: engine)
        if let error { throw WorkflowExecutionError(message: error) }
    }
    func systemShortcuts() async throws -> [SystemShortcut] {
        let engine = try readyEngine()
        return try await Task.detached { try engine.systemShortcuts() }.value
    }
    func run(workflowId: String, input: WorkflowInput, output: WorkflowOutputMode, targetPid: Int32 = 0) async throws -> ImageWorkflowRun {
        let engine = try readyEngine()
        error = nil
        try Task.checkCancellation()
        let id = try await Task.detached {
            try engine.runImageWorkflow(workflowId: workflowId, input: input, output: output, targetPid: targetPid)
        }.value
        return try await withTaskCancellationHandler {
            do {
                while true {
                    try Task.checkCancellation()
                    let state = try await Task.detached { try engine.imageWorkflowRun(runId: id) }.value
                    record(state)
                    switch state.phase {
                    case "completed": return state
                    case "failed": throw WorkflowExecutionError(message: state.message)
                    case "cancelled": throw CancellationError()
                    default: try await Task.sleep(for: .milliseconds(200))
                    }
                }
            } catch {
                if error is CancellationError {
                    try? await Task.detached { try engine.cancelImageWorkflow(runId: id) }.value
                    if let state = try? await Task.detached(operation: { try engine.imageWorkflowRun(runId: id) }).value { record(state) }
                } else { self.error = error.userFacingMessage }
                throw error
            }
        } onCancel: {
            Task.detached { try? engine.cancelImageWorkflow(runId: id) }
        }
    }
    func cancel(_ id: String) {
        Task {
            do {
                let engine = try readyEngine()
                try await Task.detached { try engine.cancelImageWorkflow(runId: id) }.value
            } catch { self.error = error.userFacingMessage }
        }
    }
    /// Includes jobs started by the Rust global shortcut listener, not just UI callers.
    func consume(_ run: ImageWorkflowRun) { record(run) }
    private func record(_ run: ImageWorkflowRun) {
        if let index = runs.firstIndex(where: { $0.id == run.id }) {
            // Polls and pushed events may cross. Terminal state must not regress.
            guard runs[index].phase == "running" || runs[index].phase == run.phase else { return }
            if run.phase == "running" && run.stepIndex < runs[index].stepIndex { return }
            runs[index] = run
        }
        else { runs.insert(run, at: 0) }
        let active = runs.filter { $0.phase == "running" }
        let recent = runs.filter { $0.phase != "running" }.prefix(5)
        runs = active + recent
    }
}
private struct WorkflowExecutionError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

extension AppStore {
    var imageWorkflowDefinitions: [WorkflowDefinition] { ImageWorkflowStore.shared.definitions }
    func saveImageWorkflow(_ draft: WorkflowDefinition, onApplied: (() -> Void)? = nil) {
        error = nil
        Task {
            do { try await ImageWorkflowStore.shared.save(draft); onApplied?() }
            catch { self.error = error.userFacingMessage }
        }
    }
    func deleteImageWorkflow(_ id: String) {
        error = nil
        Task {
            do { try await ImageWorkflowStore.shared.delete(id) }
            catch { self.error = error.userFacingMessage }
        }
    }
    func runImageWorkflow(_ id: String, historyID: UInt64? = nil) {
        error = nil
        let target = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0
        Task {
            do {
                _ = try await ImageWorkflowStore.shared.run(workflowId: id,
                    input: historyID.map { .history(id: $0) } ?? .clipboard,
                    output: historyID == nil ? .configured : .copy, targetPid: target)
            } catch is CancellationError { }
            catch { self.error = error.userFacingMessage }
        }
    }
}
