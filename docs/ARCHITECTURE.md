# DeskFuse architecture

## Target shape (collaboration core)

Native UI talks to a host assembly. The host injects Host Ports, starts
`deskfuse-collaboration`, and optionally loads product extensions. The core has
no axum, image, zip, workflow, content-library or concrete OS adapter
dependency. macOS and Windows share clipboard, file and input policy.

```text
SwiftUI / WPF
        |
Host assembly (ports + extension selection)
        |
        +-- extensions/ (content-library, workflow, lan-web, update)
        |
deskfuse-collaboration (K3 session, K5 clipboard, K6 files, K7 input)
        |
Host Ports (deskfuse-platform::ports)  <-- implemented by the host
        |
protocol / identity / transport / core / store-core
```

Input subprocesses use `CollaborationCore::new_input_only`. They must not link
content-library, workflow, lan-web or update. Architecture gates live in
`scripts/check-architecture.sh`.

The production macOS binary still links `deskfuse-ffi` → `deskfuse-engine` while
those workers are cut over. See [MIGRATION.md](MIGRATION.md), [API.md](API.md)
and [HOST_PORTS.md](HOST_PORTS.md).

## Native clients and shared Rust engine

The macOS client uses SwiftUI and Observation, with AppKit only for the file
shelf, native drag callbacks and app/window lifecycle. Rust retains discovery,
trust, authenticated QUIC, clipboard/file synchronization, persistent history,
cache leases, input coordination and signed LAN update policy.

```text
SwiftUI views / AppKit interactions
             |
     UniFFI generated Swift bindings
             |
     deskfuse-ffi (typed client API)
             |
     deskfuse-engine (business state and workers)
       /         |          \
  runtime     platform     transfer/input/update
       |         |                |
 discovery / transport / core / protocol / identity / store
```

This is a responsibility diagram, not a complete Cargo dependency graph.

| Layer | Responsibility |
| --- | --- |
| `apps/deskfuse-macos` | Native navigation, selection/focus, previews, menus, settings and shelf UI |
| `deskfuse-ffi` | Typed snapshots, commands, events, content reads and export handles; serialized callbacks |
| `deskfuse-engine` | Extracted application workers, business reducer, clipboard/history/file/cache/input/update coordination |
| `deskfuse-runtime` | Authenticated device network, pairing, connection lifecycle, input network state and update distribution |
| `deskfuse-platform` | Clipboard, permission, input capture/injection contracts and shared metadata-only timing |
| `deskfuse-platform-macos` | Actual macOS permission, pasteboard, CoreGraphics, display and Wi-Fi adapters |
| `deskfuse-platform-windows` / `deskfuse-service` | Native WPF desktop + session-bound service IPC; clipboard/pairing checkpoint, not full parity |
| `core`, `protocol`, `identity`, `store`, `discovery`, `transport`, `transfer`, `input`, `update` | Existing independent domain, I/O and persistence modules |

Concrete macOS adapter selection lives in the engine composition module.
Shared input coordination uses platform traits; native objects stay inside
adapter workers. Rust engine state has no navigation, window, selection or focus.
No production Cargo package depends on GPUI.

## Native API and execution

`EngineSnapshot`, `EngineCommand`, `EngineEvent` and `EngineError` form the native
contract. UniFFI 0.32.1 generates matching Swift/C bindings during builds; clients
never hand-code Rust layouts, share locks or obtain raw runtime/credential handles.
The native binary statically links the engine. The additional Rust dylib is only
used by the binding generator and is not shipped.

### Platform-boundary closure audit

The platform crate is a contract and memory-double crate: it has no AppKit,
Win32, GPUI, or platform dependency. Concrete OS calls remain in
`deskfuse-platform-macos`, `deskfuse-platform-windows`, `deskfuse-update-platform`,
and the native service/UI adapters. The audit found three deliberate residuals
that are not safe to delete in this pass: filesystem-backed memory ports in
`deskfuse-platform`, engine-owned legacy update/LAN/file workers, and the
Windows v2 string `Rejected.code` field. The latter now carries additive
`Rejected.boundary_error` (`BoundaryErrorCode`) while old clients continue to
read `code`.

