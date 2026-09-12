# Mouse motion transport A/B probe

`unidesk-motion-benchmark` compares one persistent, reliable, ordered QUIC uni stream
with QUIC Datagrams plus a final reliable position snapshot. Both runs create a fresh
loopback Quinn connection, use the same deterministic absolute-position fixture,
requested sampling frequency and impairment policy.

## Scope

- Drops **real UDP packets**, including QUIC retransmissions and reverse ACK traffic,
  in a local UDP relay. Does not merely discard decoded motion messages.
- Each mode runs sequentially (reliable first, hybrid second). Handshake completes
  without impairment; fixed one-way delay and seeded random loss are enabled for
  motion and final-snapshot traffic. Optional periodic bursts drop both directions.
- Same-process monotonic timestamps make observed sample age meaningful here. This
  is **not** synchronized-clock, real Wi-Fi, native injection or physical two-Mac
  evidence. Production capability/session/epoch/permission logic is not exercised.
- Deliberately uses a 33-byte synthetic envelope, not production protobuf frames.
  This isolates transport behavior, not actual protocol serialization overhead.
- JSON contains configuration, packet counters and aggregate timing only, never
  coordinates, trajectory samples, clipboard content, key values or credentials.
- A seed reproduces the pseudorandom loss decisions, **not** identical QUIC
  packetization, connection congestion state or OS scheduling. Use repeated seeds
  and alternate/external repetitions for performance conclusions; one result is not
  a universal benefit guarantee.

## Commands

```sh
cargo test -p unidesk-probe --bin unidesk-motion-benchmark
cargo build -p unidesk-probe --bin unidesk-motion-benchmark
./target/debug/unidesk-motion-benchmark --help

# Baseline and loss contrasts: capture stdout as validation JSON if required.
./target/debug/unidesk-motion-benchmark --loss-percent 0 --duration-ms 5000 --rate-hz 125 --delay-ms 5 --seed 42
./target/debug/unidesk-motion-benchmark --loss-percent 1 --duration-ms 5000 --rate-hz 125 --delay-ms 5 --seed 42
./target/debug/unidesk-motion-benchmark --loss-percent 3 --duration-ms 5000 --rate-hz 125 --delay-ms 5 --seed 42

# 80ms burst interruption each second, first burst at t=1000ms.
./target/debug/unidesk-motion-benchmark --loss-percent 1 --burst-every-ms 1000 --burst-ms 80 --duration-ms 5000 --seed 42
```

All network addresses bind to IPv4 loopback. No system network changes, native
input privileges, persistent identity stores or installed app launches are used.
The test certificate is generated and pinned only within the process.

## Parameter bounds

| Flag | Default | Accepted |
| --- | --- | --- |
| `--loss-percent` | 0 | finite 0 through 30 |
| `--duration-ms` | 5000 | 100 through 120000 |
| `--rate-hz` | 125 | 1 through 1000 |
| `--delay-ms` | 5 | 0 through 250 per direction |
| `--seed` | 42 | unsigned 64-bit integer, including zero |
| `--burst-every-ms` | 0 (off) | at most 120000; must exceed burst duration |
| `--burst-ms` | 0 (off) | at most 2000; both burst options must be nonzero to enable |

Each run permits up to an additional 15 seconds for reliable final convergence.
Handshake has an independent 10-second timeout. Queue overflow is reported and
fails the correctness gate rather than silently biasing results.

## Reading output

`sample_age` is elapsed monotonic time from creating a synthetic motion sample to
its receiving task publishing that sample. `motion_update_gap` measures intervals
between accepted (newer) **motion** arrivals; it excludes the final snapshot, so
`first_update_ms` and `final_motion_to_snapshot_ms` separately expose boundary stalls.
All distributions have sample counts, p50/p95/p99/max in milliseconds. Empty
populations are `null`, not zero. Receiver ordering accepts only increasing motion
sequence numbers. A late reliable snapshot cannot rewind newer motion.

`final_position_converged` requires receipt of the final reliable snapshot and the
fixture's final sequence/position. Snapshot transmission occurs 32ms after the
last motion. Random/burst loss of the final motion is therefore recoverable.
Snapshot correction is excluded from motion sample-age and inter-update histograms.

`passed_correctness` requires both modes to converge, no invalid samples and no
relay overflow. A successful run exits 0; a convergence/integrity failure emits
its JSON then exits nonzero. Connection/setup failures are errors, not fabricated
zero-loss or zero-latency results. This correctness gate is not a performance gate:
compare p99 and longest gaps explicitly, and retain results with no improvement.

## Production-path headless input probe

The separate existing `unidesk-input-probe` now consumes ordered input and its
lifecycle edges from `input_event_stream()`, while continuing pairing/network and
clipboard receipt handling through the ordinary runtime stream. Lifecycle events
are filtered out of the ordinary stream to avoid duplicate or reordered releases.
A delivery invalidation fails the probe and revokes its headless input availability.

Both sides may opt in using `--hybrid`. This advertises
`INPUT_MOTION_DATAGRAM_V1` and requires the resulting lease to report `hybrid`;
a reliable fallback does **not** count as a hybrid PASS. Without the flag the
previous capability set and exact reliable frame receipt checks remain enabled.

```sh
cargo build -p unidesk-probe --bin unidesk-input-probe
# Receiver on the intended trusted physical LAN host:
./target/debug/unidesk-input-probe --role receiver --run-id motion-check --bind 0.0.0.0:43424 --rounds 3 --frames 32 --hybrid
# Copy DEVICE_ID from receiver stdout; controller on the other host:
./target/debug/unidesk-input-probe --role controller --run-id motion-check --bind 0.0.0.0:43425 --peer RECEIVER_DEVICE_ID@RECEIVER_LAN_IP:43424 --rounds 3 --frames 32 --hybrid
```

Do not substitute loopback for physical-LAN acceptance. The production runtime
retains input eligibility checks. This probe still uses synthetic native-ready
fixtures: even a real two-host PASS is protocol evidence, **not native permission,
Finder or cursor-injection acceptance**.

Hybrid drag fixtures send button-down/state transitions, a standalone absolute
motion, then button-up with 20ms between commands, allowing the Datagram worker to
run before the release anchor supersedes pending movement. Receiver validation
checks flattened reliable input order and the precise pointer anchor for every
transition, independently of datagram motion sequence numbers. Reliable anchors
and final snapshots may create extra position deliveries; receipt `event_count`
counts original logical fixture events rather than these transport corrections.
Pure motion cannot advance the receiver's reliable sequence watermark. Modes have
different fixture batching, so use the A/B benchmark above for rate-matched timing,
not this functional probe.
