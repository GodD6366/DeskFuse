# Isolated input protocol probe

`unidesk-input-probe` validates authenticated input transport between two Macs.
It uses a fresh random identity, `MemoryTrustStore`, `MemoryRuntimeStore`, no
mDNS and independent ports. It never opens Keychain, modifies real pairings,
reads/writes the system pasteboard, captures input or injects native events.
The two display topologies and event payloads are explicit test fixtures.

Build from the workspace root:

```sh
cargo build -p unidesk-probe --bin unidesk-input-probe --offline --locked
```

Copy only the produced executable to the other Mac when authorized. Start the
receiver first (example destination path and LAN address):

```sh
/tmp/unidesk-input-probe --role receiver --run-id lan-20260905-a --bind 0.0.0.0:43424 --rounds 3 --frames 32 --timeout 90
```

The receiver prints `DEVICE_ID=<random ID>`. On the controller, use that exact
ID and the receiver's physical LAN address:

```sh
./target/debug/unidesk-input-probe --role controller --run-id lan-20260905-a --bind 0.0.0.0:43425 --peer RECEIVER_DEVICE_ID@192.0.2.20:43424 --rounds 3 --frames 32 --timeout 90
```

The new signed invitation exchange supplies the receiver with the controller's
identity and listener routes. Receiver discovery metadata is therefore not
required. `--peer-file PATH` remains available as an optional isolated discovery
fixture; it accepts a single public `DEVICE_ID@IP:PORT` line and never grants
trust. Both sides still require normal signed Hello verification and Connected
state. The controller explicitly sends `RequestPairing`; only the receiver
approves a matching `incoming` pending invitation in the runtime's verified
snapshot. The initiating controller does not approve itself.

Both sides must use the same run ID, round count and frame count. The probe only
approves a peer whose signed display name has the same run ID and opposite test
role; the controller also pins the receiver device ID from `--peer`. The
receiver keeps its persistent input settings at their default: disabled, role
`Disabled`, and no authorized controller. The selected controller starts an
independent activation lease; the headless receiver then reports a matching
activation ID as native-ready. This readiness is an explicit protocol fixture,
not a claim about macOS permissions. The receiver verifies that its settings and
scenes remain unchanged through completion. Run IDs are test scoping labels,
not passwords or permanent identities.

Use `--metadata-only` on both processes to exercise layout discovery while both
sharing and native availability stay disabled. This mode does not report managed
readiness or run normal input configuration. It requires each exact remote
topology, a physical LAN context, two-way receipt/completion fences, and zero
input leases or input events.

Each round waits for a runtime-approved physical LAN context (Wi-Fi, Ethernet
or Thunderbolt), requests a lease and enters an alternating fixture display.
The controller sends known motion/button/key/repeat/modifier/scroll frames. The
receiver checks exact contents and strictly increasing sequence numbers;
renewal messages may make sequence numbers non-contiguous. The controller also
requires an increased lease revision during delivery. `Leave` must release
active input while retaining the warm lease; `Release` must remove the session.
A subsequent old-session frame must not produce any receiver event. Repeating
three rounds requires three distinct session IDs and exercises stream/session
cleanup. An explicit completion fence is acknowledged in both directions.

Receiver evidence is returned through a dedicated application MIME message on
the authenticated control channel. This only acknowledges input events already
validated at the receiver; no system clipboard adapter is used. A sender enqueue
or local Ready snapshot is never counted as delivery evidence.

Success requires **both processes** to exit zero and print `RESULT PASS`.
`LAN` and `RECEIVE_ENTER` lines include actual connection context, interface and
session identity. `ACK` lines include receiver frame/event counts and final
sequence. Retain both logs. A failure exits nonzero and prints `RESULT FAIL`.
Physical native input/TCC behavior and UI acceptance remain separate gates.

Local checks:

```sh
cargo test -p unidesk-probe --bin unidesk-input-probe --offline --locked
cargo clippy -p unidesk-probe --bin unidesk-input-probe --all-targets --no-deps --offline --locked -- -D warnings
```

## Recipient-window rejection acceptance

To exercise a real app's approval window without granting it a new peer, pin the
app's real Device ID and reachable listener address, then run:

```sh
./target/debug/unidesk-input-probe --role controller --run-id ui-reject-20260905 --bind 0.0.0.0:43425 --peer APP_DEVICE_ID@192.0.2.10:42424 --expect-rejected --timeout 90
```

Choose **拒绝** (or close the invitation window) in the target app. This mode
never configures input sharing or sends clipboard/input payloads. It requires a
matching signed `PairingFinished` rejection, verifies that the peer was neither
trusted nor blocked, and checks that it did not join the probe's network. An
acceptance, expiry, timeout or protocol error fails the run. `RESULT PASS`
includes `expected_rejected=true trusted=false blocked=false`.