New boundary code must use `BoundaryErrorCode`, `PlatformCapability`, and typed
`PlatformEvent`; string codes are compatibility projections only. `Reply::rejected_typed`
is the constructor for new v2 rejection paths; `Reply::rejected` is retained
only for pre-typed commands and is the sole legacy-string conversion point.
Capability booleans in the Windows snapshot and the workflow-specific
capability snapshot are observations of existing adapters, not a second
authorization model.

The typed-port closure audit is intentionally explicit:

| Seam | Real wiring | Remaining unused path / deletion prerequisite |
| --- | --- | --- |
| `PlatformCommand` / `PlatformEvent` | Authenticated Windows service IPC validates host clipboard, file-object and workflow commands; WPF consumes typed receipts | Legacy command variants and the `Rejected.code` projection remain for v2 clients; remove only after all clients consume `boundary_error` |
| `FileDataObjectPort` | Windows WPF `DesktopClipboardController` validates local paths and publishes the typed `FileDataObjectCreated` receipt | The Rust collaboration host has no file-object consumer; do not replace deferred transfer with a second path until a native host and protocol consumer are specified |
| `WorkflowExecutionPort` | Engine workers use the injected macOS execution adapter; Windows uses the authenticated interactive host workflow adapter | Filesystem/process helpers remain host-owned and are not a license to reintroduce OS calls into common crates |
| `WorkflowOutputPort` | macOS `host_ports` injects `MacOSWorkflowOutputPort`, whose guarded-paste capability is fail-closed | The extracted collaboration workflow service does not consume this port yet; keep the adapter and unavailable default until output ownership is cut over |

These seams remain contracts rather than dead declarations: each unconsumed
path has an owner and a concrete prerequisite, and no new call site may add a
legacy string mapping.

A single event dispatcher serializes initial snapshots, monotonically revised
updates, command outcomes and host actions. Ordinary snapshots are coalesced at
100 ms; terminal states and host actions flush immediately. Swift callbacks keep
only the latest snapshot plus a reliable FIFO of notifications, and a bounded
wake-up stream updates the Observable store on MainActor. They must
not synchronously call subscribe/unsubscribe/shutdown. Unsubscribe waits for the
last callback; shutdown stops native monitors, releases input and joins workers.

`submit` returns a request ID, and explicit file sends also return a transfer ID.
`CommandApplied` means the worker has applied the command, not that a remote
pairing or file transfer completed; clients continue rendering the authoritative
pending invitation and transfer state. Worker errors return `CommandFailed` with
the same request ID. Engine shutdown terminates outstanding requests explicitly.

Snapshots contain history metadata; selected content is fetched by history ID.
File data and high-frequency input remain in Rust workers, never the UI event
queue. Native shelf staging and each drag hold separate engine cache leases,
released on completion/cancellation. Finder file pastes are independent Rust jobs
created at shortcut time with a visible path-acquisition state. One on-demand
Finder AppleScript query resolves the directory identity before delivery starts;
there is no polled directory cache or freshness deadline.
A private OSAKit language instance runs the fixed read-only script off the event
tap; AX checks current editing/search context, while AppleScript supplies the window
ID and target path. macOS Finder automation consent is separate from AX consent.
These jobs retain verified cache leases and atomically deliver complete top-level items to that
original directory, without replacing a newer clipboard or replaying Command-V.
Other application paste replay checks both the original target process and the
exact pasteboard change count. Emergency reclaim bypasses the
normal Swift actor and application-command queues.

Business settings, identity and history retain the existing storage paths.
Native preferences only persist shell behavior such as drag-shake activation.
No new identity, parallel database, historical network-protocol compatibility
layer or macOS background daemon is introduced by the UI replacement.

## Trust and data boundaries

1. **Identity is authoritative.** A peer is trusted only through the identity
   and pairing flow. Discovery is a hint, not authorization.
2. **Protocol is explicit and versioned.** Every remotely meaningful message
   has a schema/version boundary and a bounded payload. Unknown messages are
   rejected or ignored according to the compatibility rule for that message.
3. **Transfer is idempotent.** Clipboard/file operations carry correlation
   identifiers and terminal receipts. Retries must not produce duplicate
   destinations or duplicate user-visible offers.
4. **Store is local-first.** SQLite stores metadata, settings, pairing state,
   and transfer receipts as defined by the store schema. Clipboard bodies and
   file contents are not written to diagnostics by default.
