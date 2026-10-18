# Mouse action save / execution follow-up

- Observed persisted revision 22: generic button 3 = Mission Control, generic button 4 = left Space; calibrated native 211 = right Space and 212 = left Space. This establishes conflicting parallel mappings, not proof that this was the sole failure cause.
- Added explicit G502 draft/save UI, persistence result feedback, and latest native input / shortcut-post feedback.
- Save removes the calibrated position's generic mappings to prevent parallel DeskFuse actions. Original hardware functions remain unchanged.
- Built and signed development bundle, restarted it, and verified the actual Save and Apply button and successful result in the UI.
- Saved both existing side-key actions via UI. Read-only SQLite verification: revision 24 retains native 211/212 and MX 195/196; generic 3/4 conflicts removed. Physical location calibration unchanged.
- Targeted Swift tests: 18 passed (G502ButtonLayoutTests, MouseSettingsTests, MouseShortcutTests, LogitechHIDProtocolTests). git diff --check passed.
- Requested actual G4/G5 presses from user. End-to-end Spaces switching remains unverified pending physical feedback.
