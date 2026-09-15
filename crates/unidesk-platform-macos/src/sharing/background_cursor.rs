//! WindowServer cursor control for a background input-sharing process.
//!
//! Quartz hide/show normally only affect the foreground application. Resolve
//! this private connection property at runtime so a missing macOS entry point
//! fails remote entry instead of preventing the application from launching.
use core_foundation::{base::TCFType, boolean::CFBoolean, string::CFString};
use core_foundation_sys::{base::CFTypeRef, string::CFStringRef};
use std::{ffi::c_void, sync::OnceLock};

/// Process-wide resolved entry points, warmed when capture starts.
static API: OnceLock<Option<BackgroundCursorApi>> = OnceLock::new();

type DefaultConnection = unsafe extern "C" fn() -> i32;
type SetConnectionProperty = unsafe extern "C" fn(i32, i32, CFStringRef, CFTypeRef) -> i32;

struct BackgroundCursorApi {
    connection: DefaultConnection,
    set_property: SetConnectionProperty,
}

#[link(name = "System")]
unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}

impl BackgroundCursorApi {
    fn load() -> Option<Self> {
        // RTLD_DEFAULT from the macOS SDK dlfcn.h searches already-loaded
        // images. CoreGraphics is linked for the process lifetime; no dlclose
        // can invalidate these entry points.
        let scope = -2_isize as *mut c_void;
        // SAFETY: both names are static NUL-terminated strings and scope is
        // Darwin's documented special dlsym handle.
        let (connection, set_property) = unsafe {
            (
                dlsym(scope, c"_CGSDefaultConnection".as_ptr()),
                dlsym(scope, c"CGSSetConnectionProperty".as_ptr()),
            )
        };
        if connection.is_null() || set_property.is_null() {
            return None;
        }
        // SAFETY: non-null symbols have the WindowServer C ABI represented by
        // the concrete function types. Code pointers live in retained system
        // frameworks, and are never treated as data or freed.
        Some(unsafe {
            Self {
                connection: std::mem::transmute::<*mut c_void, DefaultConnection>(connection),
                set_property: std::mem::transmute::<*mut c_void, SetConnectionProperty>(
                    set_property,
                ),
            }
        })
    }
}

/// Resolve the private entry points ahead of the first handover.
///
/// `dlsym` can pull CoreGraphics in, which measured ~16 ms on the first Enter
/// and blocked the event-tap thread for exactly that long; doing it when capture
/// starts keeps every later handover sub-millisecond.
pub(super) fn warm() {
    let _ = API.get_or_init(BackgroundCursorApi::load);
}

pub(super) fn enable() -> bool {
    let Some(api) = API.get_or_init(BackgroundCursorApi::load) else {
        return false;
    };
    // SAFETY: the checked function takes no arguments. Retrieve the current
    // connection each time; never cache a connection across session changes.
    let connection = unsafe { (api.connection)() };
    if connection <= 0 {
        return false;
    }
    let name = CFString::new("SetsCursorInBackground");
    let enabled = CFBoolean::true_value();
    // SAFETY: source and target are this process's own live connection. The
    // property name/value remain retained throughout the synchronous call.
    // This only opts our cursor hide/show into background operation; it does
    // not grant input capture/injection permission or change another process.
    unsafe {
        (api.set_property)(
            connection,
            connection,
            name.as_concrete_TypeRef(),
            enabled.as_CFTypeRef(),
        ) == 0
    }
}
