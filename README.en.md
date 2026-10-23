# DeskFuse

DeskFuse has its own application identity, protocol and data directory. First launch offers an optional clipboard history import, followed by a separate option to trash the previous app while preserving its data. Previous protocols are not supported. The target release repository is `GodD6366/deskfuse-release`. Until the first public release, the download and Homebrew instructions below are release preparation, not available installers.

[中文](README.md) · [Contributing](CONTRIBUTING.md) · [Releasing](docs/RELEASING.md) · [TODO](TODO.md)

A macOS-first native desktop collaboration app (Rust 2024 + SwiftUI): sync clipboards, transfer files, stage and drag out files, and optionally share one keyboard and mouse between trusted devices. See the current [interface and behavior conventions](design/current/README.md).

> Early prerelease: GitHub ships macOS arm64 and Intel installers; Windows support is Beta, and a passing build is not two-device acceptance.

## What it does

- A two-step first-run setup: guided system permissions first, then feature defaults. Clipboard, keyboard/mouse sharing and remote applications start on; mouse buttons and microphone stay off, and every feature has a demo animation. It also runs on an existing installation until that Mac completes the current setup.
- Discover or manually pair trusted devices over IPv4; connection state and blockers stay visible. Permissions are requested on demand after setup.
- Sync text, images and native clipboard formats; history groups by type and supports manual lists.
- One file transfer page with a send area, the transfer queue and phone QR sharing; files transfer on demand, names and folder structure are preserved, and the history cache has a size limit and cleanup receipts.
- A file shelf: drag files in to stage them, drag them out in batches, or send them to another device.
- Keyboard/mouse sharing workspace: real display layout, device activation, permission state and emergency reclaim; paired online Macs join automatically.
- A health indicator in the sidebar footer summarizes anything that needs attention, and errors stay until handled or dismissed.
- Workflows are an advanced feature, off by default and enabled from Settings → Advanced.
- Beta: remote applications (macOS windows and Windows RemoteApp).

Behavior is defined by the [file transfer contract](docs/FILE_TRANSFER.md), [architecture](docs/ARCHITECTURE.md) and [roadmap](docs/ROADMAP.md).

## Compared with macOS Continuity and Deskflow

| Feature | macOS Continuity | Deskflow | DeskFuse |
| --- | --- | --- | --- |
| Keyboard/mouse crossing | ✅ same Apple Account only | ⚠️ screen positions cannot be freely arranged | ✅ |
| Clipboard sync | ✅ relies on iCloud, often unstable | ⚠️ basic text and small images only | ✅ |
| File transfer | ✅ AirDrop | ❌ | ✅ folders supported |
| Staging shelf | ❌ | ❌ | ✅ |
| Clipboard history | ❌ | ❌ | ✅ cross-device sync, image content search |
| Encrypted transport | ✅ | ✅ TLS | ✅ QUIC |
| Open source | ❌ | ✅ GPL-2.0 | ❌ Private source |

