//! Finder destinations read through AppleScript, guarded by Accessibility and delivered independently
//! of the mutable system pasteboard. Native queries must run off the event tap.

#![allow(unsafe_code)]

use std::{
    ffi::{CString, OsStr, OsString, c_char, c_int, c_uint, c_void},
    fs::{self, File, FileTimes, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    string::CFString,
};
use core_foundation_sys::{
    base::{CFTypeID, CFTypeRef},
    string::CFStringRef,
};

// AXError.h. Keep status codes in failures so diagnostics identify the actual
// failing attribute rather than attributing every IPC error to permissions.
const AX_CANNOT_COMPLETE: i32 = -25204;
const AX_ATTRIBUTE_UNSUPPORTED: i32 = -25205;
const AX_NOT_IMPLEMENTED: i32 = -25208;
const AX_API_DISABLED: i32 = -25211;
const AX_NO_VALUE: i32 = -25212;

const O_NOFOLLOW: c_int = 0x100;
const O_CREAT: c_int = 0x200;
const O_EXCL: c_int = 0x800;
const O_DIRECTORY: c_int = 0x100000;
const O_CLOEXEC: c_int = 0x1000000;
const O_WRONLY: c_int = 1;
const AT_REMOVEDIR: c_int = 0x80;
const RENAME_EXCL: c_uint = 4;
const COPYFILE_XATTR: u32 = 1 << 2;
static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout: f32) -> i32;
}

unsafe extern "C" {
    fn openat(fd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    fn mkdirat(fd: c_int, path: *const c_char, mode: u16) -> c_int;
    fn unlinkat(fd: c_int, path: *const c_char, flags: c_int) -> c_int;
    fn fchmodat(fd: c_int, path: *const c_char, mode: u16, flags: c_int) -> c_int;
    fn renameatx_np(
        from_fd: c_int,
        from: *const c_char,
        to_fd: c_int,
        to: *const c_char,
        flags: c_uint,
    ) -> c_int;
    fn fcopyfile(from_fd: c_int, to_fd: c_int, state: *mut c_void, flags: u32) -> c_int;
}

/// Immutable directory identity. Holding its descriptor prevents a replaced path
/// or newly inserted symlink from redirecting an in-flight paste.
#[derive(Clone)]
pub struct FinderDestination {
    path: PathBuf,
    directory: Arc<File>,
    device: u64,
    inode: u64,
}

impl std::fmt::Debug for FinderDestination {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FinderDestination")
            .field("device", &self.device)
            .field("inode", &self.inode)
            .finish_non_exhaustive()
    }
}

impl FinderDestination {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn identity(&self) -> (u64, u64) {
        (self.device, self.inode)
    }

    fn validate(&self) -> Result<(), String> {
        let live = open_directory_no_symlinks(&self.path)
            .map_err(|error| format!("原粘贴目录已不可用：{error}"))?;
        let metadata = live.metadata().map_err(io_error)?;
        let held = self.directory.metadata().map_err(io_error)?;
        if (metadata.dev(), metadata.ino()) != self.identity()
            || (held.dev(), held.ino()) != self.identity()
            || held.nlink() == 0
        {
            return Err("原粘贴目录已被移动、删除或替换，请重新选择目录。".into());
        }
        Ok(())
    }
}

