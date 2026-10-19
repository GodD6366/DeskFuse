import AppKit
import Foundation
import Observation
import DeskFuseEngine

/// Observable projections only. Rust owns definitions, processing and run state.
@MainActor @Observable final class WorkflowStore {
    static let shared = WorkflowStore()
    private(set) var definitions: [WorkflowDefinition] = []
    private(set) var catalog = WorkflowCatalog()
    private(set) var runs: [WorkflowRun] = []
    private(set) var error: String?
    @ObservationIgnored private var engine: Engine?
    @ObservationIgnored private var reloadRevision = 0

    /// Root startup hook: `Task { await WorkflowStore.shared.reload(engine: engine) }`.
    func reload(engine: Engine) async {
        self.engine = engine
        reloadRevision += 1
        let requested = reloadRevision
        do {
            let (values, catalogJSON) = try await Task.detached {
                (try engine.workflows(), try engine.workflowCatalogJson())
            }.value
            let catalog = try JSONDecoder().decode(WorkflowCatalog.self, from: Data(catalogJSON.utf8))
            guard requested == reloadRevision else { return }
            definitions = values.map { native in
                var flow = WorkflowDefinition(native: native)
                for index in flow.steps.indices where flow.steps[index].kind == .systemShortcut {
                    flow.steps[index].local_shortcut_id = catalog.bindings.first {
                        $0.workflow_id == flow.id && $0.step_index == UInt32(index)
                    }?.shortcut_id ?? ""
                }
                return flow
            }
            self.catalog = catalog; error = nil
            DeskFuseImageAppShortcuts.updateAppShortcutParameters()
        } catch { if requested == reloadRevision { self.error = error.userFacingMessage } }
    }
    func refresh() async {
        if let engine = engine ?? AppStore.shared.engine { await reload(engine: engine) }
    }
    private func readyEngine() throws -> Engine {
        guard let engine = engine ?? AppStore.shared.engine else { throw WorkflowExecutionError(message: "引擎尚未就绪") }
        return engine
    }
    func save(_ draft: WorkflowDefinition) async throws {
        let engine = try readyEngine(); let definition = draft.native
        try await Task.detached { try engine.saveWorkflow(workflow: definition) }.value
        await reload(engine: engine)
        if let error { throw WorkflowExecutionError(message: error) }
    }
    func delete(_ id: String) async throws {
        let engine = try readyEngine()
        try await Task.detached { try engine.deleteWorkflow(id: id) }.value
        await reload(engine: engine)
        if let error { throw WorkflowExecutionError(message: error) }
    }
    func restoreConflict(_ id: String) async throws {
        let engine = try readyEngine()
        try await Task.detached { try engine.restoreWorkflowConflict(id: id) }.value
        await reload(engine: engine)
        if let error { throw WorkflowExecutionError(message: error) }
    }
    func share(workflowID: String?, historyID: UInt64?, paths: [String], peerID: String, bypass: Bool) async throws {
        let engine = try readyEngine()
        error = nil
        _ = try await Task.detached {
            try engine.shareWorkflow(workflowId: workflowID, historyId: historyID, paths: paths, peerId: peerID, bypass: bypass)
        }.value
    }
    func systemShortcuts() async throws -> [SystemShortcut] {
        let engine = try readyEngine()
        return try await Task.detached { try engine.systemShortcuts() }.value
    }
    func run(workflowId: String, input: WorkflowInput, output: WorkflowOutputMode, targetPid: Int32 = 0) async throws -> WorkflowRun {
        let engine = try readyEngine()
        error = nil
        try Task.checkCancellation()
        let id = try await Task.detached {
            try engine.runWorkflow(workflowId: workflowId, input: input, output: output, targetPid: targetPid)
        }.value
        return try await withTaskCancellationHandler {
            do {
                while true {
                    try Task.checkCancellation()
                    let state = try await Task.detached { try engine.workflowRun(runId: id) }.value
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
                    try? await Task.detached { try engine.cancelWorkflow(runId: id) }.value
                    if let state = try? await Task.detached(operation: { try engine.workflowRun(runId: id) }).value { record(state) }
                } else { self.error = error.userFacingMessage }
                throw error
            }
        } onCancel: {
            Task.detached { try? engine.cancelWorkflow(runId: id) }
        }
    }
    func cancel(_ id: String) {
        Task {
            do {
                let engine = try readyEngine()
                try await Task.detached { try engine.cancelWorkflow(runId: id) }.value
            } catch { self.error = error.userFacingMessage }
        }
    }
    /// Includes jobs started by the Rust global shortcut listener, not just UI callers.
    func consume(_ run: WorkflowRun) { record(run) }
    private func record(_ run: WorkflowRun) {
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
    var workflowDefinitions: [WorkflowDefinition] { WorkflowStore.shared.definitions }
    func saveWorkflow(_ draft: WorkflowDefinition, onApplied: (() -> Void)? = nil) {
        error = nil
        Task {
            do { try await WorkflowStore.shared.save(draft); onApplied?() }
            catch { self.error = error.userFacingMessage }
        }
    }
    func deleteWorkflow(_ id: String) {
        error = nil
        Task {
            do { try await WorkflowStore.shared.delete(id) }
            catch { self.error = error.userFacingMessage }
        }
    }
    func runWorkflow(_ id: String, historyID: UInt64? = nil) {
        error = nil
        let target = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0
        Task {
            do {
                _ = try await WorkflowStore.shared.run(workflowId: id,
                    input: historyID.map { .history(id: $0) } ?? .clipboard,
                    output: historyID == nil ? .configured : .copy, targetPid: target)
            } catch is CancellationError { }
            catch { self.error = error.userFacingMessage }
        }
    }
}

extension AppStore {
    func shareWorkflow(_ workflowID: String?, historyID: UInt64? = nil, paths: [String] = [], peerID: String, bypass: Bool = false) {
        error = nil
        Task {
            do {
                try await WorkflowStore.shared.share(workflowID: workflowID, historyID: historyID,
                                                     paths: paths, peerID: peerID, bypass: bypass)
            } catch { self.error = error.userFacingMessage }
        }
    }
}
