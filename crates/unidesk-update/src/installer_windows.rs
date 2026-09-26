//! Windows privileged installation seam.
//!
//! The policy and signed handoff live in [`crate::windows`].  This process is
//! intentionally fail-closed until the Windows service owns admission, backup,
//! and startup self-test over an authenticated local IPC channel.  Keeping a
//! real helper binary in the package prevents callers from accidentally
//! treating an NSIS installer launched in-process as a transaction.

/// A Windows helper must never silently run an uncoordinated installer.
pub fn run_helper() -> Result<(), String> {
    Err("Windows updater helper requires service-owned admission and startup self-test".into())
}