/// A committed top-level item. Retain these on partial failure and omit their
/// `source` paths from a retry; a committed directory is always a complete tree.
#[derive(Clone, Debug)]
pub struct FinderDeliveryItem {
    pub source: PathBuf,
    pub destination: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct DeliveryResult {
    pub items: Vec<FinderDeliveryItem>,
    pub cancelled: bool,
    pub error: Option<String>,
}

pub fn is_finder_pid(pid: i32) -> bool {
    pid > 0
        && objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .and_then(|application| application.bundleIdentifier())
            .is_some_and(|bundle| bundle.to_string() == "com.apple.finder")
}

/// Capture a local Finder directory, rejecting ambiguous/virtual locations.
/// Called once on a blocking worker after the engine publishes path acquisition.
/// The directory returned by this query becomes the delivery destination.
pub fn capture_finder_destination(target_pid: i32) -> Result<Option<FinderDestination>, String> {
    if !is_finder_pid(target_pid) {
        return Ok(None);
    }
    // SAFETY: AX creates an owned CF object for this process identifier. The
    // non-null check precedes wrapping it under the create ownership rule.
    let raw = unsafe { AXUIElementCreateApplication(target_pid) };
    if raw.is_null() {
        return Err("无法读取 Finder 的辅助功能上下文。".into());
    }
    // SAFETY: `raw` is the valid +1 CF object returned by AX above.
    let application = unsafe { CFType::wrap_under_create_rule(raw) };
    let focused = ax_attribute(&application, "AXFocusedUIElement")?;
    reject_text_focus(focused.as_ref())?;
    let window = ax_attribute(&application, "AXFocusedWindow")?;
    if let Some(window) = &window {
        reject_search_context(window)?;
    }
    let location = finder_location(window.as_ref(), focused.as_ref())?;
    let path = fs::canonicalize(&location.path)
        .map_err(|error| format!("无法打开首次粘贴目录：{error}"))?;
    let directory = open_directory_no_symlinks(&path)
        .map_err(|error| format!("无法打开首次粘贴目录：{error}"))?;
    let metadata = directory.metadata().map_err(io_error)?;

    Ok(Some(FinderDestination {
        path,
        directory: Arc::new(directory),
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}

#[derive(Debug)]
struct AxReadError {
    attribute: String,
    status: i32,
}

impl std::fmt::Display for AxReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self.status {
            AX_API_DISABLED => "辅助功能接口未获允许，请检查 DeskFuse 的辅助功能授权",
            AX_CANNOT_COMPLETE => "Finder 暂未响应目录查询，请稍候重试",
            -25202 => "Finder 窗口正在变化，请等待目标文件夹打开后重试",
            _ => "Finder 无法提供目录上下文",
        };
        write!(
            formatter,
            "{reason}（{}，AX {}）。",
            self.attribute, self.status
        )
    }
}

fn read_ax_attribute(element: &CFType, name: &str) -> Result<Option<CFType>, AxReadError> {
    let failure = |status| AxReadError {
        attribute: name.into(),
        status,
    };
    // SAFETY: querying the CF object's type needs only a valid owned reference.
    if element.type_of() != unsafe { AXUIElementGetTypeID() } {
        return Err(failure(-25202));
    }
    // AX timeouts belong to the exact object, NOT its application's descendants.
    // Bound every window/control query without changing the process-wide timeout.
    // SAFETY: element is a checked AXUIElement owned for this entire call.
    let timeout_status = unsafe { AXUIElementSetMessagingTimeout(element.as_CFTypeRef(), 0.1) };
    if timeout_status != 0 {
        return Err(failure(timeout_status));
    }
    let key = CFString::new(name);
    let mut value = ptr::null();
    // SAFETY: key and element live across the call, and value is writable.
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            key.as_concrete_TypeRef(),
            &mut value,
        )
    };
    if matches!(
        status,
        AX_ATTRIBUTE_UNSUPPORTED | AX_NO_VALUE | AX_NOT_IMPLEMENTED
    ) {
        return Ok(None);
    }
    if status != 0 || value.is_null() {
        return Err(failure(if status == 0 { AX_NO_VALUE } else { status }));
    }
    // SAFETY: successful CopyAttributeValue returns one owned CF reference.
    Ok(Some(unsafe { CFType::wrap_under_create_rule(value) }))
}

fn ax_attribute(element: &CFType, name: &str) -> Result<Option<CFType>, String> {
    let result = read_ax_attribute(element, name);
    // One bounded retry for a busy Finder; never retry permission or stale-object
    // failures. Focus, document and input epoch are still checked around capture.
    match result {
        Err(error) if error.status == AX_CANNOT_COMPLETE => {
            read_ax_attribute(element, name).map_err(|error| error.to_string())
        }
        result => result.map_err(|error| error.to_string()),
    }
}

