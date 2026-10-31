import Foundation

/// Wire protocol between the native app (which owns the engine) and an opt-in
/// MyGo UI child process. It runs only over the child's inherited stdin/stdout
/// pipes; nothing here opens a socket. This file depends on Foundation only.
///
/// Frame: 4-byte big-endian length, then that many bytes of UTF-8 JSON object.
/// Request: `{"v":1,"id":"…","op":"hello|library.query|cancel","params":{…}}`
/// Reply:   `{"v":1,"id":"…","ok":true,"result":{…}}` or
///          `{"v":1,"id":"…"|null,"ok":false,"error":{"code","message","retryable"}}`
enum MyGoBridge {
    static let protocolName = "deskfuse.mygo.bridge"
    static let version = 1
    static let maxRequestBytes = 262_144
    static let maxReplyBytes = 32 * 1024 * 1024
    static let maxInFlight = 16
    static let maxIDBytes = 64
    static let maxSearchBytes = 1024
    /// Read-only engine query operations. `cancel_query` and mutations stay native.
    static let libraryOps: Set<String> = ["state", "revision", "items", "counts"]
    static let mutationOps: Set<String> = ["settings", "save_list", "delete_list", "reorder", "membership", "delete_items", "set_item_text", "set_item_title", "set_item_ocr", "want_item_payload", "clear_history", "clear_remote_index"]
    static let environmentKey = "DESKFUSE_MYGO_BRIDGE"
    static let environmentValue = "stdio-v1"
}

struct MyGoBridgeError: Error, Equatable {
    var code: String
    var message: String
    var retryable = false
    var id: String?

    /// A malformed frame stream cannot be resynchronised; the channel is closed.
    var isFatal: Bool { code == "bad_frame" || code == "frame_too_large" }
}

enum MyGoBridgeCommand: Equatable, Sendable {
    case hello
    case libraryQuery(String)
    case libraryMutate(String)
    case appSnapshot
    case clipboardContent(id: UInt64, includeImage: Bool, includeRTF: Bool)
    case clipboardThumbnail(id: UInt64, maxPixel: Int)
    case appInvoke(action: String, params: Data)
    case cancel(target: String)
}

struct MyGoBridgeRequest: Equatable, Sendable {
    let id: String
    let command: MyGoBridgeCommand
}

/// Accumulates pipe bytes and yields complete frames. A length outside
/// `1...maxRequestBytes` throws before any payload is buffered.
struct MyGoFrameReader {
    private var buffer = Data()

    mutating func append(_ data: Data) { buffer.append(data) }

    mutating func nextFrame() throws -> Data? {
        guard buffer.count >= 4 else { return nil }
        let start = buffer.startIndex
        var length = 0
        for byte in buffer[start..<start + 4] { length = (length << 8) | Int(byte) }
        guard length > 0 else { throw MyGoBridgeError(code: "bad_frame", message: "空帧") }
        guard length <= MyGoBridge.maxRequestBytes else {
            throw MyGoBridgeError(code: "frame_too_large", message: "请求超过 \(MyGoBridge.maxRequestBytes) 字节")
        }
        guard buffer.count >= 4 + length else { return nil }
        let payload = Data(buffer[start + 4..<start + 4 + length])
        buffer.removeSubrange(start..<start + 4 + length)
        return payload
    }
}

enum MyGoBridgeFrame {
    static func encode(_ payload: Data) -> Data {
        var frame = Data(capacity: payload.count + 4)
        let length = UInt32(truncatingIfNeeded: payload.count)
        frame.append(contentsOf: [UInt8(length >> 24 & 0xff), UInt8(length >> 16 & 0xff),
                                  UInt8(length >> 8 & 0xff), UInt8(length & 0xff)])
        frame.append(payload)
        return frame
    }
}