5. **Secrets stay local.** macOS identity keys are persisted by the identity provider
   in `~/Library/Application Support/DeskFuse/identity.key` with mode `0600`, without Keychain access; logs and exported
   diagnostics contain metadata and hashes only.
6. **Cancellation is normal.** Shutdown, disconnect, permission denial, and
   peer disappearance are represented as explicit outcomes rather than
   panics.

## Local and public networking

Local discovery/transport is the baseline path. Public networking is an
independent, default-off feature flag and transport adapter:

- Disabled means no public helper, listener, relay, DNS/NAT mutation, or
  alternate control path is started.
- Enabled adds only the public path; it does not replace Bonjour/network-member routes,
  pairing, encryption, or the existing trust authority.
- The default-off invariant must be covered by unit tests and a runtime startup
  assertion. Cross-network tests are an additional integration gate.

## Session capabilities and deferred files

An active route exposes the capabilities verified from that connection's signed
Hello. Discovery refreshes can update route candidates but cannot change the
feature set of an established session. Disconnecting clears the session's
capability state. A connection without authenticated capability information
exposes no supported features.

Deferred file clipboard frames require the independent `DEFERRED_FILE_CLIPBOARD`
capability; `TRANSFER` alone is insufficient. Each receiver gets its own offer
identity, so requests, cancellation and completion acknowledgements from one
receiver do not suppress another receiver's paste. User-facing transfer and
cache behavior is defined in [the file transfer contract](FILE_TRANSFER.md).

## Input sharing and the independent device network

`NetworkSnapshot` remains the network foundation. `InputSharingSnapshot` is a
separate watch stream carrying settings, native local/remote display topologies
and input availability/lease progress per peer. The UI renders runtime reasons;
it never infers input readiness from a clipboard connection, IP address or
Bonjour record. Tokens remain private to runtime and are not exposed in UI
snapshots or diagnostics.

Input settings are persisted by the runtime/store command boundary. The app
owns no second settings file. Only the controller selects participants and
decides whether sharing is enabled. A receiver's short-lived managed activation
is runtime-owned and never persisted into its input settings. Display IDs come from native display identity,
not array positions; coordinates are display-local logical points and complete
device desktop placement is a translation of that device's native topology.
Reconfiguration preserves native internal arrangement. Layout memory is bounded to
128 per-device profiles keyed by the controller and receiver's physical display
geometry, ignoring generation, discovery order and readiness. Remote coordinates
are relative to the controller desktop origin. Topology transitions remember the
previous combination; only explicit placement changes overwrite an existing
profile. After resolving all devices, disconnected desktops are reattached with
preference for their remembered neighbor and side, without modifying saved profiles.
Native topology generation is a content fingerprint, not an ordered revision;
receivers accept a different fingerprint on a currently authenticated context even
when its numeric value decreases. Changes to the active
display or its transform invalidate control; unrelated peer readiness updates
preserve the active display and stable default desktop positions. Only Ready peers with pre-granted
sessions participate in pointer crossing.

The layout view separately presents selected devices whose actual topology has
not arrived. It preserves optimistic settings until their runtime decision,
then reconciles success or failure; configuring a layout does not enable input.
The app subscribes to the full runtime view watch, including membership-only
changes, and schedules an idle window refresh. Graph membership follows the
verified roster while legacy pairwise peers remain separately manageable.

`InputSharingSnapshot.effective_settings()` overlays a live managed activation
as temporary Controlled behavior; persistent settings remain untouched. The
coordinator resets native adapters on activation/controller identity changes,
withdrawal or effective configuration changes. Heartbeat and readiness updates
do not restart adapters. After real Accessibility and unlocked-session checks, followed by successful
creation of the native capture/injection adapters, the coordinator reports a
structured native status with the
matching activation ID. Stale adapter completion cannot confirm a replacement
activation. It never uses the legacy availability-true command for a managed
receiver. Missing required permissions are visible locally and to the controller. The
current modifying `defaultTap` uses Accessibility authorization; the separate
Input Monitoring preflight is diagnostic and does not block this input path.

