# G502 photo editor verification

- Replaced the default 11-row G502 editor with bundled official top/side photographs, physical hotspots and a single selection inspector.
- Retained generic/MX editors and existing action bindings. Local physical associations are separate from synced configuration; no assumed Spy-bit-to-G-number mapping.
- Native UI inspected: both photos load from the signed development bundle; side-view thumb selection updates the inspector; listening state is visible.
- UI inspection exposed accidental capture of a UI click during cancellation. Added an explicit confirmation step before saving any physical association. Removed the test-only association; existing action mappings were not changed.
- `swift test --package-path apps/deskfuse-macos --filter 'G502ButtonLayoutTests|MouseSettingsTests|MouseShortcutTests|LogitechHIDProtocolTests|HostingEnvironmentTests'`: 20 passed.
- `python3 scripts/release.py validate`, `cargo check --locked`, `git diff --check`, and shell syntax checks passed.
- Signed development bundle built using `scripts/build_and_run.sh --build` with Xcode-beta. No release or installed-bundle replacement.
- Physical calibration of all 11 positions remains user/device acceptance, not claimed by the UI checks.
