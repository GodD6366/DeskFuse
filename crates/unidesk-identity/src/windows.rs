//! Service-only machine identity. This module never falls back to an ephemeral
//! identity and never repairs an existing key or weakens an existing ACL.
//!
//! DPAPI machine scope is not an authorization boundary: the protected DACL is.
//! Only the LocalSystem service may call this provider; the GUI must use IPC.

// Win32 FFI is intentionally confined to this platform module.
#![allow(unsafe_code)]

use crate::{IdentityError, IdentityProvider, LocalIdentity};
use std::{
    ffi::{OsStr, c_void},
    fs::File,
    io::{Read, Write},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_NO_TOKEN, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_FILE_OBJECT,
        },
        Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
            CryptProtectData, CryptUnprotectData,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl, GetTokenInformation,
        INHERIT_ONLY_ACE, IsValidAcl, IsValidSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER, TokenSessionId, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFileInformationByHandle, MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_ALWAYS,
        OPEN_EXISTING, READ_CONTROL,
    },
    System::{
        Com::CoTaskMemFree,
        Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
    },
    UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath},
};

const MAGIC: &[u8; 8] = b"UDSID\0\0\x01";
const MAX_BLOB: usize = 16 * 1024;
const DIRECTORY_SDDL: &str = "O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
const FILE_SDDL: &str = "O:SYG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";

/// Stable machine identity owned exclusively by the Windows service.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsServiceIdentityProvider;

impl WindowsServiceIdentityProvider {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn load_or_create(&self) -> Result<LocalIdentity, IdentityError> {
        <Self as IdentityProvider>::load_or_create(self)
    }
}

/// Pinned service data directory with a machine-wide exclusive runtime lease.
/// The desktop process cannot request arbitrary paths or open this directory.
/// Keep this guard alive until SQLite and the network runtime have shut down.
pub struct WindowsServiceDataDirectory {
    path: PathBuf,
    _pinned: Vec<OwnedHandle>,
    _lease: File,
}
impl WindowsServiceDataDirectory {
    pub fn open() -> Result<Self, IdentityError> {
        require_service_token()?;
        let (path, pinned) = protected_directory()?;
        let sd = SecurityDescriptor::new(FILE_SDDL)?;
        let lease = open_private_file(&path.join("runtime.lock"), OPEN_ALWAYS, &sd)
            .map_err(|e| io_error("acquire exclusive runtime lease", e))?;
        // SQLite sidecars inherit only SYSTEM/Administrators access from the
        // protected parent. Validate their actual ACEs, owner and link count;
        // only these data files may have inherited (rather than protected) ACLs.
        for name in [
            "network.sqlite",
            "network.sqlite-wal",
            "network.sqlite-shm",
            "network.sqlite-journal",
        ] {
            match open_private_file_policy(&path.join(name), OPEN_EXISTING, &sd, false) {
                Ok(file) => drop(file),
                Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => {}
                Err(e) => return Err(io_error("validate service database", e)),
            }
        }
        Ok(Self {
            path,
            _pinned: pinned,
            _lease: lease,
        })
    }
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.path.join("network.sqlite")
    }
}
fn protected_directory() -> Result<(PathBuf, Vec<OwnedHandle>), IdentityError> {
    let base = program_data()?;
    // Keep every ancestor open without FILE_SHARE_DELETE until completion.
    // Reparse points cannot redirect the protected creation sequence.
    let mut pinned = pin_ancestors(&base)?;
    let directory_sd = SecurityDescriptor::new(DIRECTORY_SDDL)?;
    let root = base.join("com.unidesk.app");
    let service = root.join("service");
    for directory in [&root, &service] {
        let name = wide(directory.as_os_str())?;
        let attributes = directory_sd.attributes();
        // SAFETY: name is NUL terminated; descriptor and attributes remain live.
        if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0 {
            // SAFETY: GetLastError immediately follows the failing call.
            let code = unsafe { GetLastError() };
            if code != ERROR_ALREADY_EXISTS {
                return Err(win_error("create service directory", code));
            }
        }
        let handle = open_directory(directory)?;
        validate_acl(handle.as_raw_handle())?;
        pinned.push(handle);
    }
    Ok((service, pinned))
}

