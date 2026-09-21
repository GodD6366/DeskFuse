# DeskFuse native SDK cache

`scripts/build-sdk.sh` writes the collaboration static library, input helper and
`manifest.json` here. Binaries are not committed. Swift UI builds should reuse
this directory when the API hash in `manifest.json` is unchanged.
