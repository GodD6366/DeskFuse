import Foundation

struct WorkflowCatalog: Decodable, Equatable {
    var local_device_id = ""
    var sync_status = "等待引擎"
    var generation: UInt64 = 0
    var error: String?
    var rules: [WorkflowRuleDraft] = []
    var conflicts: [WorkflowConflict] = []
    var bindings: [WorkflowLocalBinding] = []
    var peers: [WorkflowPeerSync] = []
    var issues: [WorkflowLocalIssue]?
}
struct WorkflowRuleDraft: Codable, Identifiable, Equatable {
    var id = UUID().uuidString
    var enabled = false
    var sender_id = ""
    var target_id = ""
    var content_type: WorkflowContent = .any
    var workflow_id = ""
    var position: UInt32 = 0
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
