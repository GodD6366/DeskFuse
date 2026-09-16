# Migration from the engine-shaped tree

This project is still in development. The collaboration refactor does **not**
preserve on-disk compatibility. Reset local state and re-pair devices after
switching to the split stores.

## What changed

| Old | New |
| --- | --- |
| `unidesk-engine` owns clipboard, files, library, workflow, LAN web, updates | `unidesk-collaboration` owns K3/K5/K6/K7; extensions are separate crates |
| Engine depends on `unidesk-platform-macos` and `axum` | Hosts inject Host Ports; `axum` lives in `unidesk-lan-web` |
| `unidesk-runtime` always links `unidesk-update` | `unidesk-runtime` feature `updates` (on by default for the current app) |
| `scripts/cargo-macos.sh` rebuilt std with rust-src | Ordinary `cargo` plus `MACOSX_DEPLOYMENT_TARGET=14.0` |
| Bindgen binary inside `unidesk-ffi` | `tools/unidesk-bindgen` has no business crate dependency |
| One SQLite schema for peers and clipboard history | `unidesk-store-core` and `unidesk-store-library` |

## Still on the production path

The shipping macOS app still links `unidesk-ffi` → `unidesk-engine`. That worker
is the next cut: clipboard/file/input loops should call
`unidesk-collaboration` instead of the in-engine copies. The new input binaries
(`unidesk-input-macos`, `unidesk-input-windows`) compile with the input-only
graph; they do not yet replace `unidesk-input-service`.

## Local data

- Core state: devices, trust, config, transfer checkpoints.
- Library state: `cl_*` tables, never mixed back into the core database.
- Identity remains `~/Library/Application Support/uniDesk/identity.key`.
  Do not delete it to “fix” pairing.

## Verify

```bash
./scripts/check-architecture.sh
./scripts/verify.sh
./scripts/build-sdk.sh
```