Emergency reclaim pauses the activation before releasing held input and the
input lease. A local pause latch closes the gap before the runtime publishes
PausedByUser; the same activation cannot resume from a delayed Ready snapshot.
The controller must explicitly disable/re-enable or remove/re-add that receiver.
The local emergency button reaches the dedicated input worker independently of
clipboard and file work. The receiver UI offers status, system permission links
and emergency reclaim, not a persistent authorization switch. A trusted
controller can activate a receiver that it explicitly added to its layout;
Wi-Fi scene votes are advisory and cannot suppress that activation or revoke
an active receiver session. A local controller managing selected devices still
refuses incoming control to prevent mutual takeover; an idle controller with
no selected devices may yield to an explicit incoming activation without
persisting a role change. Existing scenes and layouts remain editable and
unchanged by activation.

An expired/retired activation replies with `Deactivate`, allowing the controller
to retry with a fresh activation ID and the same enable intent. It must not send
an `Activated(Unavailable)` heartbeat that indefinitely renews a dead ID. A locally
paused intent still replies `PausedByUser` and requires the explicit enable edge.

The input protocol binds its grant/token to authenticated transport identity,
controller, receiver, topology generation and lease. `Enter` makes a grant active;
frames require increasing sequence numbers and an active matching lease. The
controller keeps eligible leases warm for immediate edge switching. Renewals
have a 1,500 ms expiry and 250 ms cadence. The lease revision changes only on
actual grant/renew progress, which is the sole signal used to extend native
watchdogs. Expiry is checked before accepting newer revisions, including elapsed
time during sleep; a retired lease cannot be revived by a delayed event.
Re-reading an unchanged Ready snapshot cannot keep input suppressed.
`Leave` releases held input while preserving a warm lease; revoke/expiry destroys
it. Legacy input messages never reach the native injector.

### macOS input process boundary

The macOS bundle ships `Contents/MacOS/deskfuse-input-service` as a signed,
ordinary child process. It is not a LaunchAgent, login service or root daemon.
The main process owns UI, pairing/trust, SQLite persistence, clipboard, files and
updates. The child owns native capture/injection, permission/session/topology
observations, Wi-Fi environment observation, crossing policy, input activations,
leases and its own transport/discovery. The child constructs an input-only runtime;
it does not start the ordinary application actor or open the state database.
Its identity is read from the existing `identity.key`; a missing/invalid file
fails startup and is never regenerated by the child.

Input advertises `_deskfuse-input._udp` on UDP 42425, bound to eligible physical
LAN addresses. This is separate from the ordinary control port 42424. Main-side
trusted peer keys, membership and the ordinary control connection form the
authorization snapshot. Existing IP hints may locate port 42425, but the child
independently proves peer identity and physical on-link eligibility only after
the corresponding device control session is online. A control disconnect
releases input leases and prevents new activation until the device reconnects.
Actual membership/trust withdrawal also closes input links. Input readiness still
requires the real input connection, native permissions, activation and lease.
No clipboard/file/update stream handler is started on the input service channel.
Disposable identity probes use probe endpoints; successful probes are closed and
a fresh live connection is established on the input I/O runtime.

The 42424 endpoint, accept loop and control-session handshake run on a dedicated
control I/O runtime. They do not depend on the bridge thread that constructs the
transport, so synchronous UI, clipboard or application work cannot make the
device control listener stop answering while 42425 remains available.

Input routes are ranked independently: the saved interface preference wins,
then Thunderbolt, Ethernet and Wi-Fi. The controller checks for a better route
every five seconds even while a fallback is connected. Probing and dialing run
on the discovery executor; a failed upgrade keeps the current connection.
After the new route proves bidirectional datagram readiness, the input actor
retires the old route only when its remote-display lease is not entered. Active
drags therefore keep their route until the pointer returns locally. Activation
and input leases are then reacquired on the new authenticated connection. The
receiver follows this choice. Motion diagnostics include the actual input
local/remote addresses, interface name and kind, independently of the main
connection shown by the device network.

Three inherited Unix socket pairs separate bounded control/persistence messages,
replaceable status snapshots and urgent/lifetime signals. No motion, key, button
or wheel payload crosses this process boundary. Status backpressure blocks only
the status writer; persistence uses a bounded asynchronous writer and ACKs, never
a synchronous wait on the input actor. `persistence_pending` counts outstanding
writes; failure is reported instead of claiming durable save. A setting can take
effect locally before its durable ACK; an unacknowledged setting is not guaranteed
to survive a process failure. Debug export includes a bounded cache of child
metadata-only timing/transport samples and identifies the child PID/build.

