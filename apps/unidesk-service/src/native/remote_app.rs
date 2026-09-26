//! Windows RemoteApp hosting: `TSAppAllowList` registry management, installed-app
//! enumeration and RemoteApp availability checks.
//!
//! This mirrors the mechanism of RemoteApp Tool (kimmknight/remoteapptool, MIT)
//! without bundling its VB.NET tooling. RemoteApp is the native Windows
//! single-application RDP mode: an application registered under
//! `TSAppAllowList\Applications` can be launched seamlessly by an RDP client
//! using `RemoteApplicationMode=2` and `RemoteApplicationProgram=||<name>`.
//!
//! This module owns only the registry surface and the installed-app catalog. The
//! RDP connection itself is terminated by the peer (a uniDesk Mac running an RDP
//! client) against the standard Remote Desktop listener; this service never
//! proxies RDP bytes and never reads the user's Windows account credentials.
#![allow(unsafe_code)]

use super::security;
use serde::{Deserialize, Serialize};
use std::{io, sync::Arc};
use unidesk_transport::remote_application::RemoteSession;
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegEnumKeyExW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW,
    },
};

/// Registry allow-list root that RemoteApp Tool writes to.
pub const TS_APP_ALLOW_LIST: &str =
    r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Terminal Server\TSAppAllowList\Applications";
/// Registry hive listing per-executable `App Paths` (default value = full path).
const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
/// CurrentVersion key carrying the edition identifier used for the capability gate.
const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
/// Token replaced by the controller with the resolved peer IPv4 before connecting.
pub const RDP_HOST_TOKEN: &str = "{host}";

/// Why RemoteApp cannot be hosted on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// Home / Core editions ship neither the RDP host nor RemoteApp support.
    UnsupportedEdition {
        edition: String,
    },
}

/// One registered RemoteApp, mirroring RemoteApp Tool's registry schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAppEntry {
    /// Registry subkey name; also the RemoteApp program name used in `.rdp`.
    pub name: String,
    pub full_name: String,
    pub path: String,
    pub command_line: String,
    pub command_line_setting: u32,
    pub icon_path: String,
    pub icon_index: u32,
    pub show_in_tswa: u32,
}

/// An installed executable surfaced for the local picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub path: String,
}

/// Whether the current edition can host RemoteApps. Home editions report an
/// `EditionID` beginning with `Core`; everything else (Pro/Enterprise/Server)
/// ships the RemoteApp-capable RDP host.
#[must_use]
pub fn availability() -> Availability {
    match edition_id() {
        Some(edition) if edition.to_ascii_lowercase().starts_with("core") => {
            Availability::UnsupportedEdition { edition }
        }
        _ => Availability::Available,
    }
}

/// Enumerate currently registered RemoteApps; empty when none or unavailable.
#[must_use]
pub fn list_remote_apps() -> Vec<RemoteAppEntry> {
    let Ok(parent) = open_allow_list(false) else {
        return Vec::new();
    };
    let mut apps = Vec::new();
    for name in enum_subkeys(parent) {
        if let Some(key) = open_subkey(parent, &name, KEY_READ) {
            apps.push(RemoteAppEntry {
                name,
                full_name: read_string(key, "Name").unwrap_or_default(),
                path: read_string(key, "Path").unwrap_or_default(),
                command_line: read_string(key, "RequiredCommandLine").unwrap_or_default(),
                command_line_setting: read_dword(key, "CommandLineSetting").unwrap_or(1),
                icon_path: read_string(key, "IconPath").unwrap_or_default(),
                icon_index: read_dword(key, "IconIndex").unwrap_or(0),
                show_in_tswa: read_dword(key, "ShowInTSWA").unwrap_or(0),
            });
            // SAFETY: open_subkey returned this owned handle.
            unsafe { RegCloseKey(key) };
        }
    }
    // SAFETY: open_allow_list returned this owned handle.
    unsafe { RegCloseKey(parent) };
    apps
}