fn cf_string(value: CFType) -> Option<String> {
    value
        .downcast::<CFString>()
        .map(|string| string.to_string())
}

fn ax_string(element: &CFType, name: &str) -> Result<Option<String>, String> {
    Ok(ax_attribute(element, name)?.and_then(cf_string))
}

/// Identifiers/descriptions supplement the native role/value checks. Finder
/// versions and individual controls need not implement these metadata attributes.
/// Their failures must not veto an otherwise verified local directory.
fn ax_metadata(element: &CFType, name: &str) -> Result<Option<String>, String> {
    match read_ax_attribute(element, name) {
        Ok(value) => Ok(value.and_then(cf_string)),
        Err(error) if error.status == AX_API_DISABLED => Err(error.to_string()),
        Err(_) => Ok(None),
    }
}

fn reject_text_focus(element: Option<&CFType>) -> Result<(), String> {
    if let Some(element) = element {
        let role = ax_string(element, "AXRole")?;
        if matches!(
            role.as_deref(),
            Some("AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField")
        ) {
            return Err("Finder 当前正在编辑文本，未将文件保存到未知目录。".into());
        }
    }
    Ok(())
}

/// Finder can keep a real target folder while showing search results. Inspect
/// navigation controls as well as the URL; a nonempty native search field is
/// sufficient to reject the location even after focus moves into result rows.
fn reject_search_context(window: &CFType) -> Result<(), String> {
    const MAX_NODES: usize = 192;
    const MAX_DEPTH: usize = 12;
    let mut pending = vec![(window.clone(), 0_usize)];
    let mut visited = 0_usize;
    while let Some((element, depth)) = pending.pop() {
        visited += 1;
        if visited > MAX_NODES || depth > MAX_DEPTH {
            return Err("Finder 窗口上下文过于复杂，无法安全确定粘贴目录。".into());
        }
        let role = ax_string(&element, "AXRole")?.unwrap_or_default();
        // Query subrole only where it distinguishes a native search field.
        // Leaf buttons/images/file rows need no optional AX attributes at all.
        let is_search = role == "AXSearchField"
            || (role == "AXTextField"
                && ax_string(&element, "AXSubrole")?.as_deref() == Some("AXSearchField"));
        if is_search {
            let value = ax_string(&element, "AXValue")?;
            if value
                .as_deref()
                .is_none_or(|value| !value.trim().is_empty())
            {
                return Err("Finder 正在显示或编辑搜索结果，请打开实际目标文件夹后粘贴。".into());
            }
        }
        // Inspect navigation containers and the root of the file area only.
        // Keep search-results metadata detection without querying every leaf.
        let navigation_container = matches!(
            role.as_str(),
            "AXWindow" | "AXToolbar" | "AXGroup" | "AXSplitGroup" | "AXLayoutArea" | "AXUnknown"
        );
        let content_container = matches!(
            role.as_str(),
            "AXScrollArea" | "AXOutline" | "AXTable" | "AXList" | "AXBrowser"
        );
        if !navigation_container && !content_container {
            continue;
        }
        for attribute in ["AXIdentifier", "AXDescription"] {
            if let Some(value) = ax_metadata(&element, attribute)? {
                let value = value.to_ascii_lowercase();
                if [
                    "searchresults",
                    "search-results",
                    "search_results",
                    "search results",
                ]
                .iter()
                .any(|marker| value.contains(marker))
                {
                    return Err("Finder 当前是搜索结果，不能作为文件保存目录。".into());
                }
            }
        }
        // File rows cannot contain the window's navigation search controls.
        if content_container {
            continue;
        }
        let Some(children) = ax_attribute(&element, "AXChildren")? else {
            continue;
        };
        let Some(children) = children.downcast::<CFArray>() else {
            return Err("Finder 返回了无效的窗口层级，无法确定粘贴目录。".into());
        };
        if children.len() as usize + pending.len() + visited > MAX_NODES {
            return Err("Finder 窗口上下文过于复杂，无法安全确定粘贴目录。".into());
        }
        for child in children.iter() {
            let raw = *child;
            if raw.is_null() {
                continue;
            }
            // SAFETY: AXChildren is a CF array of retained AXUIElements. The
            // array owns each non-null element; retain it before queuing it.
            pending.push((unsafe { CFType::wrap_under_get_rule(raw) }, depth + 1));
        }
    }
    Ok(())
}

