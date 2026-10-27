//! Synchronous authentication only: no await is permitted while impersonating.
#![allow(unsafe_code)]
use crate::{
    policy::{ClientSession, SessionPolicy},
    protocol::SERVICE_NAME,
};
use std::{
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
};
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        CheckTokenMembership, CreateWellKnownSid, GetLengthSid, GetTokenInformation,
        IsWellKnownSid, RevertToSelf, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_STATISTICS,
        TOKEN_USER, TokenSessionId, TokenStatistics, TokenUser, WinInteractiveSid,
        WinLocalSystemSid,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FILE_WRITE_EA, OPEN_EXISTING, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT,
    },
    System::{
        Environment::ExpandEnvironmentStringsW,
        Pipes::{
            GetNamedPipeClientSessionId, GetNamedPipeServerProcessId, ImpersonateNamedPipeClient,
        },
        Registry::{
            HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey, RegOpenKeyExW,
            RegQueryValueExW,
        },
        RemoteDesktop::{
            WTS_CURRENT_SERVER_HANDLE, WTS_SESSIONSTATE_UNLOCK, WTSActive, WTSClientProtocolType,
            WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSINFOEXW, WTSQuerySessionInformationW,
            WTSSessionInfoEx,
        },
        Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_HANDLE,
            SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
            SERVICE_STATUS_PROCESS,
        },
        Threading::{GetCurrentThread, OpenThreadToken},
    },
};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "service session access denied",
    )
}

struct Descriptor(*mut std::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated this descriptor.
        unsafe {
            LocalFree(self.0);
        }
    }
}
/// Total instances allowed for one pipe name, listening included.
///
/// Windows rejects *new client connections* with `ERROR_PIPE_BUSY` once this
/// many instances exist, even when one is listening. Long-lived connections
/// therefore make this a hard ceiling on concurrent clients, not a hint: the
/// previous value of 10 was below the service's own client limit.
pub const MAX_PIPE_INSTANCES: usize = 64;

/// IU has data read/write rights but not `FILE_CREATE_PIPE_INSTANCE` (0x4).
/// No Everyone/Authenticated Users/Network ACL; remote clients are rejected too.
pub fn create_pipe(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    create_pipe_with_sddl(name, first, "O:SYG:SYD:P(A;;GA;;;SY)(A;;0x0012019b;;;IU)")
}
/// Resolve the authenticated user's canonical library directory from the
/// machine profile list. The client cannot supply an arbitrary service-side path.
pub fn user_library_dir(session: &ClientSession) -> io::Result<std::path::PathBuf> {
    let mut sid_text = ptr::null_mut::<u16>();
    // SAFETY: user_sid is the kernel TOKEN_USER SID captured during authentication.
    if unsafe { ConvertSidToStringSidW(session.user_sid.as_ptr().cast_mut().cast(), &mut sid_text) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let sid = unsafe {
        std::ffi::OsString::from_wide(&std::slice::from_raw_parts(
            sid_text,
            (0..).take_while(|i| *sid_text.add(*i) != 0).count(),
        ))
    }
    .to_string_lossy()
    .into_owned();
    unsafe { LocalFree(sid_text.cast()) };
    let path = wide(&format!(
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{sid}"
    ));
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: path is NUL terminated and key is a valid out parameter.
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut key) } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "user profile unavailable",
        ));
    }
    let value_name = wide("ProfileImagePath");
    // ProfileImagePath is REG_EXPAND_SZ on most machines (%SystemDrive% style);
    // a plain REG_SZ also appears on some installs.
    let mut kind = 0_u32;
    let mut bytes = 0_u32;
    // SAFETY: querying the size with a null buffer does not write memory.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != 0
        || (kind != REG_SZ && kind != REG_EXPAND_SZ)
        || bytes == 0
        || bytes > 32_768
        || bytes % 2 != 0
    {
        unsafe { RegCloseKey(key) };
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "user profile path unavailable",
        ));
    }
    let mut buffer = vec![0_u16; bytes as usize / 2];
    // SAFETY: buffer matches the size returned by the registry and key remains open.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null(),
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    unsafe { RegCloseKey(key) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let profile = profile_path(&buffer, kind)?;
    Ok(profile.join("AppData").join("Local").join("DeskFuse"))
}