enum MyGoBridgeRequestParser {
    static func parse(_ payload: Data) throws -> MyGoBridgeRequest {
        guard let object = (try? JSONSerialization.jsonObject(with: payload)) as? [String: Any] else {
            throw MyGoBridgeError(code: "bad_json", message: "请求不是 JSON 对象")
        }
        guard let id = object["id"] as? String, !id.isEmpty, id.utf8.count <= MyGoBridge.maxIDBytes else {
            throw MyGoBridgeError(code: "invalid_request", message: "id 必须是 1 到 \(MyGoBridge.maxIDBytes) 字节的字符串")
        }
        func fail(_ code: String, _ message: String) -> MyGoBridgeError {
            MyGoBridgeError(code: code, message: message, id: id)
        }
        guard let version = object["v"] as? Int, version == MyGoBridge.version else {
            throw fail("unsupported_version", "仅支持协议版本 \(MyGoBridge.version)")
        }
        guard let op = object["op"] as? String else { throw fail("invalid_request", "缺少 op") }
        let params = object["params"] as? [String: Any]
        if object["params"] != nil, params == nil { throw fail("invalid_params", "params 必须是对象") }
        switch op {
        case "hello":
            return MyGoBridgeRequest(id: id, command: .hello)
        case "cancel":
            guard let target = params?["id"] as? String, !target.isEmpty, target.utf8.count <= MyGoBridge.maxIDBytes else {
                throw fail("invalid_params", "cancel 需要 params.id")
            }
            return MyGoBridgeRequest(id: id, command: .cancel(target: target))
        case "library.query":
            guard var query = params, let queryOp = query["op"] as? String, MyGoBridge.libraryOps.contains(queryOp) else {
                throw fail("invalid_params", "params.op 必须是 \(MyGoBridge.libraryOps.sorted().joined(separator: "/"))")
            }
            if let search = query["query"], (search as? String)?.utf8.count ?? .max > MyGoBridge.maxSearchBytes {
                throw fail("invalid_params", "params.query 必须是不超过 \(MyGoBridge.maxSearchBytes) 字节的字符串")
            }
            // EngineClient assigns the cancellation token; a caller must not pick one.
            if let cursor = query["cursor"] as? [String: Any] {
                var normalized = cursor
                for key in ["id", "sort_at_ms"] {
                    if let value = cursor[key] {
                        guard let number = numericID(value) else { throw fail("invalid_params", "分页游标无效") }
                        normalized[key] = NSNumber(value: number)
                    }
                }
                query["cursor"] = normalized
            }
            query["query_id"] = nil
            guard let data = try? JSONSerialization.data(withJSONObject: query) else {
                throw fail("invalid_params", "params 无法编码")
            }
            return MyGoBridgeRequest(id: id, command: .libraryQuery(String(decoding: data, as: UTF8.self)))
        case "library.mutate":
            guard var mutation = params, let op = mutation["op"] as? String, MyGoBridge.mutationOps.contains(op) else {
                throw fail("invalid_params", "不支持的 library.mutate op")
            }
            let destructive: Set<String> = ["delete_list", "delete_items", "clear_history", "clear_remote_index"]
            if destructive.contains(op), mutation["confirm"] as? Bool != true { throw fail("confirmation_required", "删除操作需要确认") }
            for key in ["id", "history_id"] {
                if let value = mutation[key], let text = value as? String, op != "delete_list", op != "reorder" {
                    guard let number = UInt64(text) else { throw fail("invalid_params", "条目 ID 无效") }
                    mutation[key] = NSNumber(value: number)
                }
            }
            if op == "delete_items", let values = mutation["ids"] as? [Any] {
                guard !values.isEmpty, values.count <= 1000 else { throw fail("invalid_params", "条目数量无效") }
                mutation["ids"] = try values.map { value -> NSNumber in
                    guard let number = numericID(value) else { throw fail("invalid_params", "条目 ID 无效") }
                    return NSNumber(value: number)
                }
            }
            mutation["query_id"] = nil
            guard let data = try? JSONSerialization.data(withJSONObject: mutation) else { throw fail("invalid_params", "params 无法编码") }
            return MyGoBridgeRequest(id: id, command: .libraryMutate(String(decoding: data, as: UTF8.self)))
        case "app.snapshot":
            return MyGoBridgeRequest(id: id, command: .appSnapshot)
        case "clipboard.content":
            guard let value = params?["id"], let idValue = numericID(value) else { throw fail("invalid_params", "需要非负 numeric id") }
            return MyGoBridgeRequest(id: id, command: .clipboardContent(id: idValue, includeImage: params?["include_image"] as? Bool ?? false, includeRTF: params?["include_rtf"] as? Bool ?? false))
        case "clipboard.thumbnail":
            guard let value = params?["id"], let idValue = numericID(value) else { throw fail("invalid_params", "需要非负 numeric id") }
            return MyGoBridgeRequest(id: id, command: .clipboardThumbnail(id: idValue, maxPixel: max(64, min(480, params?["max_pixel"] as? Int ?? 160))))
        case "app.invoke":
            guard let action = params?["action"] as? String else { throw fail("invalid_params", "需要 action") }
            guard let values = params?["params"] as? [String: Any] else { throw fail("invalid_params", "params.params 必须是对象") }
            let data = try JSONSerialization.data(withJSONObject: values, options: [.sortedKeys])
            return MyGoBridgeRequest(id: id, command: .appInvoke(action: action, params: data))
        default:
            throw fail("unsupported_op", "不支持的 op：\(op.prefix(64))")
        }
    }