fn finder_location(
    window: Option<&CFType>,
    focused: Option<&CFType>,
) -> Result<crate::finder_script::FinderLocation, String> {
    if let Some(window) = window {
        if ax_string(window, "AXSubrole")?.as_deref() == Some("AXDialog") {
            return Err("Finder 当前显示对话框，无法确定粘贴目录。".into());
        }
        return crate::finder_script::location(false);
    }
    // No front window alone does not imply Desktop. Require native desktop
    // focus before asking Finder for its desktop alias, never infer ~/Desktop.
    if let Some(focused) = focused
        && ax_string(focused, "AXSubrole")?.as_deref() == Some("AXDesktop")
    {
        return crate::finder_script::location(true);
    }
    Err("无法确定 Finder 当前目录或桌面焦点，请在目标文件夹内重新粘贴。".into())
}

fn io_error(error: io::Error) -> String {
    format!("文件保存失败：{error}")
}
fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "文件名包含无效字符"))
}

fn open_at(directory: RawFd, name: &Path, flags: c_int, mode: c_int) -> io::Result<File> {
    let name = c_path(name)?;
    // SAFETY: directory is an open directory FD borrowed for the call; name is
    // NUL terminated. The optional mode is promoted to int as required by C.
    let descriptor = unsafe {
        openat(
            directory,
            name.as_ptr(),
            flags | O_NOFOLLOW | O_CLOEXEC,
            mode,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful openat returns a newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn open_directory_no_symlinks(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "目录必须为绝对路径",
        ));
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        .open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                directory = open_at(directory.as_raw_fd(), Path::new(name), O_DIRECTORY, 0)?
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "目录路径无效")),
        }
    }
    Ok(directory)
}

/// Deliver from a caller-leased cache. `Err` means nothing was staged; after
/// staging begins, return partial commits and failure in `DeliveryResult`.
/// Progress counts copied data, not received network bytes.
pub fn deliver_finder_files(
    destination: &FinderDestination,
    sources: &[PathBuf],
    cancelled: &AtomicBool,
    on_progress: impl Fn(u64, u64),
) -> Result<DeliveryResult, String> {
    destination.validate()?;
    let mut total = 0_u64;
    for source in sources {
        total = total.saturating_add(source_size(source, cancelled).map_err(io_error)?);
    }
    let mut stage = Stage::create(destination).map_err(io_error)?;
    let mut result = DeliveryResult::default();
    let mut copied = 0_u64;
    on_progress(0, total);
    for (index, source) in sources.iter().enumerate() {
        let outcome = (|| -> Result<PathBuf, String> {
            check_cancelled(cancelled).map_err(io_error)?;
            destination.validate()?;
            let name = source
                .file_name()
                .ok_or_else(|| "源文件缺少名称。".to_owned())?;
            let relative = PathBuf::from(format!("item-{index}"));
            copy_tree(
                source,
                &relative,
                &mut stage,
                cancelled,
                &mut copied,
                total,
                &on_progress,
            )
            .map_err(io_error)?;
            check_cancelled(cancelled).map_err(io_error)?;
            destination.validate()?;
            let directory = source.symlink_metadata().map_err(io_error)?.is_dir();
            let committed = stage.commit(&relative, name, directory).map_err(io_error)?;
            Ok(destination.path.join(committed))
        })();
        match outcome {
            Ok(path) => result.items.push(FinderDeliveryItem {
                source: source.clone(),
                destination: path,
            }),
            Err(error) => {
                result.cancelled = cancelled.load(Ordering::Acquire);
                result.error = Some(if result.cancelled {
                    "已取消文件保存，已完成的文件仍保留。".into()
                } else {
                    error
                });
                break;
            }
        }
    }
    if let Err(error) = stage.cleanup() {
        let cleanup = format!("无法清理本次粘贴的暂存文件：{error}");
        result.error = Some(
            result
                .error
                .map_or(cleanup.clone(), |previous| format!("{previous} {cleanup}")),
        );
    }
    Ok(result)
}

