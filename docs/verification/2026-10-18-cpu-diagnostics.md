# CPU optimization and diagnostic coverage (2026-10-18)

## Scope

This change reduces background work without reducing mouse polling rate or input delivery rate. Existing unrelated worktree changes remain untouched. A lower CPU measurement does not independently prove physical pointer smoothness.

- Clipboard maintenance: partial index for pending large SQLite blobs; idle polling 25 ms -> 1 s, explicit wakeups remain immediate. Active work retains short bounded slices. Index creation yields to foreground work.
- Swift UI: stop constructing/animating the latency chart when its window is occluded or its page is hidden. Continue backend capture, snapshots, synchronization and errors. Ignore only fully identical replayed snapshots.
- Mouse configuration: retain 1-second synchronization, reuse unchanged decoded JSON and skip redundant HID configuration/state publication.
- HID: suppress duplicate button state processing and successful-write no-op tasks. Unknown receiver slots back off from 30 to at most 60 seconds; explicit detection retries immediately. Keep known-device monitoring and G502 renewal.
- Input queue: notify on empty-to-nonempty transition; consumer re-arms residual queue. No event rate reduction or removal of native position reads.
- Network: offline periodic roster synchronization backs off 5/10/20/30 seconds; manual actions and independent control recovery remain separate. Healthy roster synchronization remains unchanged for trust/revocation propagation.

## Logs for future incidents

Existing hourly JSONL files remain in `~/Library/Logs/DeskFuse`, retained for seven days. Engine records now carry PID and package version; app and input service can be distinguished even when writing the same hourly file.

- `clipboard_maintenance / interval_summary`: 30-second counts, busy time, maximum cycle time, failures, yield counts and safe failure category. Info follows Debug logging; failures remain warnings.
- `input_timing / capture_health`: 10-second counts for callbacks/motion/synthetic filtering, tap timeouts/disablement/panics, and 1-in-64 callback duration samples. Counts are not input contents. A maximum here is the sampled maximum, not the maximum of every callback.
- Input timing ring is incrementally persisted every 10 seconds, off the input callback. Sequence numbers support correlation/deduplication; `input_timing_loss` reports ring drops/missing entries. The newest interval may not be persisted after an abrupt termination. Queue overflow and write failures are not silently treated as complete logging.
- Background roster/scan events record peer, reason, timing/backoff and scan totals without one log line per failed address.
- Native mouse/HID state changes and failures use macOS unified logging (`subsystem == "cc.deskfuse"`, categories `mouse_buttons` and `logitech_hid`). Exported mouse snapshot includes bounded recent actions and cumulative request/failure/duplicate counters.
- Exported memory diagnostics include selected page and visible window count, alongside PID, CPU/RSS/footprint and OCR diagnostics. This is export-time context, not historical UI state.

No per-motion JSONL, coordinates, typed key stream or clipboard bodies are added. Missing/sampled records must not be interpreted as proof that an incident did not happen.

## Reproduction protocol

Record app build/PID, affected machine, local vs remote control, connection mode, visible page/window state, and incident time. Compare old/new under the same conditions after startup work settles:

1. Idle with main window closed (not quitting).
2. Local physical mouse movement.
3. Remote physical mouse movement, including leaving/returning to local control.
4. Input graph visible vs occluded.

Use interval CPU deltas for app and input service separately, plus RSS, capture health and injection/network timings. Do not compare measurements taken during compilation with quiet-system samples. Use a physical keyboard/button acceptance check separately from unit tests.