/// Register (or overwrite) a RemoteApp entry. Creates the allow-list key chain on
/// first use; this service runs as LocalSystem, so no elevation step is needed.
///
/// # Errors
/// Returns an error when the allow-list key cannot be opened for writing.
pub fn register_app(app: &RemoteAppEntry) -> io::Result<()> {
    let parent = open_allow_list(true)?;
    let key = create_subkey(parent, &app.name)?;
    let result = (|| {
        set_string(key, "Name", &app.full_name)?;
        set_string(key, "Path", &app.path)?;
        set_string(key, "RequiredCommandLine", &app.command_line)?;
        set_dword(key, "CommandLineSetting", app.command_line_setting)?;
        set_string(key, "IconPath", &app.icon_path)?;
        set_dword(key, "IconIndex", app.icon_index)?;
        set_dword(key, "ShowInTSWA", app.show_in_tswa)?;
        Ok(())
    })();
    // SAFETY: create_subkey returned this owned handle.
    unsafe { RegCloseKey(key) };
    // SAFETY: open_allow_list returned this owned handle.
    unsafe { RegCloseKey(parent) };
    result
}

/// Remove a RemoteApp entry. Missing entries are treated as success so removal
/// is idempotent.
///
/// # Errors
/// Returns an error when the allow-list key cannot be opened or the subkey
/// cannot be deleted for a reason other than absence.
pub fn unregister_app(name: &str) -> io::Result<()> {
    let parent = open_allow_list(true)?;
    let name = security::wide(name);
    // SAFETY: terminated relative subkey under the opened allow-list handle.
    let result = unsafe { RegDeleteTreeW(parent, name.as_ptr()) };
    // SAFETY: open_allow_list returned this owned handle.
    unsafe { RegCloseKey(parent) };
    if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

/// Enumerate installed executables from `App Paths` (already carries exe paths).
#[must_use]
pub fn list_installed_apps() -> Vec<InstalledApp> {
    let Some(parent) = open_key(HKEY_LOCAL_MACHINE, APP_PATHS, KEY_READ) else {
        return Vec::new();
    };
    let mut apps = Vec::new();
    for key_name in enum_subkeys(parent) {
        if let Some(key) = open_subkey(parent, &key_name, KEY_READ) {
            let path = read_string(key, "");
            // SAFETY: open_subkey returned this owned handle.
            unsafe { RegCloseKey(key) };
            if let Some(path) = path.filter(|path| !path.is_empty()) {
                let name = key_name
                    .strip_suffix(".exe")
                    .map(str::to_owned)
                    .unwrap_or(key_name);
                apps.push(InstalledApp { name, path });
            }
        }
    }
    // SAFETY: open_key returned this owned handle.
    unsafe { RegCloseKey(parent) };
    apps
}

/// Render a RemoteApp `.rdp` file. `full address` carries [`RDP_HOST_TOKEN`], to
/// be replaced by the controller with the resolved peer IPv4 (plus `:3389`).
#[must_use]
pub fn rdp_file(app: &RemoteAppEntry) -> String {
    let mut lines = Vec::new();
    lines.push(format!("full address:s:{RDP_HOST_TOKEN}:3389"));
    lines.push("prompt for credentials on client:i:1".into());
    lines.push("remoteapplicationmode:i:2".into());
    lines.push(format!("remoteapplicationprogram:s:||{}", app.name));
    lines.push(format!("remoteapplicationname:s:{}", app.full_name));
    if !app.command_line.is_empty() {
        lines.push(format!("remoteapplicationcmdline:s:{}", app.command_line));
    }
    lines.push("disableremoteappcapscheck:i:1".into());
    lines.push("alternate shell:s:rdpinit.exe".into());
    lines.push("audiomode:i:2".into());
    let mut rdp = lines.join("\r\n");
    rdp.push_str("\r\n");
    rdp
}

fn edition_id() -> Option<String> {
    let key = open_key(HKEY_LOCAL_MACHINE, CURRENT_VERSION, KEY_READ)?;
    let value = read_string(key, "EditionID");
    // SAFETY: open_key returned this owned handle.
    unsafe { RegCloseKey(key) };
    value
}

fn open_allow_list(create: bool) -> io::Result<HKEY> {
    let name = security::wide(TS_APP_ALLOW_LIST);
    let mut key = std::ptr::null_mut();
    let result = if create {
        let mut disposition = 0_u32;
        // SAFETY: terminated full path; RegCreateKeyExW creates intermediate keys.
        unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                name.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                std::ptr::null(),
                &mut key,
                &mut disposition,
            )
        }
    } else {
        // SAFETY: terminated full path under HKLM; read-only, no creation.
        unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, name.as_ptr(), 0, KEY_READ, &mut key) }
    };
    if result == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

