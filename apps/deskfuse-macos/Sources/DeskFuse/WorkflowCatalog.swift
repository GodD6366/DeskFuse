import Foundation

struct WorkflowCatalog: Decodable, Equatable {
    var local_device_id = ""
    var sync_status = "等待引擎"
    var generation: UInt64 = 0
    var error: String?
    var conflicts: [WorkflowConflict] = []
    var bindings: [WorkflowLocalBinding] = []
    var peers: [WorkflowPeerSync] = []
    var issues: [WorkflowLocalIssue]?
}
struct WorkflowConflict: Decodable, Identifiable, Equatable {
    var id: String
    var name: String
    var message: String
}
struct WorkflowLocalBinding: Decodable, Equatable {
    var workflow_id: String
    var step_index: UInt32
    var shortcut_id: String
}
struct WorkflowPeerSync: Decodable, Equatable {
    var device_id: String
    var status: String
}

struct WorkflowLocalIssue: Decodable, Equatable {
    var workflow_id: String
    var message: String
}