Sources (2026-09-15): [Apple Universal Control](https://support.apple.com/en-us/HT212757), [Universal Clipboard](https://support.apple.com/en-us/102430), [Deskflow](https://github.com/deskflow/deskflow) (drag-and-drop removal in [#8459](https://github.com/deskflow/deskflow/issues/8459)).

## Platforms

| Platform | Status |
| --- | --- |
| macOS 14+, Apple Silicon | Main platform, arm64 DMG |
| macOS 26+ | Native Liquid Glass for navigation and actions; standard materials on older systems |
| Intel Mac | x86_64 DMG is released; behavior and device acceptance are tracked separately from arm64 |
| Windows x64 | Beta; Authenticode-signed installer; installation, SCM, NLA, RemoteApp and device acceptance are tracked separately |
| Linux | Not a first-release target |

## Download and install

Downloads: [GitHub Releases](https://github.com/GodD6366/DeskFuse/releases) or the [website](https://deskfuse.godd.cc). v0.7.0 and earlier installers stay in the retired [release repository](https://github.com/GodD6366/deskfuse-release/releases).

### Homebrew (macOS 14+, Apple Silicon)

```sh
brew trust --cask godd6366/deskfuse/deskfuse
brew tap godd6366/deskfuse https://github.com/GodD6366/DeskFuse
brew install --cask godd6366/deskfuse/deskfuse
```

Run `brew trust` once (skip it on older Homebrew without the command). If you tapped the old release repository, run `brew untap godd6366/deskfuse` first and tap again as above. The cask pins the matching DMG SHA-256. Uninstall with `brew uninstall --cask godd6366/deskfuse/deskfuse`, then `brew untap godd6366/deskfuse` if unused.

### Updates

Public GitHub packages check GitHub Releases (including prereleases) in Settings → Updates. An installed Developer ID build downloads `DeskFuse-arm64-update.zip` and replaces `/Applications/DeskFuse.app` in place, without the install window. Builds without that stable signature download the same app package and follow the install window. Homebrew installations can also run:

```sh
brew update && brew upgrade --cask deskfuse
```

Internal signed packages continue using trusted LAN automatic updates. A public package replaces the installed app the same way, but trust is Developer ID plus SHA-256 rather than the LAN manifest. Older builds that cannot install the app zip need one update through Homebrew or a manually downloaded DMG; those builds only recognize the DMG, so releases keep publishing it.

### Download the app package

1. Download `DeskFuse-arm64-update.zip` or `DeskFuse-x86_64-update.zip` for your Mac, plus `SHA256SUMS`, `release.json`, and `build-info.json` from the same release, then run `shasum -a 256 -c SHA256SUMS`.
2. Open `DeskFuse.app` inside the zip. The install window copies it into Applications; run only one instance per device. A copy that is already in Applications opens directly and does not show the install window.
3. Allow local network access when prompted; input sharing also needs Accessibility permission, a trusted LAN session, enabled devices and a valid display layout.

Homebrew and already shipped clients still use `DeskFuse-arm64.dmg` / `DeskFuse-x86_64.dmg` from the same release. Those older clients only recognize the disk image, so releases keep publishing it.

GitHub prereleases built with Apple credentials are Developer ID signed and notarized, and should open normally. Never disable Gatekeeper or SIP. Older ad-hoc builds may still be blocked: use the system's “Open Anyway” option, or `sudo xattr -rd com.apple.quarantine "/Applications/DeskFuse.app"`. Maintainer credentials: [notarization secrets](docs/NOTARIZE.md).

## Build from source

Requires an Apple Silicon Mac, full **Xcode 27+ (macOS 27+ SDK)**, Rustup and Python 3.11+; Command Line Tools alone are not enough. The deployment target is macOS 14, with Rust and dependencies pinned in `rust-toolchain.toml` and `Cargo.lock`.

```sh
git clone git@github.com:GodD6366/DeskFuse.git
cd DeskFuse
rustup show active-toolchain
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh
```

Without a signing certificate, add `CODESIGN_IDENTITY=-` explicitly; add `--build` to build without launching, and select a specific Xcode per command with `DEVELOPER_DIR=...`. For a local LAN installer, run `./scripts/package-macos.sh` (equivalent to `--lan`); output goes to `dist/{version}-build{number}/`. Homebrew/GitHub packages require explicit `--homebrew`. For an ad-hoc channel test, use `CODESIGN_IDENTITY=- ./scripts/package-macos.sh --homebrew`, which writes to `dist/homebrew/` and takes no part in LAN updates.

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

- The Windows installer is an experimental offline desktop package; installation, SCM, session switching and cross-platform features need separate acceptance.
- Native Finder paste, drag-out, permission changes, Wi-Fi input crossing and real two-device behavior cannot be replaced by CI.
- GitHub CI does not sign LAN update payloads; the coordinator build supports automatic macOS LAN updates, and older versions need one manual base install, see [RELEASING.md](docs/RELEASING.md).
- Logs contain metadata, state and hashes only; check attachments before filing issues and never upload clipboard contents, device keys or credentials.

Report vulnerabilities according to [SECURITY.md](SECURITY.md).

## Acknowledgments

- [OpenLogi](https://github.com/AprilNEA/OpenLogi) (MIT/Apache-2.0, © AprilNEA): DeskFuse's Logitech HID++ mouse support references its feature addressing, device compatibility list, and battery/SmartShift protocol notes; DeskFuse uses its own implementation, see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- [BlueSync](https://github.com/deskflow/deskflow): interaction behavior reference.

## License

Current source is maintained privately. Previously published versions retain their original [MIT](LICENSE-MIT) / [Apache-2.0](LICENSE-APACHE) licenses; new original code is covered by [LICENSE](LICENSE). Dependencies and third-party assets retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