impl IdentityProvider for WindowsServiceIdentityProvider {
    fn load_or_create(&self) -> Result<LocalIdentity, IdentityError> {
        require_service_token()?;
        let (service, _pinned) = protected_directory()?;
        let file_sd = SecurityDescriptor::new(FILE_SDDL)?;
        // No sharing: cross-process exclusion, not only a Rust mutex. An existing
        // lock is reusable, but never remove/replace it (which could split locks).
        let _lock = open_private_file(&service.join("identity.lock"), OPEN_ALWAYS, &file_sd)
            .map_err(|error| io_error("acquire service identity lock", error))?;
        let destination = service.join("identity.dpapi");
        match open_private_file(&destination, OPEN_EXISTING, &file_sd) {
            Ok(file) => return decode_file(file),
            Err(error) if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) => {}
            Err(error) => return Err(io_error("open stored identity", error)),
        }
        // A leftover pending file means creation may have been interrupted. Do
        // not silently generate a replacement identity; require explicit recovery.
        let pending = service.join("identity.pending");
        let mut output = open_private_file(&pending, CREATE_NEW, &file_sd).map_err(|error| {
            io_error(
                "create pending identity (existing pending file requires recovery)",
                error,
            )
        })?;
        let identity = LocalIdentity::generate();
        let mut secret = identity.secret_bytes();
        let encrypted = protect(&mut secret);
        wipe(&mut secret);
        let encrypted = encrypted?;
        output
            .write_all(MAGIC)
            .and_then(|()| output.write_all(&encrypted))
            .and_then(|()| output.sync_all())
            .map_err(|error| io_error("persist pending identity", error))?;
        drop(output);
        let pending_name = wide(pending.as_os_str())?;
        let destination_name = wide(destination.as_os_str())?;
        // SAFETY: both paths are NUL terminated and in pinned protected dirs.
        // No REPLACE_EXISTING: a preexisting identity can never be overwritten.
        if unsafe {
            MoveFileExW(
                pending_name.as_ptr(),
                destination_name.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(last_error("commit identity"));
        }
        // Reopen and validate the committed bytes, not just the in-memory key.
        decode_file(
            open_private_file(&destination, OPEN_EXISTING, &file_sd)
                .map_err(|error| io_error("reopen committed identity", error))?,
        )
    }
}

fn failure(message: impl Into<String>) -> IdentityError {
    IdentityError::WindowsService(message.into())
}
fn win_error(operation: &str, code: u32) -> IdentityError {
    failure(format!("{operation}: Windows error {code}"))
}
fn last_error(operation: &str) -> IdentityError {
    // SAFETY: reading thread-local error does not dereference memory.
    win_error(operation, unsafe { GetLastError() })
}
fn io_error(operation: &str, error: std::io::Error) -> IdentityError {
    failure(format!("{operation}: {error}"))
}
fn wide(value: &OsStr) -> Result<Vec<u16>, IdentityError> {
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(failure("NUL in identity path"));
    }
    value.push(0);
    Ok(value)
}

