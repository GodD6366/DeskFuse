# Migration from the engine-shaped tree

This project is still in development. The collaboration refactor does **not**
preserve on-disk compatibility. Reset local state and re-pair devices after
switching to the split stores.

## What changed

| Old | New |
| --- | --- |
| `deskfuse-engine` owns clipboard, files, library, workflow, LAN web, updates | `deskfuse-collaboration` owns K3/K5/K6/K7; extensions are separate crates |
| Engine depends on `deskfuse-platform-macos` and `axum` | Hosts inject Host Ports; `axum` lives in `deskfuse-lan-web` |
| `deskfuse-runtime` always links `deskfuse-update` | `deskfuse-runtime` feature `updates` (on by default for the current app) |
| `scripts/cargo-macos.sh` rebuilt std with rust-src | Ordinary `cargo` plus `MACOSX_DEPLOYMENT_TARGET=14.0` |
| Bindgen binary inside `deskfuse-ffi` | `tools/deskfuse-bindgen` has no business crate dependency |
| One SQLite schema for peers and clipboard history | `deskfuse-store-core` and `deskfuse-store-library` |

## Still on the production path

The shipping macOS app still links `deskfuse-ffi` → `deskfuse-engine`. The desktop
host (`deskfuse-engine::host::EngineHost`) injects Host Ports at startup. Remote
native clipboard applies in `TokioBridge` go through `apply_clipboard` (revision
check, remote-origin mark, superseded-by-local-copy). File clipboard frames and
the wire protocol still live in the engine worker until they call
`CollaborationCore`.

`scripts/build-rust-macos.sh` builds the FFI library, the input service and the
Swift bindings without compiling Swift. Cargo owns Rust incrementality, and the
bindings are cached in
`apps/deskfuse-macos/SDK/.swift-bindings-fingerprint.json` by hashing the built
library, the bindgen binary, the configuration and the generated files. Force
recompilation and regeneration with `DESKFUSE_FORCE_RUST=1` or
`./scripts/build-client-macos.sh --rebuild-rust`.

The new input binaries (`deskfuse-input-macos`, `deskfuse-input-windows`) compile
with the input-only graph; they do not yet replace `deskfuse-input-service`.

## Local data

- Core state: devices, trust, config, transfer checkpoints.
- Library state: `cl_*` tables, never mixed back into the core database.
- Identity remains `~/Library/Application Support/DeskFuse/identity.key`.
  Do not delete it to “fix” pairing.

## Verify

```bash
./scripts/check-architecture.sh
./scripts/verify.sh
./scripts/build-sdk.sh
```
