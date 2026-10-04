//! Pure catalog planning shared with the Windows host. The controller supplies
//! only a catalog ID; executable paths are always resolved on the host.
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fmt::Write};

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
    let mut entries = registered;
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
    entries.sort_by_key(|e| (e.full_name.to_lowercase(), e.name.clone()));
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
}
