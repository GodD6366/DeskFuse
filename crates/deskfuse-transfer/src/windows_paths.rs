//! Windows filename representability, checked before any receiver writes.
//! Reject rather than rename: alternate streams, DOS devices and case aliases
//! are not valid remote filenames for the Win32 filesystem interface.
use crate::TransferError;

pub(crate) fn validate_component(component: &str) -> Result<(), TransferError> {
    let invalid = component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with(['.', ' '])
        || component.encode_utf16().count() > 255
        || component
            .chars()
            .any(|c| c < ' ' || "<>:\"/\\|?*".contains(c));
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if invalid || device {
        return Err(TransferError::InvalidPath(format!(
            "filename cannot be represented on Windows: {component}"
        )));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn validate_manifest(manifest: &crate::TransferManifest) -> Result<(), TransferError> {
    use deskfuse_platform_windows::filesystem::compare_names_ignore_case;
    use std::cmp::Ordering;
    // Include implicit parent directories: A/x and a/y are a collision even
    // when the sender didn't emit directory entries. Exact shared prefixes are OK.
    let mut paths = std::collections::BTreeSet::new();
    for entry in manifest.entries() {
        let mut prefix = String::new();
        for part in entry.path().as_str().split('/') {
            validate_component(part)?;
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            paths.insert(prefix.clone());
        }
    }
    let mut paths: Vec<_> = paths.into_iter().collect();
    let mut failed = None;
    paths.sort_by(|a, b| match compare_names_ignore_case(a, b) {
        Ok(order) => order,
        Err(error) => {
            failed = Some(error);
            Ordering::Equal
        }
    });
    if let Some(error) = failed {
        return Err(error.into());
    }
    for pair in paths.windows(2) {
        if compare_names_ignore_case(&pair[0], &pair[1])? == Ordering::Equal {
            return Err(TransferError::InvalidManifest(format!(
                "Windows case-insensitive path collision: {} / {}",
                pair[0], pair[1]
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_device_stream_and_lossy_names() {
        for name in [
            "CON",
            "nul.txt",
            "COM1.png",
            "Lpt².log",
            "CON .txt",
            "file:stream",
            "a.",
            "a ",
            "a?",
            "a\u{1}",
            "..",
            "CONOUT$",
        ] {
            assert!(validate_component(name).is_err(), "{name:?}");
        }
        assert!(validate_component(&"😀".repeat(128)).is_err());
    }
    #[test]
    fn preserves_representable_names() {
        for name in [
            "你好.txt",
            "COM10",
            "context.txt",
            ".hidden",
            "a b",
            "my😀file",
            "LPT0",
        ] {
            validate_component(name).unwrap();
        }
        validate_component(&"a".repeat(255)).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn native_comparison_rejects_implicit_directory_aliases() {
        use crate::{TransferEntry, TransferManifest, sha256_bytes};
        let make = |path| TransferEntry::file(path, 1, 0, sha256_bytes(b"x")).unwrap();
        assert!(TransferManifest::new(vec![make("Folder/a"), make("folder/b")]).is_err());
        assert!(TransferManifest::new(vec![make("A.txt"), make("a.txt")]).is_err());
        assert!(TransferManifest::new(vec![make("Folder/a"), make("Folder/b")]).is_ok());
    }
}
