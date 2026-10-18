# Mouse pin popover UI verification

- Removed the fixed inspector column. Each photo hotspot anchors its own native popover with calibration, action choice and explicit Save and Apply.
- Preserved the existing backend and persisted bindings; no configuration migration in this change.
- Signed development build succeeded with Xcode-beta via scripts/build_and_run.sh --build; launched dist/DeskFuse-dev.app.
- Native UI verified: side-view G4 opens an adjacent popover; action menu lists supported actions; selecting Copy shows an unsaved draft; closing and reopening discards it and reloads the existing configuration. Read-only database check confirms G4 remains right-Space, not Copy.
- git diff --check passed. UI-only change validated interactively; no new unit tests or hardware acceptance claim.
