# uniDesk

[中文](README.md) · [Contributing](CONTRIBUTING.md) · [Releasing](docs/RELEASING.md) · [TODO](TODO.md)

A native **Rust 2024 + SwiftUI** desktop app for trusted-device clipboard sync, file transfer, a compact drag-out file shelf, and separately enabled keyboard/mouse sharing. macOS first.

Real interface screenshots will be added after the first public build. This README intentionally contains no design mockups or illustrations; the interface follows the running app.

## Status and downloads

- **macOS 14+, Apple Silicon:** experimental arm64 DMG from [GitHub Releases](https://github.com/GodD6366/uniDesk/releases).
- **Windows x64:** experimental native adapters and service checkpoint, **not** a complete GUI or installer.
- Intel Mac and Linux are not first-release targets.

Download the DMG, `SHA256SUMS`, `release.json` and `build-info.json` from the same release. Run `shasum -a 256 -c SHA256SUMS`, open the DMG and drag uniDesk into Applications. Run only one instance.

### Homebrew (macOS 14+, Apple Silicon)

The public repository also serves as a Homebrew tap whose cask points at the same GitHub release:

```sh
brew trust --cask godd6366/unidesk/unidesk
brew tap godd6366/unidesk https://github.com/GodD6366/uniDesk
brew install --cask godd6366/unidesk/unidesk
```

Homebrew 7 and newer require trusting a third-party tap once (skip that line on older versions without `brew trust`). The cask pins the SHA-256 of the matching DMG and is refreshed by the release workflow after a prerelease becomes public.

The first-release build is ad-hoc signed and not notarized, and a Homebrew install stays quarantined. Clear the attribute before the first launch; never disable Gatekeeper or SIP:

```sh
sudo xattr -rd com.apple.quarantine "/Applications/uniDesk.app"
```

Uninstall with `brew uninstall --cask godd6366/unidesk/unidesk`.

The first-release DMG is **ad-hoc signed and not notarized**. macOS may block it. Verify its origin and checksum first; use the system's Privacy & Security “Open Anyway” option only if offered and you choose to trust it. Do not disable Gatekeeper or SIP. Developer ID distribution is not configured.

Allow local network access when prompted. Input sharing also requires Accessibility permission, a trusted physical LAN session, enabled devices and valid display layout. Device identity is stored in `~/Library/Application Support/uniDesk/identity.key` with mode `0600`, without Keychain access. Switching from a Keychain build creates a new local identity and requires re-pairing; old Keychain entries are untouched. Never delete the identity file to repair pairing or work around permissions.

## Build

Requires an Apple Silicon Mac, full **Xcode 27+ with the macOS 27+ SDK**, Rustup and Python 3.11+. Command Line Tools alone are insufficient for the current SwiftUI / App Intents sources. The deployment target remains macOS 14; the build SDK is a separate requirement.

```sh
git clone https://github.com/GodD6366/uniDesk.git
cd uniDesk
rustup show active-toolchain
rustup target add aarch64-apple-darwin
rustup component add rust-src
CODESIGN_IDENTITY=- ./scripts/build_and_run.sh
```

`./scripts/build_and_run.sh --build` only builds the development bundle. The native client does not expose the legacy fixture preview. The launcher replaces the running instance only after successfully building and signing its replacement. Rust is pinned in `rust-toolchain.toml`; always preserve the root lockfile.

## Boundaries

CI verifies source and packaging, not physical two-device interaction, native Finder paste/drag-out, input permissions or Windows service installation. No LAN auto-update payload is published by this pipeline. See the Chinese [architecture](docs/ARCHITECTURE.md), [file contract](docs/FILE_TRANSFER.md), [release guide](docs/RELEASING.md) and [roadmap](docs/ROADMAP.md).

Report vulnerabilities according to [SECURITY.md](SECURITY.md). Keep clipboard contents, private keys and credentials out of logs and issue attachments.

Contributors without an Apple signing certificate can explicitly use `CODESIGN_IDENTITY=-` for an ad-hoc build. Omit it to preserve an existing local signing configuration. Select Xcode per command with `DEVELOPER_DIR` when multiple versions are installed.

For a standalone development DMG, run `CODESIGN_IDENTITY=- ./scripts/package-macos.sh --bootstrap`. Output is isolated in `dist/bootstrap/`; this mode does not read the maintainer's release key, allocate a coordinator build number, upload to NAS, or seed LAN updates. See [open-source submission preparation](docs/OPEN_SOURCE.md) and [release documentation](docs/RELEASING.md).

## License

Original code is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Dependencies and third-party assets retain their own licenses; see [third-party notices](THIRD_PARTY_NOTICES.md).