Parent EOF or explicit quit stops native work and releases input. Closing a
window is not quitting. There is no parent heartbeat expiry: suspending or
hanging the parent does not itself invalidate healthy child input. The existing
native/remote lease watchdogs remain active. The supervisor retries crashes at
1/2/4 seconds, at most three times, then requires an explicit sharing retry.
Generation-checked publication rejects old child snapshots; old live leases and
queued native events are not transferred to a replacement process. The main
process performs only a one-shot pointer reassociation after observed child exit,
not a fallback capture engine. Helper permission results, rather than assumed
main-process TCC grants, drive input availability.

The native build/package scripts compile both binaries with the same build ID;
IPC rejects mismatches. DMG verification checks the helper's presence, arm64
architecture, signature, minimum OS and static linkage. These are build/package
checks, not two-device input or TCC acceptance. Process isolation does not remove
shared physical network, OS scheduling or hardware limits.

`InputCoordinator` runs on its own operating-system thread inside the child,
independent of clipboard/file work. Native capture wakes it on event arrival; there is no
fixed-rate mouse polling. A 20 ms timer handles only housekeeping, edge dwell
and lease checks. It composes the pure `SharingController` with native
Observe/Exclusive capture, virtual remote pointer state and a non-blocking
native injection mailbox. Capture excludes marked synthetic events.

Only adjacent motion is replaceable: capture retains the latest native position
and accumulates relative deltas; runtime, transport and injection retain the
latest absolute position within the same session/display. Button, key, wheel,
control, display and session transitions are ordering boundaries. Capture,
transport and injection coalesce at enqueue so a motion burst cannot fill their
mailboxes with old positions. Runtime drains at most 64 already-queued motion
commands per turn, validates every original position, and preserves its one
look-ahead boundary. Native capture turns are also bounded; remaining events
retain a wakeup. Command saturation fails closed rather than waiting on the
input coordinator or dropping a key/button edge. The existing reliable QUIC
protocol and increasing-sequence validation are unchanged; this does not remove
network retransmission delays.

Permission, active-session and display queries run on a separate observer
thread. The input path consumes its latest real observation; observations older
than one second (including query duration and elapsed sleep) cannot enable input.
The receiver wakes a dedicated native worker without waiting for each event's
acknowledgement. Reclaim/re-arm invalidate queued and already-popped work by
session and native generation, including re-entry into the same lease. Native
errors clear pending work, release held input and wake the coordinator. Control
boundaries still synchronize with native posting so obsolete work cannot leak
into a newly armed session. Local capture becomes exclusive only after entering a
permitted remote display; returning locally restores native cursor association
and position. Before hiding the controller cursor, macOS capture enables
`SetsCursorInBackground` on its own WindowServer connection. Quartz hide can
otherwise succeed without hiding the pointer while another app is foreground.
The private entry points are dynamically resolved; absence or rejection fails
remote entry with local input restored. Every acquired hide count is balanced
on return, error, watchdog expiry and shutdown; no per-motion hide calls or
foreground activation are needed. Injected keys/buttons are tracked so every
exit releases them.
Native capture/injection threads own additional deadlines, ensuring cursor
recovery if app/runtime scheduling stops. Permission and active desktop-session
checks temporarily gate input without overwriting the user's saved enabled
setting. Restart restores configuration and standby, never an active remote
cursor or a cursor lock.

Named input scenes persist through runtime/store commands, contain input
settings only and never change the clipboard switch. Applying a scene revokes
active leases before applying its settings. The shortcut recorder temporarily
closes input availability and stops native capture, then restores the saved
role after a valid chord or cancellation. Controlled mode reserves only the
fixed emergency chord; controller direction/lock chords pass through locally.

