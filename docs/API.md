# Collaboration API

`unidesk-collaboration` is the cross-platform core. Hosts submit
[`CollaborationCommand`](../crates/unidesk-collaboration/src/api.rs) values,
subscribe to `CollaborationEvent`, and rebuild UI from
`CollaborationSnapshot`. Product extensions use the same surface; they are not
compiled into the core.

## Commands

| Command | Effect |
| --- | --- |
| `StartPairing` | Begin a pairing window. Identity remains the trust authority. |
| `AcceptDevice` / `RejectDevice` | Resolve a discovered device. |
| `RemoveTrust` | Drop a stored grant. The next connection must pair again. |
| `PublishClipboard` | Publish the current native capture to connected sessions. |
| `CancelClipboard` | Cancel the in-flight publish generation. |
| `SendFiles` / `CancelTransfer` | File collaboration. Hosts authorize paths first. |
| `RequestInputControl` / `ReleaseInputControl` | Input lease. Native posting stays in the host. |

`submit` returning success means the core accepted the command. Remote pairing
or a finished transfer is a later event.

## Events and snapshots

Events are metadata only. Clipboard bodies travel as `ContentHandle` (id, mime,
size, digest). Snapshots are the rebuild surface for a new subscriber.

## Construction

```rust
let ports = HostPorts::memory(data_dir);
let transport = Arc::new(MemoryTransport::new());
let core = CollaborationCore::new_desktop(ports, transport, data_dir);
core.start().await?;
```

Input-only processes call `CollaborationCore::new_input_only` so clipboard,
files and product extensions are never started.