fn require_service_token() -> Result<(), IdentityError> {
    let mut handle = null_mut();
    // SAFETY: pseudo thread handle is valid and handle is a writable out pointer.
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) } != 0 {
        // SAFETY: a successful OpenThreadToken transfers ownership of a real handle.
        drop(unsafe { OwnedHandle::from_raw_handle(handle) });
        return Err(failure(
            "identity access forbidden while impersonating a client",
        ));
    }
    // SAFETY: query the immediately preceding call's error.
    if unsafe { GetLastError() } != ERROR_NO_TOKEN {
        return Err(last_error("query thread identity"));
    }
    // SAFETY: valid process pseudo handle and writable handle output.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
        return Err(last_error("open service token"));
    }
    // SAFETY: handle was returned successfully and is now owned exactly once.
    let token = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut required = 0;
    // SAFETY: null buffer with zero length asks only for the required buffer size.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            null_mut(),
            0,
            &mut required,
        );
    }
    if required < size_of::<TOKEN_USER>() as u32 || required > 64 * 1024 {
        return Err(failure("invalid service token information length"));
    }
    // usize backing gives TOKEN_USER native pointer alignment.
    let mut storage = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    // SAFETY: aligned allocation has at least required bytes and remains live.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(last_error("read service token"));
    }
    // SAFETY: successful TokenUser writes a TOKEN_USER and its SID into storage.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    // SAFETY: SID pointer belongs to the still-live successful token buffer.
    if unsafe { IsWellKnownSid(user.User.Sid, WinLocalSystemSid) } == 0 {
        return Err(failure(
            "machine identity requires the LocalSystem service; desktop clients must use IPC",
        ));
    }
    // A LocalSystem desktop helper is still not the service. Keep key access in
    // Session 0 so a helper launched on an interactive or secure desktop cannot
    // accidentally gain direct persistence access merely through its token SID.
    let mut session_id: u32 = u32::MAX;
    let mut returned = 0;
    // SAFETY: TokenSessionId returns one DWORD into the correctly sized output.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenSessionId,
            std::ptr::addr_of_mut!(session_id).cast(),
            size_of::<u32>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(last_error("query service session"));
    }
    if returned != size_of::<u32>() as u32 || session_id != 0 {
        return Err(failure(
            "machine identity requires a Session 0 LocalSystem service",
        ));
    }
    Ok(())
}

fn program_data() -> Result<PathBuf, IdentityError> {
    let mut result = null_mut();
    // SAFETY: constant known-folder GUID and writable pointer output, no user token.
    let status = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, null_mut(), &mut result) };
    if status < 0 {
        return Err(failure(format!("resolve ProgramData: HRESULT {status:#x}")));
    }
    if result.is_null() {
        return Err(failure("ProgramData returned no path"));
    }
    // SAFETY: successful API returns a NUL-terminated CoTaskMem allocated string.
    let path = unsafe {
        let mut length = 0;
        while *result.add(length) != 0 {
            length += 1;
        }
        let value = String::from_utf16(std::slice::from_raw_parts(result, length));
        CoTaskMemFree(result.cast());
        value.map_err(|_| failure("invalid ProgramData UTF-16"))?
    };
    let path = PathBuf::from(path);
    match path.components().next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
                && path.is_absolute() =>
        {
            Ok(path)
        }
        _ => Err(failure("ProgramData must be on a local absolute disk path")),
    }
}

fn pin_ancestors(path: &Path) -> Result<Vec<OwnedHandle>, IdentityError> {
    let mut paths: Vec<_> = path.ancestors().collect();
    paths.reverse();
    paths.into_iter().map(open_directory).collect()
}
fn open_directory(path: &Path) -> Result<OwnedHandle, IdentityError> {
    let name = wide(path.as_os_str())?;
    // SAFETY: terminated path; no security attrs/template handle for existing dir.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("pin identity ancestor"));
    }
    // SAFETY: ownership of successful real file handle moves into RAII wrapper.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    validate_kind(handle.as_raw_handle(), true)?;
    Ok(handle)
}