Incoming device-network invitations are represented by verified runtime
pending entries. `ShellState` retains their queue independently of the main
window, deduplicates request IDs and removes completed/expired invitations.
A dedicated window displays the signed owner/member names. Its approval and
rejection buttons use runtime commands; closing it rejects only that request.
Discovery cards request a connection and show an outgoing waiting state.
The device page supports automatic discovery and explicit IPv4 pairing. The
manual entry uses the standard control port 42424, proves the remote identity
with a bounded one-shot network handshake, then follows the same invitation and
recipient approval flow. IP input never grants trust or opens an application
session. Successful membership persists the signed member endpoint; legacy
user-entered address records remain excluded on load.
Forgetting a device revokes trust and deletes the authenticated row but retains
its live discovery hint. Explicit search restarts Bonjour browsing and republishes
known services, so coalesced advertisements cannot hide a removed device. Search
also clears the local device-list filter and reports that a scan was requested.
Rediscovery does not reconnect or restore trust without explicit pairing.

On macOS, `PlatformDiscovery` uses system DNS-SD Bonjour (`DNSServiceRegister`,
`DNSServiceBrowse`, `DNSServiceResolve`, and interface-scoped A-record queries)
with IncludeP2P. Other platforms retain `mdns-sd`. Service/TXT identity and SRV
port are validated before yielding IPv4 hints. Results from multiple interfaces
are merged; losing one interface does not remove a peer found on another.
Refresh preserves the event handle and registration, replays known hints, and
expires services absent from the new browse after five seconds. System errors
are surfaced and retried; shutdown joins the worker after deallocating its native
references. A successfully authenticated endpoint becomes `CachedDirect` when
Bonjour disappears, preserving a live connection and subsequent retry path.
Transport failure and trust revocation still control session lifetime.
IncludeP2P does not imply AWDL transport support: the current QUIC socket and
endpoint format remain IPv4-only. No subnet scan is performed.

A solo device can initiate joining an established network: its owner explicitly
approves and returns an owner-signed roster bound to that exact request. An
ordinary member redirects the request to the signed owner without granting
application trust. Solo rosters left by failed invitations can be replaced on
approval. Any active member can initiate an invitation or network merge from its
own device. Its signed, expiring request is relayed to the current roster signer,
which independently verifies the target and runs the normal invitation flow.
The initiating device receives the final signed result and roster, including
after a redirect to the receiving network's owner. The signer must be online and
support member initiation; failures are returned to the initiating device.
Non-members, replayed requests and locally blocked targets cannot use this path.
Two established networks merge after the receiving owner approves the combined
signed member list. The receiving
network keeps its ID and owner. Signed source-owner merge permits travel with the
persisted roster, allowing original members (including offline members and later
merge chains) to authenticate migration without resetting their local identities.
Local blocks remain in effect; the combined network still has a 32-device limit.
A submitted decision is only marked processing until `PairingFinished` or the
verified pending snapshot confirms completion. A request-scoped decision error
restores its retry actions; queue delivery alone never resolves an invitation.
An integrity error in persisted membership fails closed for automatic clipboard
and input candidates. Leaving the network clears that membership through the
runtime while retaining local input settings, scenes and display placements.

### Display metadata with sharing disabled

Screen layout configuration uses authenticated display metadata independently of
input permissions. A configured controller contacts only selected participants;
a peer may reply with its own topology on the same verified physical LAN
connection while its input role is disabled. This does not authorize a lease or
input injection. To verify this on two Macs, run `deskfuse-input-probe --role
receiver --run-id metadata-check --metadata-only` on the receiver, then run the
same binary with `--role controller --run-id metadata-check --metadata-only
--peer DEVICE_ID@IP:43424`, using the printed receiver identity and its LAN IP.
Both sides retain disabled sharing and native availability; the receiver has no
authorized controller. Success requires the exact synthetic topology from each
peer, verified LAN contexts, independent two-way receipt/completion messages,
and zero input leases or input events. Identities and trust remain in memory;
the default probe mode still exercises input sessions instead. This metadata
probe does not prove actual native capture or injection.

### Physical acceptance still required

Use two or more Macs with actual displays and TCC permissions. Verify a trusted
LAN session and each displayed activation/permission reason, controller enable/disable and receiver emergency pause,
negative-position/Retina/rotated monitors, immediate/dwell/double-touch edges,
corner exclusions, modifier and drag fidelity, directional and emergency
shortcuts, and device-group layout persistence across restart. While holding a
remote button or modifier, disconnect the cable/Wi-Fi, sleep/lock, revoke TCC,
restart a process and replace a display; every case must restore local input
and release held remote input within the lease bound. Repeat during large
file transfers. A screenshot or passing domain/runtime test is not proof of
these native multi-Mac behaviors.