fn profile_path(buffer: &[u16], kind: u32) -> io::Result<std::path::PathBuf> {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "unterminated user profile path")
        })?;
    let raw = if kind == REG_EXPAND_SZ {
        // Keep the registry terminator: passing a trimmed UTF-16 slice to a
        // NUL-terminated Win32 API reads beyond the allocation into arbitrary data.
        let terminated = &buffer[..=length];
        let expanded = unsafe {
            ExpandEnvironmentStringsW(terminated.as_ptr().cast_mut(), ptr::null_mut(), 0)
        };
        if expanded == 0 || expanded > 32_768 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid expanded profile size",
            ));
        }
        let mut target = vec![0_u16; expanded as usize];
        let written = unsafe {
            ExpandEnvironmentStringsW(
                terminated.as_ptr().cast_mut(),
                target.as_mut_ptr(),
                expanded,
            )
        };
        if written == 0 || written > expanded || target[written as usize - 1] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "profile expansion failed",
            ));
        }
        target.truncate(written as usize - 1);
        target
    } else if kind == REG_SZ {
        buffer[..length].to_vec()
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid profile registry type",
        ));
    };
    let profile = std::path::PathBuf::from(std::ffi::OsString::from_wide(&raw));
    if !profile.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "user profile path is not absolute",
        ));
    }
    Ok(profile)
}

#[cfg(test)]
mod profile_path_tests {
    use super::*;

    #[test]
    fn expanded_profile_stops_at_registry_terminator() {
        let expected = r"C:\Users\DeskFuseTest";
        let mut value = wide(expected);
        value.extend("THIS_MUST_NOT_ENTER_THE_PATH".encode_utf16());
        for _ in 0..64 {
            assert_eq!(
                profile_path(&value, REG_EXPAND_SZ).unwrap(),
                std::path::PathBuf::from(expected)
            );
        }
        assert_eq!(
            profile_path(&value, REG_SZ).unwrap(),
            std::path::PathBuf::from(expected)
        );
    }

    #[test]
    fn malformed_profile_is_rejected_without_creating_directories() {
        assert!(
            profile_path(
                &r"C:\Users\Test".encode_utf16().collect::<Vec<_>>(),
                REG_EXPAND_SZ
            )
            .is_err()
        );
        assert!(profile_path(&wide("relative"), REG_SZ).is_err());
        assert!(profile_path(&wide(r"C:\Users\Test"), 0).is_err());
    }
}

fn create_pipe_with_sddl(name: &str, first: bool, sddl: &str) -> io::Result<NamedPipeServer> {
    let sddl = wide(sddl);
    let mut descriptor = ptr::null_mut();
    // SAFETY: terminated input and valid output descriptor pointer; revision 1 is SDDL_REVISION_1.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let descriptor = Descriptor(descriptor);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).map_err(|_| denied())?,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: SECURITY_ATTRIBUTES and its self-relative descriptor are alive throughout creation;
    // Windows copies the descriptor. Handles are not inherited. Tokio owns returned handle.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(MAX_PIPE_INSTANCES)
            .in_buffer_size(16_384)
            .out_buffer_size(16_384)
            .create_with_security_attributes_raw(name, (&raw mut attributes).cast())
    }
}
/// Open with concrete data rights, NOT `GENERIC_WRITE` (which includes the
/// `FILE_CREATE_PIPE_INSTANCE`/`FILE_APPEND_DATA` bit denied by the server DACL).
pub fn open_client(name: &str) -> io::Result<NamedPipeClient> {
    if !name.starts_with(r"\\.\pipe\") || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected local service pipe",
        ));
    }
    let name = wide(name);
    let access = FILE_GENERIC_READ | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES | FILE_WRITE_EA;
    // SAFETY: terminated local pipe name; no inheritable handle/security attrs;
    // overlapped flag is required by Tokio. Identification SQOS prevents this
    // server from acting as the caller outside token identification.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: transfer sole ownership of this overlapped pipe to Tokio. Its
    // from_raw_handle first wraps with Mio ownership, so even registration
    // failure closes the handle; no second CloseHandle on that error path.
    unsafe { NamedPipeClient::from_raw_handle(handle) }
}