fn open_key(hive: HKEY, path: &str, access: u32) -> Option<HKEY> {
    let path = security::wide(path);
    let mut key = std::ptr::null_mut();
    // SAFETY: terminated path; output handle is null on failure.
    let result = unsafe { RegOpenKeyExW(hive, path.as_ptr(), 0, access, &mut key) };
    (result == ERROR_SUCCESS).then_some(key)
}

fn open_subkey(parent: HKEY, name: &str, access: u32) -> Option<HKEY> {
    let name = security::wide(name);
    let mut key = std::ptr::null_mut();
    // SAFETY: terminated relative subkey under the opened parent handle.
    let result = unsafe { RegOpenKeyExW(parent, name.as_ptr(), 0, access, &mut key) };
    (result == ERROR_SUCCESS).then_some(key)
}

fn create_subkey(parent: HKEY, name: &str) -> io::Result<HKEY> {
    let name = security::wide(name);
    let mut key = std::ptr::null_mut();
    let mut disposition = 0_u32;
    // SAFETY: terminated relative subkey; intermediate keys are not required here.
    let result = unsafe {
        RegCreateKeyExW(
            parent,
            name.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            std::ptr::null(),
            &mut key,
            &mut disposition,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

fn enum_subkeys(hkey: HKEY) -> Vec<String> {
    let mut names = Vec::new();
    let mut index = 0_u32;
    loop {
        let mut buffer = [0_u16; 256];
        let mut len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: writable name buffer; length is in characters; no class/time output.
        let result = unsafe {
            RegEnumKeyExW(
                hkey,
                index,
                buffer.as_mut_ptr(),
                &mut len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if result == ERROR_NO_MORE_ITEMS {
            break;
        }
        if result != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&buffer[..len as usize]));
        index += 1;
    }
    names
}

fn read_string(hkey: HKEY, name: &str) -> Option<String> {
    let name = security::wide(name);
    let mut ty = 0_u32;
    let mut len = 0_u32;
    // SAFETY: null data buffer requests only the required byte size.
    let result = unsafe {
        RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if result != ERROR_SUCCESS && result != ERROR_MORE_DATA {
        return None;
    }
    let mut buffer = vec![0_u16; (len as usize / 2).saturating_add(1)];
    // SAFETY: buffer is at least the reported size; length is in bytes.
    let result = unsafe {
        RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buffer.as_mut_ptr().cast::<u8>(),
            &mut len,
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    // REG_SZ is UTF-16 with a trailing NUL included in `len` (bytes).
    let chars = (len as usize / 2).saturating_sub(1).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..chars]))
}

fn read_dword(hkey: HKEY, name: &str) -> Option<u32> {
    let name = security::wide(name);
    let mut ty = 0_u32;
    let mut value = 0_u32;
    let mut len = 4_u32;
    // SAFETY: exact-size DWORD buffer; length is in bytes.
    let result = unsafe {
        RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            (&mut value as *mut u32).cast::<u8>(),
            &mut len,
        )
    };
    (result == ERROR_SUCCESS).then_some(value)
}

fn set_string(hkey: HKEY, name: &str, value: &str) -> io::Result<()> {
    let name = security::wide(name);
    let value = security::wide(value);
    let bytes = u32::try_from(value.len())
        .ok()
        .and_then(|len| len.checked_mul(2))
        .ok_or_else(|| io::Error::other("registry value too long"))?;
    // SAFETY: terminated name and value; byte length covers the NUL terminator.
    let result = unsafe {
        RegSetValueExW(
            hkey,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast::<u8>(),
            bytes,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

fn set_dword(hkey: HKEY, name: &str, value: u32) -> io::Result<()> {
    let name = security::wide(name);
    // SAFETY: terminated name; pointer is a live aligned u32 with length 4.
    let result = unsafe {
        RegSetValueExW(
            hkey,
            name.as_ptr(),
            0,
            REG_DWORD,
            (&value as *const u32).cast::<u8>(),
            4,
        )
    };
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result as i32))
    }
}

// --- Cross-platform catalog protocol (length-framed JSON over the remote-app
// --- QUIC stream). The controller (a uniDesk Mac embedding an RDP client) sends
// --- [`RemoteAppRequest`] and receives [`RemoteAppMessage`]; both are externally
// --- tagged `type` JSON documents so Rust and Swift can share the same schema.

/// Catalog entry sent to a controller.
#[derive(Clone, Serialize)]
pub struct CatalogApp {
    pub name: String,
    pub display_name: String,
    pub path: String,
    pub command_line: String,
    pub icon_path: String,
    pub icon_index: u32,
}

impl From<RemoteAppEntry> for CatalogApp {
    fn from(entry: RemoteAppEntry) -> Self {
        Self {
            name: entry.name,
            display_name: entry.full_name,
            path: entry.path,
            command_line: entry.command_line,
            icon_path: entry.icon_path,
            icon_index: entry.icon_index,
        }
    }
}

/// Server → controller messages.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteAppMessage {
    Catalog {
        version: u32,
        available: bool,
        reason: Option<String>,
        apps: Vec<CatalogApp>,
    },
    Rdp {
        version: u32,
        rdp: String,
    },
    Error {
        version: u32,
        code: String,
        message: String,
    },
}

