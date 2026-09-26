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
{"version":2,"request_id":10,"command":{"PublishFileClipboard":{"session_epoch":0,"sequence":2,"paths":["C:\\Users\\me\\Documents\\a.txt"]}}}
{"version":2,"request_id":10,"command":{"RequestFileClipboard":{"session_epoch":1,"transfer_id":"0b9e6c1d-2d40-4f21-a15b-64d5b8e4c8f0"}}}
{"version":2,"request_id":10,"command":{"CancelFileClipboard":{"session_epoch":1,"transfer_id":"0b9e6c1d-2d40-4f21-a15b-64d5b8e4c8f0"}}}
{"version":2,"request_id":10,"command":{"CommitFileClipboard":{"session_epoch":1,"transfer_id":"0b9e6c1d-2d40-4f21-a15b-64d5b8e4c8f0","applied":true}}}
{"version":2,"request_id":11,"command":{"RegisterInput":{"topology":{"generation":1,"displays":[{"id":"win-monitor-1","frame_x":0.0,"frame_y":0.0,"width":1920,"height":1080,"scale_factor":1.0,"rotation_degrees":0,"is_primary":true}]}}}}
{"version":2,"request_id":12,"command":{"PollInput":{"session_epoch":1,"after_sequence":0,"desktop":"Default"}}}
{"version":2,"request_id":13,"command":"ReclaimInput"}
{"version":2,"request_id":13,"command":{"EnterInputControl":{"session_id":"6f0f3d1e-8d1a-4c25-9f0e-111111111111","target_display":"mac-display-1","x":5.0,"y":200.0}}}
{"version":2,"request_id":13,"command":{"SendInputEvents":{"session_id":"6f0f3d1e-8d1a-4c25-9f0e-111111111111","target_display":"mac-display-1","batch":1,"events":[]}}}
{"version":2,"request_id":13,"command":{"LeaveInputControl":{"session_id":"6f0f3d1e-8d1a-4c25-9f0e-111111111111"}}}
{"version":2,"request_id":13,"command":{"ReleaseInputControl":{"session_id":"6f0f3d1e-8d1a-4c25-9f0e-111111111111"}}}
{"version":2,"request_id":2,"command":{"EnableInputControl":{"peer_id":"device-id"}}}
{"version":2,"request_id":3,"command":"DisableInputControl"}
{"version":2,"request_id":4,"command":{"RestartInputControl":{"peer_id":"device-id"}}}
{"version":2,"request_id":5,"command":{"MoveControlDevice":{"device_id":"device-id","display_id":"mac-display-1","x":1920.0,"y":0.0}}}
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
{"package_version":"0.1.0","session_epoch":0,"client_session_id":1,"sequence":1,"local_device_id":"...","local_name":"DESKTOP-AB12CD3","network_running":true,"control_port":42424,"capabilities":{"clipboard":true,"files":true,"input":true,"elevated_input":true,"secure_desktop_input":false,"login_input":false,"secure_attention_sequence":false,"updater":false},"input":{"security":{"session_bound":true,"ui_access":true,"elevated":false,"desktop":"Default","reason":"UIAccess verified; Default desktop only; secure/login desktops and SAS unsupported"},"helper_connected":true,"managed":false,"status":"standby","reason":null,"controller_id":null,"display_count":1},"peers":[],"pending":[],"file_transfers":[],"file_cache":{"max_bytes":1073741824,"used_bytes":0},"pairing_status":null,"last_error":null}
```

`routes` contain serialized core RouteCandidate values; labels can display `endpoint`, `health`, `kind`. `last_error` and `pairing_status` are bounded metadata-only messages. Capability flags are runtime availability, not user preference; false must visibly disable the relevant UI. Updater remains false. `files` is true for an active authenticated desktop session whose service runtime advertises `DEFERRED_FILE_CLIPBOARD`; it gates `PublishFileClipboard` and the transfer queue UI, and is additionally gated per peer by the route capability and actual HELLO exchange. Input is true only while the ordinary desktop helper heartbeat and authenticated GUI generation are current; elevated, Winlogon and UAC desktops remain unavailable. Clipboard is true for an active authenticated desktop session; per-peer readiness is additionally gated by the actual HELLO exchange and connection generation. Publish rejects when no eligible trusted peer has completed that handshake. Clipboard uses the existing acknowledged sync v2 protocol and the macOS zlib/fixed-bincode native bundle (common UTI subset), never bare legacy ClipboardPayload MIME. It sends APPLIED only after CommitClipboard(applied:true). TIFF-only mac native images are converted to PNG (prefer existing public.png), with 4,194,304-pixel / 32 MiB decoder allocation limits and PNG output within the remaining 16 MiB common-content budget. Larger or unsupported images are rejected, not silently acknowledged.

## Cross-device file transfer (deferred clipboard file protocol)

The service speaks the macOS engine's `application/x-unidesk-file-clipboard;version=1` frames over the existing clipboard session. Only the metadata-only `Deferred` announcement rides the acknowledged sync v2 channel (so it is chunked, retried and acknowledged like any other clipboard payload, with APPLIED meaning "offer recorded", never "files moved"); all content frames (`Request`, `RequestedOffer`, `Chunk`, `Complete`, `Receipt`, `Ack`, `Cancel`) are direct `CLIPBOARD_FILE_MIME` payloads.

Sending (Windows → remote):

1. The desktop detects a local `CF_HDROP` copy (or the shelf's 发送到设备), sends `InvalidateClipboard` and then `PublishFileClipboard { paths }` reusing the invalidation sequence, exactly like `PublishClipboard`. Paths are absolute local file/folder references (≤512); they are service-session data and are never sent to peers as text.
2. The service scans metadata only (`DeferredClipboardSource`), registers one offer per eligible peer (trusted, connected, route advertises `DEFERRED_FILE_CLIPBOARD`) and announces a `Deferred` frame per peer. Offers expire after ten minutes; a newer local copy does not revoke an already-announced offer. The reply is Queued even before the scan finishes; scan failures surface via `last_error`. No eligible peer rejects synchronously with `no_file_peer`.
3. When the peer requests content, the service revalidates the source unchanged, materialises the manifest, streams `RequestedOffer` + 1 MiB chunks + `Complete`, honours the receiver's `Receipt` flow-control credit (aborting after 30 s without credit) and waits up to 600 s for the receiver's `Ack`. Source changes cancel the send; `InvalidateClipboard`/`reset` cancel jobs and notify peers.

Receiving (remote → Windows):

1. A `Deferred` announcement records a pending offer; `Poll`/`Snapshot` expose it in `file_transfers` (`state:"offered"`, names, total bytes, peer). Offers never auto-download and never become clipboard content by themselves.
2. The desktop explicitly requests with `RequestFileClipboard` (quota: 1 GiB per transfer by default). The service sends `Request`, materialises the manifest when the peer answers, and streams chunks into a bounded staging area under the protected service data directory (`transfer-cache/<transfer-id>`); each accepted frame is credited with a `Receipt` so the sender cannot stall.
3. After the last chunk the worker verifies every complete-file SHA-256 and commits; `Snapshot` then shows `state:"staged"` with absolute `staged_paths`. The desktop copies the files into a user-chosen destination (as the user), may place them on the clipboard, and reports `CommitFileClipboard { applied }`; only then does the service send the protocol `Ack` (or `Cancel` on failure). The staging directory is removed after the commit. The desktop has 60 s after staging; a timeout cancels the sender and removes staged bytes.
4. Only up to 3 receiving and 4 sending transfers run concurrently. Session lock/logoff, epoch change or service stop cancel all transfers, notify peers, and remove staged bytes. Received files never overwrite silently: the desktop renames conflicts (`名称 (2).扩展名`), and the service cache never mixes transfers.

`Snapshot` additionally carries `file_transfers` (bridge view rows: `transfer_id`, `peer_id`, `direction`, `names`, `total_bytes`, `transferred_bytes`, `state`, `detail`, `staged_paths`, `created_unix_ms`, `updated_unix_ms`) and `file_cache { max_bytes, used_bytes }`. Terminal rows remain visible for ten minutes. Explorer OLE on-demand rendering uses `FileGroupDescriptorW` and `FileContents`: metadata enumeration does not download, while the first `GetData(FileGroupDescriptorW)` requests and verifies the deferred offer into a user-private staging view. `CommitClipboard` remains text/image-only for ordinary payloads; file offers use `CommitFileClipboard`. Explorer may invoke the descriptor while previewing, so that call can intentionally start the transfer.

Implementation note for parent integration: runtime currently exposes no rename-local command. `Rename` explicitly rejects; the displayed local name is the Windows computer hostname (`GetComputerNameExW(ComputerNameDnsHostname)`), matching the macOS engine's `/bin/hostname` display label. Renaming the computer and restarting the service updates it; the GUI keeps the field read-only like macOS.

## Ordinary-desktop input helper

The service owns a separate input-only QUIC runtime on UDP 42425 and advertises `_unidesk-input._udp`; the ordinary control/clipboard runtime on 42424 does not advertise input. `unidesk-input-helper.exe` is launched and supervised by the WPF process, receives no private key or peer authority, and registers only its validated native display topology. Its parent PID is mandatory; parent exit, desktop generation expiry, pipe failure, helper fault or lease expiry releases all injected keys/buttons and stops the helper.

`PollInput` returns ordered `Lease`, `Batch` and `Release` deliveries plus a cursor. The helper has an independent request-id sequence because it is a separate process; these commands are idempotent and still bound to the same kernel-authenticated SID/logon LUID/session and service epoch. Batches carry runtime-validated session/display/state epochs and the existing bounded `SharedInputEvent` array. The helper enforces its own maximum 1,500 ms lease and refuses non-Default desktops. `InputFault` reports a bounded metadata-only reason and marks the exact managed activation unavailable. `ReclaimInput` pauses the current activation and queues a release; Ctrl+Alt+Win+Esc reaches the same path.

## Windows-as-controller input (2026-09-18)

The same helper captures local input when this desktop is an input controller. All edge math runs locally through the cross-platform `SharingController`/`layout_displays` (identical to the macOS engine); only the session protocol crosses this pipe:

1. The GUI enables control with `EnableInputControl { peer_id }`. The service preserves saved shortcuts/corners/placements and only sets role=Controller and adds the participant, then persists via the input runtime's settings store. `DisableInputControl` returns to Disabled keeping the saved participants. `RestartInputControl` re-arms a retired activation after a remote pause.
2. While the helper heartbeat is live, the service reports `SetInputAvailability(true)` to the input runtime; helper death or `InputFault` reverts to false, releasing both directions.
3. The runtime's activation actor sends `Activate` intents to the peer (fixed 1500 ms TTL, 250 ms renew) using the same protocol as macOS. Peer approval arrives as an activation status (`Preparing`/`Ready`/`PausedByUser`/`Unavailable`) mirrored in `Snapshot.input.activations`.
4. When the runtime grants a control session, the desktop pushes a `ControlGranted { session_id, lease_ms }` delivery; denials and releases arrive as `ControlDenied`/`ControlReleased`. The helper mirrors these grants locally exactly like the macOS coordinator (retired-session set, expiry, `arm_session` on the capture suppression lease).
5. When the local pointer crosses an edge onto a remote display, the helper sends `EnterInputControl { session_id, target_display, x, y }` (session taken from `input.links[].control_session`), then routes captured events as `SendInputEvents { session_id, target_display, batch, events }`. `batch` is per-session monotonic; the service drops non-increasing batches so a retransmitted helper request can never double-inject. Batches are capped at 256 events. Crossing back sends `LeaveInputControl`; ending control sends `ReleaseInputControl`. The runtime validates every frame against the live lease, remote topology and activation, exactly like the macOS engine.
6. `MoveControlDevice { device_id, display_id, x, y }` moves a remote display group in the controller layout (runtime-owned magnetic snapping and persistence); the GUI canvas is read-only positioning for now.

`Snapshot.input` now carries `local_device_id`, effective `settings`, `controlling`, `activations`, `remote_topologies` and `links` for both directions. Ctrl+Alt+Win+Esc remains the emergency chord for both roles: it releases held input, ends any control session, pauses a managed activation and stops the helper.

This checkpoint wires both directions on the unlocked ordinary desktop. It is not evidence for UAC/Winlogon/login input, secure-desktop input, SAS, cross-system latency acceptance or physical two-machine behavior.

### UIAccess and capability truth

`RegisterInput` and every subsequent helper request are authenticated against the kernel-reported named-pipe client PID. The service opens that process with query/synchronize rights and verifies its installed helper path, process creation time, token session, user SID, authentication LUID, and `TokenUIAccess`; a helper without the effective UIAccess token bit is rejected rather than downgraded to ordinary input. `PollInput` carries the helper's fresh read-only session/desktop probe and the current service epoch. The service accepts input only when the helper is alive, bound to the same unlocked physical-console user session, and on `Default`; `Locked`, `Winlogon`/UAC and unknown desktops clear the lease and report `desktop_unavailable`.

The protocol exposes `capabilities.elevated_input` and `input.security` only from these observations. `secure_desktop_input`, `login_input`, and `secure_attention_sequence` are always false in this implementation: UIAccess is not a service/session-agent or secure-desktop authorization, and no capability is inferred from the manifest alone.

## Windows 启动与传输修正（2026-09-13）

客户端读取完整响应后写入单字节 `0x06` 确认；服务保留 pipe 实例至确认/客户端关闭，仍受外层请求超时限制，避免响应尚未被读取便关闭管道。此字节仅确认传输读取，不代表剪贴板 APPLIED（仍必须原生写入后 CommitClipboard）。会话校验失败只回复不含业务数据的 `session_unavailable`，不放宽物理桌面/解锁/身份约束。

用户只打开 uniDesk GUI。GUI 查询 SCM，后台停止时自动请求启动并等待就绪，失败有退避，不使用备用身份或另起网络运行时。安装器仅为 Users 增加现有固定服务的 START 权限，不授予 STOP、修改配置或修改安全描述符权限；GUI 无需管理员运行。