fn open_private_file(
    path: &Path,
    creation: u32,
    descriptor: &SecurityDescriptor,
) -> std::io::Result<File> {
    open_private_file_policy(path, creation, descriptor, true)
}
fn open_private_file_policy(
    path: &Path,
    creation: u32,
    descriptor: &SecurityDescriptor,
    require_protected: bool,
) -> std::io::Result<File> {
    let name = wide(path.as_os_str()).map_err(std::io::Error::other)?;
    let attributes = descriptor.attributes();
    let access = if creation == OPEN_EXISTING {
        FILE_GENERIC_READ
    } else {
        FILE_GENERIC_READ | FILE_GENERIC_WRITE
    };
    // SAFETY: buffers/descriptor live through the call; no sharing prevents replacement.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access | READ_CONTROL,
            0,
            &attributes,
            creation,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: File becomes sole owner of successful CreateFileW handle.
    let file = unsafe { File::from_raw_handle(handle) };
    validate_kind(file.as_raw_handle(), false)
        .and_then(|()| validate_acl_policy(file.as_raw_handle(), require_protected))
        .map_err(std::io::Error::other)?;
    Ok(file)
}
fn validate_kind(handle: HANDLE, directory: bool) -> Result<(), IdentityError> {
    // SAFETY: BY_HANDLE_FILE_INFORMATION is a plain C integer record.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    // SAFETY: borrowed valid file handle and initialized writable output.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(last_error("inspect identity path"));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(failure(
            "identity path is a reparse point, wrong object type, or hard-linked file",
        ));
    }
    Ok(())
}

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);
impl SecurityDescriptor {
    fn new(sddl: &str) -> Result<Self, IdentityError> {
        let text = wide(OsStr::new(sddl))?;
        let mut descriptor = null_mut();
        // SAFETY: NUL-terminated SDDL, revision 1, writable allocated-descriptor output.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error("create identity security descriptor"));
        }
        Ok(Self(descriptor))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: descriptor is allocated by Convert.../GetSecurityInfo using LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn validate_acl(handle: HANDLE) -> Result<(), IdentityError> {
    validate_acl_policy(handle, true)
}
fn validate_acl_policy(handle: HANDLE, require_protected: bool) -> Result<(), IdentityError> {
    let mut owner: PSID = null_mut();
    let mut acl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: valid borrowed handle, output pointers kept alive until return.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(win_error("inspect identity ACL", status));
    }
    let descriptor = SecurityDescriptor(descriptor);
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: GetSecurityInfo allocated descriptor contains the owner and DACL.
    unsafe {
        if owner.is_null()
            || (IsWellKnownSid(owner, WinLocalSystemSid) == 0
                && IsWellKnownSid(owner, WinBuiltinAdministratorsSid) == 0)
            || acl.is_null()
            || IsValidAcl(acl) == 0
            || GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) == 0
            || (require_protected && control & SE_DACL_PROTECTED == 0)
            || (*acl).AceCount != 2
        {
            return Err(failure(
                "existing identity path has an unsafe owner or unprotected ACL",
            ));
        }
        let mut seen_system = false;
        let mut seen_admin = false;
        for index in 0..2 {
            let mut ace: *mut c_void = null_mut();
            if GetAce(acl, index, &mut ace) == 0 {
                return Err(last_error("read identity ACL entry"));
            }
            let header = &*ace.cast::<ACE_HEADER>();
            // ACCESS_ALLOWED_ACE_TYPE = 0. Reject all other layouts before casting.
            if header.AceType != 0
                || header.AceFlags & INHERIT_ONLY_ACE as u8 != 0
                || usize::from(header.AceSize) < 16
            {
                return Err(failure(
                    "existing identity ACL contains an unsupported entry",
                ));
            }
            let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            // A SID has an 8-byte header and four bytes per subauthority.
            // Bound every SID byte against this ACE before calling SID APIs;
            // IsValidAcl alone does not establish variable-size SID bounds.
            let sid_bytes = std::ptr::addr_of!(allowed.SidStart).cast::<u8>();
            let subauthorities = *sid_bytes.add(1) as usize;
            if subauthorities > 15 || 16 + 4 * subauthorities > usize::from(header.AceSize) {
                return Err(failure("identity ACL contains an invalid SID length"));
            }
            let sid = sid_bytes.cast_mut().cast();
            if IsValidSid(sid) == 0 {
                return Err(failure("identity ACL contains an invalid SID"));
            }
            let system = IsWellKnownSid(sid, WinLocalSystemSid) != 0;
            let admin = IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0;
            if (!system && !admin) || allowed.Mask & 0x1f01ff != 0x1f01ff {
                return Err(failure(
                    "identity ACL must grant full access only to SYSTEM and Administrators",
                ));
            }
            seen_system |= system;
            seen_admin |= admin;
        }
        if !seen_system || !seen_admin {
            return Err(failure("identity ACL missing required service principals"));
        }
    }
    Ok(())
}