/// Controller → server messages.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteAppRequest {
    Catalog { version: u32 },
    Launch { version: u32, name: String },
}

/// Serve one authenticated remote-app stream: advertise the catalog, then answer
/// `catalog` (resend) and `launch` (return the `.rdp` file) requests until the
/// peer closes the stream. Runs on a blocking thread; `receive` waits at most
/// 250 ms per poll and never touches the Tokio worker.
pub fn serve(session: Arc<RemoteSession>) {
    let availability = availability();
    let (available, reason) = match &availability {
        Availability::Available => (true, None),
        Availability::UnsupportedEdition { edition } => (
            false,
            Some(format!("当前 Windows 版本（{edition}）不支持 RemoteApp")),
        ),
    };
    let catalog = RemoteAppMessage::Catalog {
        version: 1,
        available,
        reason,
        apps: if available {
            list_remote_apps()
                .into_iter()
                .map(CatalogApp::from)
                .collect()
        } else {
            Vec::new()
        },
    };
    if send(&session, &catalog).is_err() {
        session.close();
        return;
    }
    loop {
        match session.receive(500) {
            Ok(Some(data)) => {
                let Ok(request) = serde_json::from_slice::<RemoteAppRequest>(&data) else {
                    let _ = send(
                        &session,
                        &RemoteAppMessage::Error {
                            version: 1,
                            code: "invalid_request".into(),
                            message: "无法解析请求".into(),
                        },
                    );
                    continue;
                };
                match request {
                    RemoteAppRequest::Catalog { .. } => {
                        if send(&session, &catalog).is_err() {
                            break;
                        }
                    }
                    RemoteAppRequest::Launch { name, .. } => {
                        if !available {
                            let _ = send(
                                &session,
                                &RemoteAppMessage::Error {
                                    version: 1,
                                    code: "unavailable".into(),
                                    message: reason.clone().unwrap_or_default(),
                                },
                            );
                            continue;
                        }
                        let Some(entry) = list_remote_apps()
                            .into_iter()
                            .find(|entry| entry.name == name)
                        else {
                            let _ = send(
                                &session,
                                &RemoteAppMessage::Error {
                                    version: 1,
                                    code: "not_found".into(),
                                    message: "该应用尚未注册为 RemoteApp".into(),
                                },
                            );
                            continue;
                        };
                        if send(
                            &session,
                            &RemoteAppMessage::Rdp {
                                version: 1,
                                rdp: rdp_file(&entry),
                            },
                        )
                        .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(_) => break,
        }
        if session.is_closed() {
            break;
        }
    }
    session.close();
}

fn send(session: &RemoteSession, message: &RemoteAppMessage) -> Result<(), String> {
    let data = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    session.try_send(data)
}
