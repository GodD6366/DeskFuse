# Host Ports

Host Ports are the only way collaboration talks to the operating system. The
traits live in `unidesk-platform::ports` and have no AppKit, Win32, or GPUI
dependency. macOS and Windows hosts inject implementations; tests inject
memory doubles.

| Port | Responsibility |
| --- | --- |
| `ClipboardCapturePort` | Observe the native pasteboard, including a remote-write mark. |
| `ClipboardApplyPort` | Write remote content iff the native revision still matches. |
| `FileAuthorizationPort` | Grant readable files and a writable destination directory. |
| `InputExecutionPort` | Post one input event, or release every held key/button. |
| `CredentialStorePort` | Load/store the local identity secret. |
| `PermissionObserverPort` | Report native permission state. Granted ≠ available. |

## Apply contract

`ClipboardApplyPort::apply` must:

1. Compare `expected_revision` with the live native change count.
2. Return `Superseded` when the user already wrote something newer.
3. Mark the write as remote so the next capture is not published back.

## Input contract

Collaboration owns the lease. The host never decides whether a key should be
injected. On lease expiry the core calls `release_all`.