fn check_cancelled(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "已取消文件保存"))
    } else {
        Ok(())
    }
}

fn source_size(source: &Path, cancelled: &AtomicBool) -> io::Result<u64> {
    check_cancelled(cancelled)?;
    let metadata = source.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "暂不支持保存设备、套接字等特殊文件",
        ));
    }
    let mut total = 0_u64;
    for entry in fs::read_dir(source)? {
        total = total.saturating_add(source_size(&entry?.path(), cancelled)?);
    }
    Ok(total)
}

struct Stage<'a> {
    destination: &'a FinderDestination,
    name: CString,
    directory: File,
    /// Entries still private to this operation, in creation order.
    created: Vec<(PathBuf, bool)>,
    removed: bool,
}

impl<'a> Stage<'a> {
    fn create(destination: &'a FinderDestination) -> io::Result<Self> {
        for _ in 0..128 {
            let number = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let name = CString::new(format!(".deskfuse-paste-{}-{number}", std::process::id()))
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "暂存名称无效"))?;
            // SAFETY: held directory FD and NUL terminated single component;
            // mode 0700 keeps unfinished content private to this user.
            if unsafe { mkdirat(destination.directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(error);
            }
            let directory = match open_at(
                destination.directory.as_raw_fd(),
                Path::new(OsStr::from_bytes(name.as_bytes())),
                O_DIRECTORY,
                0,
            ) {
                Ok(directory) => directory,
                Err(error) => {
                    // SAFETY: removes only the empty directory just created.
                    unsafe {
                        unlinkat(
                            destination.directory.as_raw_fd(),
                            name.as_ptr(),
                            AT_REMOVEDIR,
                        );
                    }
                    return Err(error);
                }
            };
            return Ok(Self {
                destination,
                name,
                directory,
                created: Vec::new(),
                removed: false,
            });
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "无法创建独立的粘贴暂存目录",
        ))
    }

    fn commit(&mut self, relative: &Path, name: &OsStr, directory: bool) -> io::Result<OsString> {
        let from = c_path(relative)?;
        for number in 0_u32..100_000 {
            let candidate = numbered_name(name, directory, number);
            let to = c_path(Path::new(&candidate))?;
            // SAFETY: source belongs to this stage; destination is the pinned
            // directory. RENAME_EXCL atomically refuses any existing entry.
            let status = unsafe {
                renameatx_np(
                    self.directory.as_raw_fd(),
                    from.as_ptr(),
                    self.destination.directory.as_raw_fd(),
                    to.as_ptr(),
                    RENAME_EXCL,
                )
            };
            if status == 0 {
                self.created.retain(|(path, _)| !path.starts_with(relative));
                return Ok(candidate);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "同名文件过多，无法生成唯一名称",
        ))
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if self.removed {
            return Ok(());
        }
        let mut first_error = None;
        // Source permissions may be read-only. Restore access only within this
        // operation's uncommitted staging tree before deleting it.
        for (path, directory) in &self.created {
            if *directory && let Ok(name) = c_path(path) {
                // SAFETY: all paths are private stage entries we created;
                // no source paths or committed destination names occur here.
                unsafe {
                    fchmodat(self.directory.as_raw_fd(), name.as_ptr(), 0o700, 0);
                }
            }
        }
        for (path, directory) in self.created.iter().rev() {
            let name = c_path(path)?;
            // SAFETY: as above; unlinkat never follows the final symlink.
            if unsafe {
                unlinkat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    if *directory { AT_REMOVEDIR } else { 0 },
                )
            } != 0
            {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::NotFound && first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        // SAFETY: only remove this operation's originally created stage name.
        if unsafe {
            unlinkat(
                self.destination.directory.as_raw_fd(),
                self.name.as_ptr(),
                AT_REMOVEDIR,
            )
        } != 0
        {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(error);
            }
        }
        self.removed = first_error.is_none();
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn numbered_name(name: &OsStr, directory: bool, number: u32) -> OsString {
    if number == 0 {
        return name.to_owned();
    }
    let path = Path::new(name);
    let mut result = if directory {
        name.to_owned()
    } else {
        path.file_stem().unwrap_or(name).to_owned()
    };
    result.push(format!(" ({number})"));
    if !directory && let Some(extension) = path.extension() {
        result.push(".");
        result.push(extension);
    }
    result
}

fn copy_tree(
    source: &Path,
    relative: &Path,
    stage: &mut Stage<'_>,
    cancelled: &AtomicBool,
    copied: &mut u64,
    total: u64,
    on_progress: &impl Fn(u64, u64),
) -> io::Result<()> {
    check_cancelled(cancelled)?;
    let metadata = source.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;
        // The private stage contains only directories we just created; links are
        // copied as leaves and are never traversed during sizing or delivery.
        rustix::fs::symlinkat(&target, &stage.directory, relative).map_err(io::Error::from)?;
        stage.created.push((relative.to_owned(), false));
    } else if metadata.is_dir() {
        let name = c_path(relative)?;
        // SAFETY: private stage FD with a relative path composed only from
        // read_dir entry names; mkdir is exclusive and never replaces an entry.
        if unsafe { mkdirat(stage.directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        stage.created.push((relative.to_owned(), true));
        let output = open_at(stage.directory.as_raw_fd(), relative, O_DIRECTORY, 0)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &relative.join(entry.file_name()),
                stage,
                cancelled,
                copied,
                total,
                on_progress,
            )?;
        }
        let input = OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
            .open(source)?;
        copy_metadata(&input, &output, &metadata)?;
    } else if metadata.is_file() {
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_CLOEXEC)
            .open(source)?;
        let opened = input.metadata()?;
        if (opened.dev(), opened.ino(), opened.len())
            != (metadata.dev(), metadata.ino(), metadata.len())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "源缓存文件在保存前发生变化",
            ));
        }
        let mut output = open_at(
            stage.directory.as_raw_fd(),
            relative,
            O_WRONLY | O_CREAT | O_EXCL,
            0o600,
        )?;
        stage.created.push((relative.to_owned(), false));
        let mut buffer = vec![0_u8; 1024 * 1024];
        let mut item_bytes = 0_u64;
        loop {
            check_cancelled(cancelled)?;
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            item_bytes = item_bytes.saturating_add(read as u64);
            *copied = copied.saturating_add(read as u64);
            on_progress((*copied).min(total), total);
        }
        if item_bytes != metadata.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "源缓存文件在保存期间发生变化",
            ));
        }
        copy_metadata(&input, &output, &metadata)?;
        output.sync_all()?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "暂不支持保存设备、套接字等特殊文件",
        ));
    }
    Ok(())
}

fn copy_metadata(input: &File, output: &File, metadata: &fs::Metadata) -> io::Result<()> {
    // SAFETY: both descriptors remain valid; a null copyfile state requests
    // system allocation. Only xattrs are copied, not immutable flags or ACLs.
    if unsafe {
        fcopyfile(
            input.as_raw_fd(),
            output.as_raw_fd(),
            ptr::null_mut(),
            COPYFILE_XATTR,
        )
    } != 0
    {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Unsupported {
            return Err(error);
        }
    }
    output.set_permissions(fs::Permissions::from_mode(metadata.mode() & 0o777))?;
    let mut times = FileTimes::new();
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    output.set_times(times)
}