## Native macOS release gate

The native Swift executable remains `deskfuse-app`, packaged as `DeskFuse.app`
with `com.deskfuse.app`. SwiftUI owns the main application event loop, Dock,
menu-bar extra, windows and Settings. Closing the main window keeps the engine
running; application termination releases input and joins the engine before exit.
The FFI layer prevents two engines in one process; the client activates an
already-running same-identity application instead of starting a second engine.

Build with `scripts/build-native-macos.sh`; the existing build/run and DMG
wrappers call it. Rust static libraries and the update helper rebuild the matching
standard library for macOS 14 through `scripts/cargo-macos.sh`, avoiding a newer
minimum OS inherited from a Homebrew standard library. Matching rust-src and a
SwiftUI-capable Xcode SDK are required. Xcode selection is process-local.
SwiftUI startup confirmation replaces the old rendering-framework self-test;
self-test mode performs storage checks and shows the native window without
starting business writes, clipboard capture or networking before acknowledgement.

## Deskflow clean-room policy

The project may use public behavior and public interoperability observations
as requirements. It must not copy, link, or translate GPL Deskflow source into
this repository. Protocol and behavior work is implemented independently with
our own tests and documentation. This is a development policy and should be
reviewed by the project owner for licensing questions.

## M0/M1 status and evidence

### M0 (root bootstrap)

Implemented at the root: member inventory, exact dependency pins, Rust 1.97.1
configuration, lints, package metadata, CI entrypoint, and documentation.
The root deliberately does not fabricate a Cargo lockfile or run Cargo when
bootstrapping on a machine without rustup.

### M1 (device-network skeleton)

The intended M1 evidence is member-crate compilation/tests, app-shell coverage, local discovery/pairing seams, and a launchable local app shell. The
root can verify only workspace shape and script/config syntax until all member
crates are present. M1 is not complete merely because `cargo check` passes:
authenticated two-Mac behavior, route evidence, and a mounted DMG launch are
separate gates.

## Windows and LAN updates

See [the confirmed cross-platform implementation contract](WINDOWS_AND_LAN_UPDATES.md).
Windows 11 x64, native session/service integration, and permanently automatic
signed LAN updates are tracked in this scope; macOS now connects distribution, installation, self-test and rollback. The contract separates implemented work
from pending platform/build and user-owned physical-device validation.


### Windows native desktop checkpoint (2026-09-13)

`apps/deskfuse-windows` is an ordinary-user WPF desktop with tray, local history,
clipboard previews, device pairing and a local file shelf. The service remains the
only network/identity owner. Bounded desktop v2 IPC binds user SID/logon LUID,
console session and generation; clipboard-sync-v2 receipts follow actual native
clipboard writes. An isolated input-only runtime advertises `_deskfuse-input._udp`
on UDP 42425, while a GUI-supervised ordinary-user helper performs only Default-
desktop injection with native lease expiry and emergency reclaim. This checkpoint
supports Mac controller to Windows receiver on the unlocked ordinary desktop;
Windows controller capture, files, elevated/secure-desktop input and the Windows
updater remain unavailable. See [Windows client implementation matrix](WINDOWS_CLIENT.md) and
[desktop IPC](WINDOWS_DESKTOP_IPC.md). Local build/package evidence is not
Windows installation, Explorer or two-machine acceptance.

### Windows native adapter checkpoint

`deskfuse-platform` owns shared clipboard/geometry/permission-snapshot/session types;
`deskfuse-platform-windows` owns Win32 clipboard, display/session probes and ordinary
desktop HID input adapters. `deskfuse-identity::WindowsServiceIdentityProvider` is
service-only (LocalSystem, session 0) and is deliberately not selected by the GUI.
The app clipboard worker now uses `platform::create_worker_pasteboard`; Windows
clipboard/pairing business IPC is connected. Windows capture/injection implement
the shared platform traits, including bounded fail-open suppression, stable monitor
mapping and ordinary-desktop injection. The installed product currently wires only
the receiver half through the service-owned input-only runtime and user helper;
Windows controller composition remains pending. Service network ownership is implemented in the checkpoint below.
See section 10 of the Windows contract for unsupported native cases and the
read-only `scripts/check-windows-native.ps1` validation entry point.

