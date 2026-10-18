Native capture and injection adapters for this process belong here. They implement
`deskfuse_platform::ports::InputExecutionPort` and must not pull content-library,
workflow, lan-web or update crates. The production helper is still
`deskfuse-engine`'s `deskfuse-input-service` until those adapters are wired.
