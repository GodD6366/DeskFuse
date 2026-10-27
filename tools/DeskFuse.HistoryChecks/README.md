# Windows history checks

Run from the repository root:

```text
dotnet run --project tools/DeskFuse.HistoryChecks/DeskFuse.HistoryChecks.csproj
```

This dependency-free .NET 8 executable links the production history and content types. It uses isolated temporary directories and an injected clock; it never starts the service, accesses the system clipboard, or opens the user's history. A failed assertion exits nonzero.

Coverage includes legacy v1 JSON compatibility, native representation preservation, title persistence and duplicate capture, Unicode limits, retention at load and during maintenance, source-file protection, unlimited count/aggregate history retention, atomic device-history replacement that cannot be overwritten by queued saves, and preservation of unreadable history. It also links the production canonical query builder and models to check 23/25-hour calendar days, yesterday/last-N-days/range boundaries, malformed rules, and shared item/count date windows.

This is not a WinUI interaction or two-device acceptance test. RichEdit formatting, clipboard writes, settings dialogs, and cross-device synchronization require separate validation.
