//! Pure catalog planning shared with the Windows host. The controller supplies
//! only a catalog ID; executable paths are always resolved on the host.
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fmt::Write};

pub const MAX_REMOTE_APP_NAME: usize = 128;
pub const MAX_REMOTE_APP_FULL_NAME: usize = 1024;
pub const MAX_REMOTE_APP_COMMAND_LINE: usize = 4096;
pub const MAX_REMOTE_APP_ICON_PATH: usize = 32767;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAppEntry {
    pub name: String,
    pub full_name: String,
    pub path: String,
    pub command_line: String,
    pub command_line_setting: u32,
    pub icon_path: String,
    pub icon_index: u32,
    pub show_in_tswa: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub path: String,
}

/// Registry subkey names are also embedded in the `||<name>` RDP token. Keep
/// them a single, bounded path component so unregister cannot delete a nested
/// key and the generated line-oriented RDP document cannot be injected.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_REMOTE_APP_NAME
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | '/' | ':' | '"'))
}

pub fn validate_entry(entry: &RemoteAppEntry) -> bool {
    fn text(value: &str, max: usize) -> bool {
        value.len() <= max
            && !value
                .chars()
                .any(|c| c.is_control() || c == '\r' || c == '\n')
    }
    text(&entry.name, MAX_REMOTE_APP_NAME)
        && valid_name(&entry.name)
        && text(&entry.full_name, MAX_REMOTE_APP_FULL_NAME)
        && executable_path(&entry.path)
        && text(&entry.command_line, MAX_REMOTE_APP_COMMAND_LINE)
        && text(&entry.icon_path, MAX_REMOTE_APP_ICON_PATH)
}

/// Only local, absolute executable paths, with no RDP line injection. Existence
/// is checked separately by the Windows host before publishing or launching.
pub fn executable_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() > 3
        && bytes.len() <= 32767
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
        && !path.chars().any(|c| c.is_control() || c == '"')
        && path.to_ascii_lowercase().ends_with(".exe")
}

fn path_key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// Existing manual registrations win. Discovered applications receive stable,
/// path-derived IDs so duplicate display names cannot launch the wrong program.
pub fn merge_catalog(
    registered: Vec<RemoteAppEntry>,
    installed: Vec<InstalledApp>,
) -> Vec<RemoteAppEntry> {
    let mut entries: Vec<_> = registered.into_iter().filter(validate_entry).collect();
    let mut paths: HashSet<_> = entries.iter().map(|e| path_key(&e.path)).collect();
    let mut names: HashSet<_> = entries.iter().map(|e| e.name.to_lowercase()).collect();
    for app in installed {
        if !executable_path(&app.path) || app.name.chars().any(char::is_control) {
            continue;
        }
        let key = path_key(&app.path);
        if !paths.insert(key.clone()) {
            continue;
        }
        let mut name = String::from("deskfuse-");
        for byte in Sha256::digest(key.as_bytes()) {
            let _ = write!(name, "{byte:02x}");
        }
        if !names.insert(name.clone()) {
            continue;
        }
        entries.push(RemoteAppEntry {
            name,
            full_name: app.name,
            icon_path: app.path.clone(),
            path: app.path,
            command_line: String::new(),
            command_line_setting: 0,
            icon_index: 0,
            show_in_tswa: 0,
        });
    }
    entries.sort_by_key(|e| {
        (
            e.full_name.to_lowercase(),
            e.name.to_lowercase(),
            path_key(&e.path),
        )
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    fn installed(name: &str, path: &str) -> InstalledApp {
        InstalledApp {
            name: name.into(),
            path: path.into(),
        }
    }
    #[test]
    fn rejects_nonlocal_and_injected_paths() {
        for path in [
            "relative.exe",
            r"\\server\share\a.exe",
            "C:\\a.exe\nusername:s:other",
            "C:\\a.exe\"",
            "C:\\a.cmd",
        ] {
            assert!(!executable_path(path), "{path}");
        }
        assert!(executable_path(r"C:\Program Files\App\app.exe"));
    }
    #[test]
    fn deduplicates_paths_but_not_display_names() {
        let entries = merge_catalog(
            vec![],
            vec![
                installed("App", r"C:\one.exe"),
                installed("App", "c:/ONE.exe"),
                installed("App", r"C:\two.exe"),
            ],
        );
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].name, entries[1].name);
        let again = merge_catalog(vec![], vec![installed("Renamed", r"C:\one.exe")]);
        assert!(entries.iter().any(|e| e.name == again[0].name));
    }
    #[test]
    fn preserves_manual_registration_and_launch_arguments() {
        let mut manual = merge_catalog(vec![], vec![installed("App", r"C:\app.exe")]).remove(0);
        manual.name = "manual".into();
        manual.command_line = "--existing".into();
        assert_eq!(
            merge_catalog(vec![manual.clone()], vec![installed("App", "c:/APP.exe")]),
            vec![manual]
        );
    }

    #[test]
    fn rejects_injected_or_oversized_entry_fields() {
        let base = RemoteAppEntry {
            name: "app".into(),
            full_name: "App".into(),
            path: r"C:\app.exe".into(),
            command_line: "".into(),
            command_line_setting: 0,
            icon_path: "".into(),
            icon_index: 0,
            show_in_tswa: 0,
        };
        assert!(validate_entry(&base));
        for (field, value) in [
            ("name", "app\r\nfoo"),
            ("full", "x\ny"),
            ("command", "x\r\ny"),
            ("icon", "x\n"),
        ] {
            let mut candidate = base.clone();
            match field {
                "name" => candidate.name = value.into(),
                "full" => candidate.full_name = value.into(),
                "command" => candidate.command_line = value.into(),
                _ => candidate.icon_path = value.into(),
            }
            assert!(!validate_entry(&candidate), "{field}");
        }
        let mut oversized = base;
        oversized.command_line = "x".repeat(MAX_REMOTE_APP_COMMAND_LINE + 1);
        assert!(!validate_entry(&oversized));
    }

    #[test]
    fn rejects_invalid_registry_names() {
        for name in ["", r"foo\bar", "foo/bar", "foo:bar", "foo\r\nbar"] {
            assert!(!valid_name(name), "{name:?}");
        }
        assert!(valid_name("notepad"));
    }

    #[test]
    fn catalog_order_is_stable_for_case_collisions() {
        let entries = merge_catalog(
            vec![],
            vec![installed("z", r"C:\z.exe"), installed("A", r"C:\a.exe")],
        );
        let again = merge_catalog(
            vec![],
            vec![installed("A", r"C:\a.exe"), installed("z", r"C:\z.exe")],
        );
        assert_eq!(entries, again);
    }
}
