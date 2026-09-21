//! Narrow safe native filesystem queries. These are metadata snapshots, not a
//! secure path traversal or content-reader API. Receivers need a separate boundary.
#![allow(unsafe_code)]
use std::{
    cmp::Ordering,
    fs::OpenOptions,
    io,
    mem::size_of,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::TRUE,
    Globalization::{CSTR_EQUAL, CSTR_GREATER_THAN, CSTR_LESS_THAN, CompareStringOrdinal},
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FileBasicInfo,
        FileIdInfo, GetFileInformationByHandleEx,
    },
};

/// Locale-independent Win32 case-insensitive ordering, without normalization.
///
/// # Errors
/// Returns an error on oversized strings or a failed system comparison.
pub fn compare_names_ignore_case(a: &str, b: &str) -> io::Result<Ordering> {
    if a.is_empty() || b.is_empty() {
        return Ok(a.is_empty().cmp(&b.is_empty()).reverse());
    }
    let a: Vec<_> = a.encode_utf16().collect();
    let b: Vec<_> = b.encode_utf16().collect();
    let alen = i32::try_from(a.len()).map_err(|_| io::Error::other("name too long"))?;
    let blen = i32::try_from(b.len()).map_err(|_| io::Error::other("name too long"))?;
    // SAFETY: the valid UTF-16 slices stay alive and their explicit lengths fit i32.
    match unsafe { CompareStringOrdinal(a.as_ptr(), alen, b.as_ptr(), blen, TRUE) } {
        CSTR_EQUAL => Ok(Ordering::Equal),
        CSTR_LESS_THAN => Ok(Ordering::Less),
        CSTR_GREATER_THAN => Ok(Ordering::Greater),
        _ => Err(io::Error::last_os_error()),
    }
}

/// Handle-derived identity and change stamps; no file contents are read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceStamp {
    pub volume: u64,
    pub file_id: [u8; 16],
    pub modified: i64,
    pub changed: i64,
    pub created: i64,
}

/// Open only for metadata, including directories, and reject final reparse points.
///
/// # Errors
/// Fails closed if native identity/change information is unavailable. The caller
/// must not treat this path-based snapshot as a race-free content read capability.
pub fn source_stamp(path: &Path) -> io::Result<(std::fs::Metadata, SourceStamp)> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut basic = FILE_BASIC_INFO::default();
    let mut id = FILE_ID_INFO::default();
    // SAFETY: the file owns a live handle. Each writable buffer has the exact
    // corresponding Win32 struct layout/size and lives until the call returns.
    let basic_ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&raw mut basic).cast(),
            u32::try_from(size_of::<FILE_BASIC_INFO>())
                .map_err(|_| io::Error::other("native structure size overflow"))?,
        )
    };
    if basic_ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if basic.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::other("reparse source is not allowed"));
    }
    // SAFETY: as above, this buffer matches FileIdInfo and is valid for the call.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&raw mut id).cast(),
            u32::try_from(size_of::<FILE_ID_INFO>())
                .map_err(|_| io::Error::other("native structure size overflow"))?,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((
        file.metadata()?,
        SourceStamp {
            volume: id.VolumeSerialNumber,
            file_id: id.FileId.Identifier,
            modified: basic.LastWriteTime,
            changed: basic.ChangeTime,
            created: basic.CreationTime,
        },
    ))
}
