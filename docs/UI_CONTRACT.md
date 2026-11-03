# Cross-platform UI contract

The native host remains the owner of engine state, permissions, clipboard
content and window lifecycle. macOS and Windows shells may render these
surfaces natively or host the shared MyGo pages; they must use the same bridge
contract and must not create a second business-state store.

## Read surfaces

| Request | Purpose | Memory rule |
| --- | --- | --- |
| `hello` | Negotiate the UI client and required capabilities | Do not include history or media bodies. |
| `app.snapshot` | Rebuild the current device/input/transfer state | Pass `include_history: false`; history is paged separately. |
| `library.query` | Read one clipboard-history page | Keep page windows bounded; never request image bodies for a list. |
| `clipboard.thumbnail` | Fetch a display-sized image | Decode to the requested maximum pixel size. |
| `clipboard.content` | Fetch a selected item | Request image/RTF only for an explicit preview or paste action. |

## Commands

UI actions are sent through `app.invoke` as `{ "action": string,
"params": object }`. The native host returns `{ "outcome": string,
"value": object }` for actions with a value. Clipboard actions include
`clipboard.copy`, `clipboard.paste`, `clipboard.save`, `clipboard.delete`,
`clipboard.showPanel` and `clipboard.showInMain`; the exact action parameters
remain the existing bridge schema and are shared by both platform shells.

`clipboard.showPanel` and `clipboard.togglePanel` are window commands. A native
shell should present its own lightweight popup and must not start a WebView just
to show the clipboard picker. The command still uses the host's captured paste
target and native paste coordinator.

## Platform responsibilities

- Keep Rust/Go as the single source of truth for data and commands.
- Implement status-item/tray, popup windows and permission prompts natively.
- Host at most one WebView for pages that genuinely need the shared web UI.
- Destroy or suspend the WebView process after the last complex page closes.
- Drop page snapshots, image blobs and decoded thumbnails when a popup closes.
- Treat `app.snapshot` as a rebuild input; do not retain a second long-lived copy.

This contract applies to both macOS and Windows. A platform-specific window
implementation is therefore an interchangeable presentation layer rather than
a second clipboard or synchronization implementation.
