# Windows desktop IPC v2

Service owns identity/network/trust. Desktop never opens a second runtime.

## Transport

`\\.\pipe\com.unidesk.app.desktop.v2`: one request/response per connection, four-byte little-endian unsigned JSON UTF-8 byte length, 1..=25165824 bytes. Each exchange authenticates kernel pipe session, token SID and authentication LUID, physical console and unlocked state, and service session epoch. Requests expire after three seconds. Existing `\\.\pipe\com.unidesk.app.service.v1` Status remains unchanged with its 16 KiB bound.

Requests have `version:2`, a process-local monotonically increasing `request_id` (u64), and `command` (serde externally tagged). GUI commands share one sequence; the separately authenticated input helper uses its own sequence only for the idempotent helper command subset.

```json
{"version":2,"request_id":1,"command":"Snapshot"}
{"version":2,"request_id":2,"command":{"RequestPairing":{"peer_id":"device-id"}}}
{"version":2,"request_id":3,"command":{"RequestPairingAt":{"ipv4":"192.168.1.23"}}}
{"version":2,"request_id":4,"command":{"AcceptPairing":{"request_id":"invitation-uuid"}}}
{"version":2,"request_id":5,"command":{"RejectPairing":{"request_id":"invitation-uuid"}}}
{"version":2,"request_id":6,"command":{"Disconnect":{"peer_id":"device-id"}}}
{"version":2,"request_id":7,"command":{"Revoke":{"peer_id":"device-id"}}}
{"version":2,"request_id":8,"command":{"Rename":{"name":"My Windows PC"}}}
{"version":2,"request_id":9,"command":{"Poll":{"session_epoch":0,"after_sequence":0}}}
{"version":2,"request_id":10,"command":{"PublishClipboard":{"session_epoch":0,"sequence":1,"formats":[{"mime":"text/plain","data_base64":"aGVsbG8="}]}}}
{"version":2,"request_id":11,"command":{"RegisterInput":{"topology":{"generation":1,"displays":[{"id":"win-monitor-1","frame_x":0.0,"frame_y":0.0,"width":1920,"height":1080,"scale_factor":1.0,"rotation_degrees":0,"is_primary":true}]}}}}
{"version":2,"request_id":12,"command":{"PollInput":{"after_sequence":0}}}
{"version":2,"request_id":13,"command":"ReclaimInput"}
```

Snapshot opens/reopens desktop clipboard generation, increments the service epoch and discards prior buffered clipboard. Use the returned session_epoch, not the sample zero. Poll never opens a generation: first issue Snapshot and use its epoch. Reconnection after application restart must use Snapshot. Poll returns a full snapshot, at most one latest clipboard item after its cursor, and cursor. Poll once per second; inactive desktop expires after 5 seconds. Session lock/logoff/change invalidates and clears all clipboard buffers. Publish supports `text/plain`, `image/png`, `text/html`, `text/rtf` and max 16 MiB total decoded bytes. File references are not supported and must not be reduced to text paths.

For **every local native clipboard change**, including files, oversized/unsupported content, and disabling sync, first send `{"version":2,"request_id":12,"command":{"InvalidateClipboard":{"session_epoch":1,"sequence":2}}}`. This command has no content and discards all partial/awaiting remote content with SUPERSEDED receipts. `sequence` must increase for each local change. A following `PublishClipboard` may reuse that exact sequence once; a duplicate Publish is rejected. If Publish is sent without prior Invalidate, it performs the same invalidation before validating content. Do not skip invalidation when no supported payload can be captured.

Desktop MUST locally recheck that its interactive session remains unlocked and its current clipboard generation still equals the pre-poll baseline before native application. It MUST discard responses belonging to an obsolete session_epoch. Desktop MUST acknowledge native write with `{"version":2,"request_id":11,"command":{"CommitClipboard":{"session_epoch":0,"sequence":1,"applied":true}}}`. `applied:false` reports rejected. Service sends network APPLIED only after this command; Poll/transport delivery are never treated as application. Commit returns Queued. New local Publish supersedes any outstanding remote item, so stale Commit cannot overwrite newer local clipboard.

## Response

Always `{"version":2,"request_id":<echo>,"result":...}`. `result` is one of:

- `{"Snapshot":<snapshot>}`
- `{"Poll":{"snapshot":<snapshot>,"clipboard":null,"clipboard_sequence":0}}` (`clipboard` when present has `sequence`, `peer_id`, `formats:[{mime,data_base64}]`).
- `{"Queued":{"message":"queued; observe snapshot for completion"}}` means local command admission only, never remote success.
- `{"Rejected":{"code":"invalid_request","message":"..."}}`, `UnsupportedVersion`.

Snapshot shape (all keys always present):

```json
{"package_version":"0.1.0","session_epoch":0,"client_session_id":1,"sequence":1,"local_device_id":"...","local_name":"uniDesk Windows","network_running":true,"control_port":42424,"capabilities":{"clipboard":true,"files":false,"input":true,"updater":false},"input":{"helper_connected":true,"managed":false,"status":"standby","reason":null,"controller_id":null,"display_count":1},"peers":[],"pending":[],"pairing_status":null,"last_error":null}
```

`routes` contain serialized core RouteCandidate values; labels can display `endpoint`, `health`, `kind`. `last_error` and `pairing_status` are bounded metadata-only messages. Capability flags are runtime availability, not user preference; false must visibly disable the relevant UI. Files/updater remain false. Input is true only while the ordinary desktop helper heartbeat and authenticated GUI generation are current; elevated, Winlogon and UAC desktops remain unavailable. Clipboard is true for an active authenticated desktop session; per-peer readiness is additionally gated by the actual HELLO exchange and connection generation. Publish rejects when no eligible trusted peer has completed that handshake. Clipboard uses the existing acknowledged sync v2 protocol and the macOS zlib/fixed-bincode native bundle (common UTI subset), never bare legacy ClipboardPayload MIME. It sends APPLIED only after CommitClipboard(applied:true). TIFF-only mac native images are converted to PNG (prefer existing public.png), with 4,194,304-pixel / 32 MiB decoder allocation limits and PNG output within the remaining 16 MiB common-content budget. Larger or unsupported images are rejected, not silently acknowledged.

Implementation note for parent integration: runtime currently exposes no rename-local command. Rename will explicitly reject until an authoritative runtime rename API exists; do not only relabel GUI.

## Ordinary-desktop input helper

The service owns a separate input-only QUIC runtime on UDP 42425 and advertises `_unidesk-input._udp`; the ordinary control/clipboard runtime on 42424 does not advertise input. `unidesk-input-helper.exe` is launched and supervised by the WPF process, receives no private key or peer authority, and registers only its validated native display topology. Its parent PID is mandatory; parent exit, desktop generation expiry, pipe failure, helper fault or lease expiry releases all injected keys/buttons and stops the helper.

`PollInput` returns ordered `Lease`, `Batch` and `Release` deliveries plus a cursor. The helper has an independent request-id sequence because it is a separate process; these commands are idempotent and still bound to the same kernel-authenticated SID/logon LUID/session and service epoch. Batches carry runtime-validated session/display/state epochs and the existing bounded `SharedInputEvent` array. The helper enforces its own maximum 1,500 ms lease and refuses non-Default desktops. `InputFault` reports a bounded metadata-only reason and marks the exact managed activation unavailable. `ReclaimInput` pauses the current activation and queues a release; Ctrl+Alt+Win+Esc reaches the same path.

This checkpoint wires Mac controller → Windows receiver on the unlocked ordinary desktop. It is not evidence for Windows controller capture, elevated applications, UAC/Winlogon/login input or physical two-machine behavior.

## Windows 启动与传输修正（2026-09-13）

客户端读取完整响应后写入单字节 `0x06` 确认；服务保留 pipe 实例至确认/客户端关闭，仍受外层请求超时限制，避免响应尚未被读取便关闭管道。此字节仅确认传输读取，不代表剪贴板 APPLIED（仍必须原生写入后 CommitClipboard）。会话校验失败只回复不含业务数据的 `session_unavailable`，不放宽物理桌面/解锁/身份约束。

用户只打开 uniDesk GUI。GUI 查询 SCM，后台停止时自动请求启动并等待就绪，失败有退避，不使用备用身份或另起网络运行时。安装器仅为 Users 增加现有固定服务的 START 权限，不授予 STOP、修改配置或修改安全描述符权限；GUI 无需管理员运行。