    private static func numericID(_ value: Any) -> UInt64? {
        if let string = value as? String { return UInt64(string) }
        if let number = value as? NSNumber, number.doubleValue >= 0, number.doubleValue.rounded() == number.doubleValue { return UInt64(number.uint64Value) }
        return value as? UInt64
    }
}

enum MyGoBridgeReply {
    private static func encodedID(_ id: String?) -> Data {
        guard let id, let data = try? JSONEncoder().encode(id) else { return Data("null".utf8) }
        return data
    }

    /// `result` is embedded verbatim so a large engine page is never decoded and re-encoded.
    static func success(id: String, result: Data) throws -> Data {
        let bytes = result.drop { $0 == 0x20 || $0 == 0x0a || $0 == 0x0d || $0 == 0x09 }
        guard bytes.first == UInt8(ascii: "{") else {
            throw MyGoBridgeError(code: "internal", message: "引擎返回了非对象结果", id: id)
        }
        guard result.count <= MyGoBridge.maxReplyBytes else {
            throw MyGoBridgeError(code: "response_too_large", message: "结果超过 \(MyGoBridge.maxReplyBytes) 字节，请缩小 limit", id: id)
        }
        var reply = Data(#"{"v":1,"id":"#.utf8)
        reply.append(encodedID(id))
        reply.append(Data(#","ok":true,"result":"#.utf8))
        reply.append(result)
        reply.append(UInt8(ascii: "}"))
        return reply
    }

    static func failure(_ error: MyGoBridgeError) -> Data {
        var reply = Data(#"{"v":1,"id":"#.utf8)
        reply.append(encodedID(error.id))
        reply.append(Data(#","ok":false,"error":"#.utf8))
        let body: [String: Any] = ["code": error.code, "message": error.message, "retryable": error.retryable]
        reply.append((try? JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])) ?? Data("{}".utf8))
        reply.append(UInt8(ascii: "}"))
        return reply
    }

    static let helloResult = Data(
        #"{"protocol":"\#(MyGoBridge.protocolName)","version":\#(MyGoBridge.version),"max_request_bytes":\#(MyGoBridge.maxRequestBytes),"max_in_flight":\#(MyGoBridge.maxInFlight),"ops":["hello","library.query","library.mutate","app.snapshot","app.invoke","clipboard.content","clipboard.thumbnail","cancel"],"library_ops":["items","counts","revision","state"]}"#.utf8)
}
