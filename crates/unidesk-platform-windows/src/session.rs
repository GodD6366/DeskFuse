//! Read-only session/desktop capability probes. A normal GUI process must not
//! infer access to the login, lock, elevated, or UAC desktop from an unlocked
//! user desktop. No desktop switching, token duplication or policy changes occur.
#![allow(unsafe_code)]

use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE},
    System::{
        RemoteDesktop::{
            ProcessIdToSessionId, WTS_CURRENT_SERVER_HANDLE, WTS_INFO_CLASS, WTS_SESSIONSTATE_LOCK,
            WTS_SESSIONSTATE_UNLOCK, WTSActive, WTSClientProtocolType, WTSFreeMemory, WTSINFOEXW,
            WTSQuerySessionInformationW, WTSSessionInfoEx,
        },
        StationsAndDesktops::{
            CloseDesktop, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW, HDESK,
            OpenInputDesktop, UOI_NAME,
        },
        Threading::{GetCurrentProcessId, GetCurrentThreadId},
    },
};

use unidesk_platform::{DesktopState, SessionBlocker, SessionCapabilities};

struct SessionBuffer(*mut u16);
impl Drop for SessionBuffer {
    fn drop(&mut self) {
        // SAFETY: WTSQuerySessionInformationW returned this owned allocation.
        unsafe {
            WTSFreeMemory(self.0.cast());
        }
    }
}
fn session_buffer(session: u32, class: WTS_INFO_CLASS) -> Result<(SessionBuffer, u32), u32> {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0;
    // SAFETY: output pointers are valid; null server selects the current host.
    if unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            class,
            &raw mut ptr,
            &raw mut len,
        )
    } == 0
    {
        // SAFETY: GetLastError has no preconditions and is read immediately after failure.
        return Err(unsafe { GetLastError() });
    }
    if ptr.is_null() {
        return Err(13); // ERROR_INVALID_DATA
    }
    Ok((SessionBuffer(ptr), len))
}

struct InputDesktop(HDESK);
impl Drop for InputDesktop {
    fn drop(&mut self) {
        // SAFETY: this is the owned handle from OpenInputDesktop, not a borrowed thread desktop.
        unsafe {
            CloseDesktop(self.0);
        }
    }
}
fn desktop_name(handle: HANDLE) -> Result<String, u32> {
    if handle.is_null() {
        return Err(6); // ERROR_INVALID_HANDLE
    }
    let mut buffer = [0_u16; 256];
    let mut needed = 0;
    // SAFETY: handle was acquired on this thread, buffer is aligned and capacity is given in bytes.
    if unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            buffer.as_mut_ptr().cast(),
            u32::try_from(size_of_val(&buffer)).map_err(|_| 13_u32)?,
            &raw mut needed,
        )
    } == 0
    {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    let end = buffer.iter().position(|c| *c == 0).ok_or(13_u32)?;
    Ok(String::from_utf16_lossy(&buffer[..end]))
}

/// Snapshot the calling process's actual WTS session and current input desktop.
/// Re-read on session/desktop changes; it is not a durable authorization decision.
#[must_use]
pub fn session_capabilities() -> SessionCapabilities {
    let mut session = 0;
    // SAFETY: process ID is valid and output points to a live u32.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &raw mut session) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        return SessionCapabilities::unavailable(
            None,
            SessionBlocker::SessionQueryFailed,
            Some(unsafe { GetLastError() }),
        );
    }
    let mut caps =
        SessionCapabilities::unavailable(Some(session), SessionBlocker::SessionQueryFailed, None);
    let (buffer, len) = match session_buffer(session, WTSSessionInfoEx) {
        Ok(value) => value,
        Err(code) => {
            caps.native_error = Some(code);
            return caps;
        }
    };
    if (len as usize) < size_of::<WTSINFOEXW>() {
        caps.native_error = Some(13);
        return caps;
    }
    // SAFETY: WTS returns at least the checked struct size. read_unaligned does not require allocator alignment assumptions.
    let info = unsafe { std::ptr::read_unaligned(buffer.0.cast::<WTSINFOEXW>()) };
    if info.Level != 1 {
        return caps;
    }
    // SAFETY: API explicitly tags Data as WTSInfoExLevel1 when Level == 1.
    let info = unsafe { info.Data.WTSInfoExLevel1 };
    drop(buffer);
    if info.SessionState != WTSActive {
        caps.desktop = DesktopState::InactiveSession;
        caps.blockers[0] = SessionBlocker::SessionNotActive;
        return caps;
    }
    if info.SessionFlags == WTS_SESSIONSTATE_LOCK.cast_signed() {
        caps.desktop = DesktopState::Locked;
        caps.blockers[0] = SessionBlocker::SessionLocked;
        return caps;
    }
    if info.SessionFlags != WTS_SESSIONSTATE_UNLOCK.cast_signed() {
        return caps;
    }
    let (buffer, len) = match session_buffer(session, WTSClientProtocolType) {
        Ok(value) => value,
        Err(code) => {
            caps.native_error = Some(code);
            return caps;
        }
    };
    if (len as usize) < size_of::<u16>() {
        return caps;
    }
    // SAFETY: the protocol query returns a USHORT and the length was checked.
    let remote = unsafe { std::ptr::read_unaligned(buffer.0) } != 0;
    drop(buffer);
    if remote {
        caps.desktop = DesktopState::RemoteSession;
        caps.blockers[0] = SessionBlocker::RemoteSession;
        return caps;
    }
    // SAFETY: request only read access; no inheritance or desktop switching.
    let input = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if input.is_null() {
        caps.blockers[0] = SessionBlocker::InputDesktopUnavailable;
        // SAFETY: GetLastError read immediately after failure.
        caps.native_error = Some(unsafe { GetLastError() });
        return caps;
    }
    let input = InputDesktop(input);
    let input_name = match desktop_name(input.0) {
        Ok(name) => name,
        Err(code) => {
            caps.native_error = Some(code);
            return caps;
        }
    };
    if !input_name.eq_ignore_ascii_case("Default") {
        if input_name.eq_ignore_ascii_case("Winlogon") {
            caps.desktop = DesktopState::LoginOrSecureDesktop;
            caps.blockers[0] = SessionBlocker::ProtectedDesktop;
        } else {
            caps.blockers[0] = SessionBlocker::DifferentDesktop;
        }
        return caps;
    }
    // SAFETY: GetThreadDesktop returns a borrowed handle; it is deliberately not closed.
    let current = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
    let current_name = match desktop_name(current) {
        Ok(name) => name,
        Err(code) => {
            caps.native_error = Some(code);
            return caps;
        }
    };
    if !current_name.eq_ignore_ascii_case(&input_name) {
        caps.blockers[0] = SessionBlocker::DifferentDesktop;
        return caps;
    }
    caps.desktop = DesktopState::UserDesktop;
    caps.user_content_allowed = true;
    caps.blockers = vec![SessionBlocker::AuthorizedInputServiceRequired];
    caps
}
