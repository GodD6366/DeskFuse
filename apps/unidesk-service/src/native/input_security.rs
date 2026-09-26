//! Kernel-observed helper identity. No PID, token flags or executable path from IPC is trusted.
use super::security;
use crate::policy::ClientSession;
use std::{
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{FILETIME, GetLastError, WAIT_TIMEOUT},
    Security::{
        GetLengthSid, GetTokenInformation, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
        TokenElevation, TokenSessionId, TokenStatistics, TokenUIAccess, TokenUser,
    },
    System::{
        Pipes::GetNamedPipeClientProcessId,
        Threading::{
            GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
};

pub(super) struct HelperIdentity {
    process: OwnedHandle,
    pid: u32,
    created: u64,
    pub ui_access: bool,
    pub elevated: bool,
}
impl HelperIdentity {
    pub fn alive(&self) -> bool {
        // SAFETY: owned process handle, nonblocking query.
        unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) == WAIT_TIMEOUT }
    }
    pub fn same_process(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.created == other.created
            && self.alive()
            && other.alive()
            && self.ui_access == other.ui_access
            && self.elevated == other.elevated
    }
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "fixed input helper identity unavailable",
    )
}
fn token_value<T: Default>(
    token: &OwnedHandle,
    class: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> io::Result<T> {
    let mut value = T::default();
    let mut needed = 0;
    // SAFETY: callers supply the fixed-size structure for the information class.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            (&raw mut value).cast(),
            size_of::<T>() as u32,
            &raw mut needed,
        )
    } == 0
        || needed as usize != size_of::<T>()
    {
        return Err(denied());
    }
    Ok(value)
}

pub(super) fn authenticate(
    pipe: windows_sys::Win32::Foundation::HANDLE,
    auth: ClientSession,
) -> io::Result<HelperIdentity> {
    let mut pid = 0;
    // SAFETY: connected server pipe and writable PID; identity is supplied by the kernel.
    if unsafe { GetNamedPipeClientProcessId(pipe, &raw mut pid) } == 0 || pid == 0 {
        return Err(denied());
    }
    // SAFETY: query/synchronize only; never inherit the handle.
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if process.is_null() {
        return Err(denied());
    }
    // SAFETY: successful OpenProcess transfers ownership.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let mut image = [0_u16; 32768];
    let mut length = image.len() as u32;
    // SAFETY: owned process and correctly sized writable UTF-16 buffer.
    if unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            0,
            image.as_mut_ptr(),
            &raw mut length,
        )
    } == 0
    {
        return Err(denied());
    }
    let expected = std::env::current_exe()?.with_file_name("unidesk-input-helper.exe");
    // The service and helper are installed together in the protected installation directory.
    // UIAccess below additionally requires Windows to have accepted the image's manifest/signature.
    if !String::from_utf16_lossy(&image[..length as usize])
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err(denied());
    }
    let mut token = ptr::null_mut();
    // SAFETY: query-only access to the pinned process token.
    if unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(denied());
    }
    // SAFETY: successful OpenProcessToken transfers ownership.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let session: u32 = token_value(&token, TokenSessionId)?;
    let stats: TOKEN_STATISTICS = token_value(&token, TokenStatistics)?;
    let logon = (u64::from(stats.AuthenticationId.HighPart.cast_unsigned()) << 32)
        | u64::from(stats.AuthenticationId.LowPart);
    let mut user = [0_usize; 64];
    let mut needed = 0;
    // SAFETY: aligned storage fits TOKEN_USER and a maximum-sized SID.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            user.as_mut_ptr().cast(),
            size_of_val(&user) as u32,
            &raw mut needed,
        )
    } == 0
    {
        return Err(denied());
    }
    // SAFETY: successful TokenUser query points to a valid SID within live storage.
    let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let length = unsafe { GetLengthSid(sid) } as usize;
    if length == 0 || length > auth.user_sid.len() {
        return Err(denied());
    }
    // SAFETY: SID length from Windows, bounded above.
    let sid = unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), length) };
    if session != auth.token_session || logon != auth.logon_id || sid != &auth.user_sid[..length] {
        return Err(denied());
    }
    let ui_access = token_value::<u32>(&token, TokenUIAccess)? != 0;
    let elevated = token_value::<u32>(&token, TokenElevation)? != 0;
    // Do not silently downgrade a failed UIAccess installation to ordinary SendInput.
    if !ui_access {
        return Err(denied());
    }
    let mut created = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: pinned process and four writable FILETIME outputs.
    if unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &raw mut created,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32
        ));
    }
    let identity = HelperIdentity {
        process,
        pid,
        created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        ui_access,
        elevated,
    };
    if !identity.alive() {
        return Err(denied());
    }
    // Recheck the pipe session/token after opening the process, closing PID reuse and impersonation races.
    if security::authenticate(pipe)? != auth {
        return Err(denied());
    }
    Ok(identity)
}
