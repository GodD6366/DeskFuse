# uniDesk

[中文](README.md) · [Contributing](CONTRIBUTING.md) · [Releasing](docs/RELEASING.md) · [TODO](TODO.md)

A macOS-first native desktop collaboration app (Rust 2024 + SwiftUI): sync clipboards, transfer files, stage and drag out files, and optionally share one keyboard and mouse between trusted devices. See the current [interface and behavior conventions](design/current/README.md).

> Early prerelease: macOS ships an Apple Silicon test build, Windows is an experimental implementation; a passing build is not two-device acceptance.

## What it does

- Discover or manually pair trusted devices over IPv4; connection state and blockers stay visible.
- Sync text, images and native clipboard formats.
- Transfer files on demand, with optional early transfer for small files; names and folder structure are preserved, and the history cache has a size limit and cleanup receipts.
- A file shelf: drag files in to stage them, drag them out in batches, or send them to another device.
- A separate keyboard/mouse sharing workspace: real display layout, device activation, permission state and emergency reclaim.

Behavior is defined by the [file transfer contract](docs/FILE_TRANSFER.md), [architecture](docs/ARCHITECTURE.md) and [roadmap](docs/ROADMAP.md).

## Compared with macOS Continuity and Deskflow

| Feature | macOS Continuity | Deskflow | uniDesk |
| --- | --- | --- | --- |
| Keyboard/mouse crossing | ✅ same Apple Account only | ⚠️ screen positions cannot be freely arranged | ✅ |
| Clipboard sync | ✅ relies on iCloud, often unstable | ⚠️ basic text and small images only | ✅ |
| File transfer | ✅ AirDrop | ❌ | ✅ folders supported |
| Staging shelf | ❌ | ❌ | ✅ |
| Clipboard history | ❌ | ❌ | ✅ cross-device sync, image content search |
| Encrypted transport | ✅ | ✅ TLS | ✅ QUIC |
| Open source | ❌ | ✅ GPL-2.0 | ✅ MIT / Apache-2.0 |

Sources (2026-09-15): [Apple Universal Control](https://support.apple.com/en-us/HT212757), [Universal Clipboard](https://support.apple.com/en-us/102430), [Deskflow](https://github.com/deskflow/deskflow) (drag-and-drop removal in [#8459](https://github.com/deskflow/deskflow/issues/8459)).

## Platforms

| Platform | Status |
| --- | --- |
| macOS 14+, Apple Silicon | Main platform, arm64 DMG |
| macOS 26+ | Native Liquid Glass for navigation and actions; standard materials on older systems |
| Intel Mac | No release, no compatibility claim |
| Windows x64 | Native adapters and service checkpoint; no GUI, installer or full cross-platform collaboration |
| Linux | Not a first-release target |

## Download and install

Downloads: [GitHub Releases](https://github.com/GodD6366/uniDesk/releases).

### Homebrew (macOS 14+, Apple Silicon)

```sh
brew trust --cask godd6366/unidesk/unidesk
brew tap godd6366/unidesk https://github.com/GodD6366/uniDesk
brew install --cask godd6366/unidesk/unidesk
sudo xattr -rd com.apple.quarantine "/Applications/uniDesk.app"
```

Run `brew trust` once (skip it on older Homebrew without the command). The cask pins the matching DMG SHA-256. Uninstall with `brew uninstall --cask godd6366/unidesk/unidesk`, then `brew untap godd6366/unidesk` if unused.

### Updates

Public Homebrew / GitHub packages check GitHub Releases (including prereleases) in Settings → Updates and link to the matching release. For Homebrew installations, run:

```sh
brew update && brew upgrade --cask unidesk
```

Internal signed packages retain trusted-LAN updates. Public packages do not use that installer. Older builds without GitHub checks need an initial update through Homebrew or a manually downloaded DMG.

### Manual DMG install

1. Download `uniDesk-arm64.dmg`, `SHA256SUMS`, `release.json` and `build-info.json` from the same release, then run `shasum -a 256 -c SHA256SUMS`.
2. Open the DMG and drag `uniDesk.app` into Applications; run only one instance per device.
3. Allow local network access when prompted; input sharing also needs Accessibility permission, a trusted LAN session, enabled devices and a valid display layout.

The build is **ad-hoc signed and not notarized**, so macOS may block it: verify the origin and checksum, then use the system's “Open Anyway” option if you choose. Never disable Gatekeeper or SIP. The Homebrew commands above already include the quarantine removal step.

## Build from source

Requires an Apple Silicon Mac, full **Xcode 27+ (macOS 27+ SDK)**, Rustup and Python 3.11+; Command Line Tools alone are not enough. The deployment target is macOS 14, with Rust and dependencies pinned in `rust-toolchain.toml` and `Cargo.lock`.

```sh
git clone https://github.com/GodD6366/uniDesk.git
cd uniDesk
rustup show active-toolchain
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh
```

Without a signing certificate, add `CODESIGN_IDENTITY=-` explicitly; add `--build` to build without launching, and select a specific Xcode per command with `DEVELOPER_DIR=...`. For a standalone test DMG, run `CODESIGN_IDENTITY=- ./scripts/package-macos.sh --bootstrap`, which writes to `dist/bootstrap/` and takes no part in LAN updates.

## Layout

```text
apps/           SwiftUI app, Windows service, diagnostic probes
crates/         Domain, protocol, identity, storage, network, transfer, input and platform modules
scripts/        Development, checks, asset generation, packaging and release tools
Casks/          Public Homebrew cask (also the tap)
resources/      Source SVG, bundle metadata, third-party icons, public transfer guides
design/current/ Current interface behavior and icon resources
docs/           Architecture, feature contracts, release and support boundaries
.github/        CI, prerelease workflow, contribution templates and dependency updates
```

`target/`, `dist/`, `artifacts/` and historical designs are not part of the public source.

## Known limitations

- The Windows service package is not a GUI or installer; real SCM, session switching and cross-platform features need separate acceptance.
- Native Finder paste, drag-out, permission changes, Wi-Fi input crossing and real two-device behavior cannot be replaced by CI.
- GitHub CI does not sign LAN update payloads; the coordinator build supports automatic macOS LAN updates, and older versions need one manual base install, see [RELEASING.md](docs/RELEASING.md).
- Logs contain metadata, state and hashes only; check attachments before filing issues and never upload clipboard contents, device keys or credentials.

Report vulnerabilities according to [SECURITY.md](SECURITY.md).

## License

Original code is dual-licensed under **MIT or Apache-2.0**, at your option; see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE). Dependencies and third-party assets retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