### Windows service ownership checkpoint (2026-09-06)

`apps/deskfuse-service` owns SCM lifecycle, the machine DPAPI identity, protected
SQLite state/trust, QUIC and mDNS. `WindowsServiceDataDirectory` retains directory
handles and an exclusive runtime lease until network shutdown. The original service checkpoint only advertised `NETWORK`; the desktop extension adds acknowledged
clipboard on the control runtime and ordinary-desktop receiver input on the isolated
input runtime. File and updater capabilities remain unavailable.
The fixed, local-only Named Pipe exposes bounded desktop commands and the helper's
session-bound input stream. SCM PID verification, concrete client data rights and per-request token/logon-LUID,
console/unlock and epoch checks guard the boundary. The control runtime advertises
clipboard only; input uses a separate 42425 runtime and never receives clipboard,
file or updater commands. File and updater capabilities remain unavailable.
See [service checkpoint](WINDOWS_SERVICE_CHECKPOINT.md) for the historical two-EXE
package boundary; that development ZIP still excludes the WPF client and input helper.

### 多接口自动选路

设备网络名单里的地址只是候选地址，同一 `host:port` 只保留一条。可信设备重连时收集发现及缓存中的 IPv4 地址；非 macOS 后端还解析其发布的稳定 Bonjour 主机名，macOS 使用系统 Bonjour 按接口解析出的地址。拨号前对去重后的 IPv4 做短超时身份探测，记录握手延迟；自动模式选实测最快的可达地址，无测速结果时仍按雷电、有线、Wi-Fi 排序。设备详情中的优先路径按设备持久化，只提升对应物理接口，不会被上次回程 IP 盖掉，路径不可用时仍回退。刷新会重新探测，已连接时若出现明显更快或更符合偏好的线路则切换。每次拨号继续验证预期设备身份，地址发现不授予信任。控制连接成功后记录实际 IP，更新通道优先跟随该接口。独立键鼠服务在 42425 端口自行验证和选择物理线路，自动模式优先雷电，并定期准备更优线路，在鼠标返回本机后切换；不能用主连接线路代替鼠标线路。单个控制路径握手最多等待 3 秒，再尝试下一候选地址。

### 按 Wi-Fi 场景选择键鼠主控

当前 Wi-Fi 名称通过 macOS CoreWLAN 读取；用户在启动授权步骤或「设置 → 权限」中申请系统定位授权，仅为读取 SSID，不启动定位采集。读取任务独立于 UI 与输入处理，5 秒更新一次。可信物理局域网内支持 `INPUT_ENVIRONMENT` 的设备交换主控意图与 Wi-Fi 场景，供有线设备识别唯一场景；SSID 不是身份或授权依据。

`INPUT_ENVIRONMENT` 使用独立的 bit 10；鼠标 Datagram 的 `INPUT_MOTION_DATAGRAM_V1` 保持 bit 9。协商到鼠标 Datagram 不代表支持主控场景消息，只有双方明确支持场景能力时才能发送这类消息。

首次双主控或规则冲突在 Wi-Fi 场景页面提示用户选择，但场景投票不阻断主控对排布中明确加入的设备发起激活，也不撤销已经建立的被控会话。明确选择和删除规则同步到当前在线的兼容设备；离线设备不承诺已同步，其旧规则冲突时仍需选择。规则随激活策略持久化，原有主控设置、参与设备和显示器布局不被覆盖。本机已开启主控时，远端的选择与投票不会自动挂起本机主控角色。SSID 变化稳定 10 秒且鼠标返回本机后应用；短暂连接中断保留主控意图 12 秒，原生输入控制租约仍独立及时失效并释放按键。保存的主控离线时不自动抢控，可临时选择其他主控。绑定到当前 SSID 的布局场景会在该网络稳定后自动恢复排布，仅恢复屏幕排布，不改变共享开关、参与设备或授权。

陌生网络、SSID 读取失败、多个不同 SSID 或矛盾规则提供本次选择入口；有线设备可依据可信 Wi-Fi 成员的唯一 SSID 匹配场景。相同名称网络上的规则仍只能选择已配对设备。旧客户端继续使用原激活协议，场景协商需相关设备安装支持此能力的版本。