fn decode_file(mut file: File) -> Result<LocalIdentity, IdentityError> {
    let mut encoded = Vec::new();
    (&mut file)
        .take((MAX_BLOB + MAGIC.len() + 1) as u64)
        .read_to_end(&mut encoded)
        .map_err(|error| io_error("read identity", error))?;
    let encrypted = encrypted_payload(&encoded)?;
    let mut secret = unprotect(encrypted)?;
    let identity = LocalIdentity::from_bytes(&secret);
    wipe(&mut secret);
    identity
}
fn encrypted_payload(encoded: &[u8]) -> Result<&[u8], IdentityError> {
    if encoded.len() <= MAGIC.len()
        || encoded.len() > MAGIC.len() + MAX_BLOB
        || !encoded.starts_with(MAGIC)
    {
        return Err(failure(
            "stored machine identity is corrupt or has an unsupported format; not regenerated",
        ));
    }
    Ok(&encoded[MAGIC.len()..])
}
fn protect(secret: &mut [u8]) -> Result<Vec<u8>, IdentityError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: secret.len() as u32,
        pbData: secret.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // SAFETY: valid input bytes and writable output; flags prohibit UI prompts.
    if unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    } == 0
    {
        return Err(last_error("DPAPI protect identity"));
    }
    take_dpapi_output(output)
}
fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>, IdentityError> {
    // API declares mutable bytes but does not mutate the input DATA_BLOB.
    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // SAFETY: input slice lives throughout call, all optional outputs omitted.
    if unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    } == 0
    {
        return Err(last_error(
            "DPAPI decrypt identity (stored identity will not be replaced)",
        ));
    }
    take_dpapi_output(output)
}
fn take_dpapi_output(output: CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, IdentityError> {
    if output.pbData.is_null() {
        return Err(failure("DPAPI returned no data"));
    }
    // SAFETY: successful DPAPI allocated cbData valid bytes with LocalAlloc.
    unsafe {
        let bytes = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = if bytes.is_empty() || bytes.len() > MAX_BLOB {
            Err(failure("DPAPI returned an invalid identity blob length"))
        } else {
            Ok(bytes.to_vec())
        };
        wipe(bytes);
        LocalFree(output.pbData.cast());
        result
    }
}
fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        // SAFETY: each pointer points to an exclusive live byte; volatile avoids elision.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_and_unknown_identity_formats_are_rejected() {
        for encoded in [
            vec![],
            MAGIC.to_vec(),
            vec![0; 64],
            [MAGIC.as_slice(), &[0; MAX_BLOB + 1]].concat(),
        ] {
            assert!(encrypted_payload(&encoded).is_err());
        }
        let mut unknown = [MAGIC.as_slice(), &[1]].concat();
        unknown[7] = 2;
        assert!(encrypted_payload(&unknown).is_err());
    }
    #[test]
    fn format_preserves_exact_ciphertext() {
        let ciphertext = [1, 2, 3, 4];
        let encoded = [MAGIC.as_slice(), &ciphertext].concat();
        assert_eq!(encrypted_payload(&encoded).unwrap(), ciphertext);
    }
    #[test]
    fn wipe_clears_all_bytes() {
        let mut bytes = [42; 32];
        wipe(&mut bytes);
        assert_eq!(bytes, [0; 32]);
    }
}