struct Impersonation;
impl Drop for Impersonation {
    fn drop(&mut self) {
        // SAFETY: this guard lives only on the thread that successfully impersonated.
        // Continuing as a client would be a privilege-context error: terminate instead.
        if unsafe { RevertToSelf() } == 0 {
            std::process::abort();
        }
    }
}

pub fn authenticate(pipe: HANDLE, policy: SessionPolicy) -> io::Result<ClientSession> {
    let mut pipe_session = 0;
    // SAFETY: caller owns a connected pipe; output integer is writable.
    if unsafe { GetNamedPipeClientSessionId(pipe, &raw mut pipe_session) } == 0 {
        return Err(denied());
    }
    // SAFETY: the most recent pipe request was read before this function. No await occurs here.
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return Err(denied());
    }
    let guard = Impersonation;
    let result = token_session(pipe_session);
    drop(guard); // Explicitly revert before any WTS calls or returning to the async executor.
    let (token_session, logon_id, user_sid, interactive, local_system) = result?;
    // SAFETY: no arguments or ownership obligations.
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let (unlocked, physical_console) = local_session_facts(pipe_session)?;
    let auth = ClientSession {
        pipe_session,
        token_session,
        logon_id,
        user_sid,
        interactive,
        local_system,
        active_console_session: console,
        unlocked,
        physical_console,
    };
    if !auth.allowed(policy) {
        return Err(denied());
    }
    Ok(auth)
}
fn token_session(pipe_session: u32) -> io::Result<(u32, u64, [u8; 68], bool, bool)> {
    let mut token = ptr::null_mut();
    // SAFETY: writable output; OpenAsSelf checks access as the service, not the impersonated user.
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &raw mut token) } == 0 {
        return Err(denied());
    }
    // SAFETY: the successful call transfers an owned token handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut session = 0_u32;
    let mut needed = 0;
    // SAFETY: exact-size writable u32 buffer matches TokenSessionId.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenSessionId,
            (&raw mut session).cast(),
            4,
            &raw mut needed,
        )
    } == 0
        || needed != 4
        || session != pipe_session
    {
        return Err(denied());
    }
    let mut storage = [0_usize; 64];
    // SAFETY: suitably aligned 512-byte buffer outlives TOKEN_USER and its inline SID.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            u32::try_from(size_of_val(&storage)).map_err(|_| denied())?,
            &raw mut needed,
        )
    } == 0
        || (needed as usize) < size_of::<TOKEN_USER>()
    {
        return Err(denied());
    }
    // SAFETY: successful query writes TOKEN_USER and SID in this live aligned buffer.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    // SAFETY: user SID points inside the successful TokenUser result.
    let system = unsafe { IsWellKnownSid(user.User.Sid, WinLocalSystemSid) } != 0;
    // SAFETY: the token query returned a valid SID in live aligned storage.
    let length = unsafe { GetLengthSid(user.User.Sid) } as usize;
    if length == 0 || length > 68 {
        return Err(denied());
    }
    let mut user_sid = [0_u8; 68];
    // SAFETY: Windows guarantees a valid SID of GetLengthSid bytes from TokenUser.
    user_sid[..length]
        .copy_from_slice(unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), length) });
    let mut sid = [0_usize; 16];
    let mut sid_size = u32::try_from(size_of_val(&sid)).map_err(|_| denied())?;
    let mut member = 0;
    // SAFETY: SID buffer is aligned and sized; builtin SID requires no domain SID.
    if unsafe {
        CreateWellKnownSid(
            WinInteractiveSid,
            ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &raw mut sid_size,
        )
    } == 0
    {
        return Err(denied());
    }
    // SAFETY: token is a live impersonation token; sid and output remain valid.
    if unsafe {
        CheckTokenMembership(
            token.as_raw_handle(),
            sid.as_mut_ptr().cast(),
            &raw mut member,
        )
    } == 0
    {
        return Err(denied());
    }
    let mut stats = TOKEN_STATISTICS::default();
    // SAFETY: exact writable structure/size matches TokenStatistics; token still live.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenStatistics,
            (&raw mut stats).cast(),
            u32::try_from(size_of::<TOKEN_STATISTICS>()).map_err(|_| denied())?,
            &raw mut needed,
        )
    } == 0
    {
        return Err(denied());
    }
    let logon_id = (u64::from(stats.AuthenticationId.HighPart.cast_unsigned()) << 32)
        | u64::from(stats.AuthenticationId.LowPart);
    Ok((session, logon_id, user_sid, member != 0, system))
}
struct WtsBuffer(*mut u16);
impl Drop for WtsBuffer {
    fn drop(&mut self) {
        // SAFETY: successful WTS query allocated this buffer.
        unsafe {
            WTSFreeMemory(self.0.cast());
        }
    }
}
pub(super) fn session_still_active(auth: ClientSession, policy: SessionPolicy) -> bool {
    // SAFETY: system query with no pointer or ownership arguments.
    if (unsafe { WTSGetActiveConsoleSessionId() }) != auth.pipe_session {
        return false;
    }
    local_session_facts(auth.pipe_session).is_ok_and(|(unlocked, physical_console)| {
        (unlocked || policy.allow_locked) && (physical_console || policy.allow_remote)
    })
}
/// Facts of the session's current console attachment: (unlocked, physical console).
/// A session that is not WTSActive reports both as false; it cannot satisfy any policy.
fn local_session_facts(session: u32) -> io::Result<(bool, bool)> {
    let mut data = ptr::null_mut();
    let mut size = 0;
    // SAFETY: valid output pointers; WTS_CURRENT_SERVER_HANDLE selects this machine.
    if unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            WTSSessionInfoEx,
            &raw mut data,
            &raw mut size,
        )
    } == 0
    {
        return Err(denied());
    }
    let data = WtsBuffer(data);
    if data.0.is_null() || (size as usize) < size_of::<WTSINFOEXW>() {
        return Err(denied());
    }
    // SAFETY: buffer length checked; read_unaligned avoids assumptions about WTS alignment.
    let info = unsafe { ptr::read_unaligned(data.0.cast::<WTSINFOEXW>()) };
    if info.Level != 1 {
        return Err(denied());
    }
    // SAFETY: Level 1 is the documented union discriminant.
    let info = unsafe { info.Data.WTSInfoExLevel1 };
    if info.SessionId != session || info.SessionState != WTSActive {
        return Ok((false, false));
    }
    let unlocked = info.SessionFlags == WTS_SESSIONSTATE_UNLOCK.cast_signed();
    let mut protocol = ptr::null_mut();
    // SAFETY: same query contract, this time a USHORT protocol id.
    if unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            WTSClientProtocolType,
            &raw mut protocol,
            &raw mut size,
        )
    } == 0
    {
        return Err(denied());
    }
    let protocol = WtsBuffer(protocol);
    if protocol.0.is_null() || size < 2 {
        return Err(denied());
    }
    // SAFETY: checked 2-byte buffer, only protocol 0 is the physical console.
    let physical = unsafe { ptr::read_unaligned(protocol.0) } == 0;
    Ok((unlocked, physical))
}
struct ScHandle(SC_HANDLE);
impl Drop for ScHandle {
    fn drop(&mut self) {
        // SAFETY: successful OpenSCManager/OpenService returned the owned handle.
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}
/// Client verifies the server against SCM before writing even a metadata request.
pub fn verify_server(pipe: HANDLE) -> io::Result<()> {
    let mut pid = 0;
    // SAFETY: pipe is a live connected client handle, output writable.
    if unsafe { GetNamedPipeServerProcessId(pipe, &raw mut pid) } == 0 || pid == 0 {
        return Err(denied());
    }
    // SAFETY: null machine/database selects local active SCM; read-only access.
    let scm = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT) };
    if scm.is_null() {
        return Err(denied());
    }
    let scm = ScHandle(scm);
    // SAFETY: scm valid, terminated service name, query-only access.
    let service = unsafe { OpenServiceW(scm.0, wide(SERVICE_NAME).as_ptr(), SERVICE_QUERY_STATUS) };
    if service.is_null() {
        return Err(denied());
    }
    let service = ScHandle(service);
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut bytes = 0;
    // SAFETY: writable struct matches SC_STATUS_PROCESS_INFO; length checked.
    if unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            (&raw mut status).cast(),
            u32::try_from(size_of::<SERVICE_STATUS_PROCESS>()).map_err(|_| denied())?,
            &raw mut bytes,
        )
    } == 0
    {
        return Err(denied());
    }
    if status.dwProcessId != pid || status.dwCurrentState != SERVICE_RUNNING {
        return Err(denied());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::windows::named_pipe::ClientOptions,
    };
    fn test_name() -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        format!(
            r"\\.\pipe\deskfuse-test-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
    #[tokio::test]
    async fn concrete_client_rights_work_without_granting_server_instance_rights() {
        let name = test_name();
        // Use the exact production data mask. AU replaces IU only so an SSH
        // test token can exercise the ACL without changing any logon session.
        let mut server = create_pipe_with_sddl(&name, true, "D:P(A;;0x0012019b;;;AU)").unwrap();
        let mut client = open_client(&name).unwrap();
        server.connect().await.unwrap();
        client.write_all(b"request").await.unwrap();
        let mut request = [0_u8; 7];
        server.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"request");
        server.write_all(b"response").await.unwrap();
        let mut response = [0_u8; 8];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"response");
        let duplicate = create_pipe_with_sddl(&name, false, "D:P(A;;0x0012019b;;;AU)").unwrap_err();
        assert_eq!(duplicate.raw_os_error(), Some(5));
    }
    #[tokio::test]
    async fn desktop_client_rejects_non_scm_pipe_server_before_sending_request() {
        let name = test_name();
        // Test fixture deliberately does not use or install the production service.
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new()
            .security_qos_flags(windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION)
            .open(&name)
            .unwrap();
        server.connect().await.unwrap();
        assert!(verify_server(client.as_raw_handle()).is_err());
        assert!(
            ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)
                .is_err()
        );
    }
    #[tokio::test]
    async fn authentication_always_reverts_impersonation_on_failure_or_success() {
        let name = test_name();
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .unwrap();
        let mut client = ClientOptions::new()
            .security_qos_flags(windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION)
            .open(&name)
            .unwrap();
        server.connect().await.unwrap();
        client.write_all(b"x").await.unwrap();
        assert_eq!(server.read_u8().await.unwrap(), b'x');
        let result = authenticate(server.as_raw_handle(), SessionPolicy::STRICT);
        // SSH/session 0 must fail; an interactive console test may authenticate.
        if let Ok(auth) = result {
            assert!(auth.allowed(SessionPolicy::STRICT));
        }
        let mut token = ptr::null_mut();
        // SAFETY: query current thread after authentication guard has been dropped.
        assert_eq!(
            unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &raw mut token) },
            0
        );
        // SAFETY: immediately read last error after the failed token query.
        assert_eq!(
            unsafe { windows_sys::Win32::Foundation::GetLastError() },
            windows_sys::Win32::Foundation::ERROR_NO_TOKEN
        );
    }
}
