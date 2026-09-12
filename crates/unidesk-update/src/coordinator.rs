//! Local release numbering. The coordinator owns this ledger outside the checkout.
use crate::{ReleaseVersion, UpdateError};
use semver::Version;
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use uuid::Uuid;

/// Append-only, exclusively locked allocation. Truncated/corrupt records fail
/// closed rather than reset the counter. A failed release consumes its number.
///
/// # Errors
/// Rejects invalid versions, corrupt/overflowing ledgers, or filesystem/locking failures.
pub fn allocate_release(
    version: Version,
    ledger_path: &Path,
) -> Result<ReleaseVersion, UpdateError> {
    if !version.build.is_empty() {
        return Err(UpdateError::Invalid("version build metadata"));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(ledger_path)?;
    file.lock()?;
    if file.metadata()?.len() > 16 * 1024 * 1024 {
        return Err(UpdateError::Invalid("release ledger bounds"));
    }
    let mut ledger = String::new();
    file.read_to_string(&mut ledger)?;
    if !ledger.is_empty() && !ledger.ends_with('\n') {
        return Err(UpdateError::Invalid("incomplete release ledger"));
    }
    let mut previous = 0u64;
    for line in ledger.lines() {
        let value = line
            .parse::<u64>()
            .map_err(|_| UpdateError::Invalid("corrupt release ledger"))?;
        if value
            != previous
                .checked_add(1)
                .ok_or(UpdateError::Invalid("release number overflow"))?
        {
            return Err(UpdateError::Invalid("non-monotonic release ledger"));
        }
        previous = value;
    }
    let build_number = previous
        .checked_add(1)
        .ok_or(UpdateError::Invalid("release number overflow"))?;
    file.seek(SeekFrom::End(0))?;
    writeln!(file, "{build_number}")?;
    file.sync_all()?;
    Ok(ReleaseVersion {
        version,
        build_number,
        release_id: Uuid::new_v4(),
    })
}
