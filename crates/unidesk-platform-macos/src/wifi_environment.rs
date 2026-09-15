//! Read only the associated SSID. No scans or geographic location collection.
#![allow(unsafe_code)]
use objc2::{
    class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_foundation::NSString;
use std::cell::RefCell;

#[link(name = "CoreWLAN", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}

pub fn current_ssid() -> Option<String> {
    autoreleasepool(|_| {
        // SAFETY: CoreWLAN objects use their documented object-returning methods;
        // retained references stay inside this worker and nil is handled.
        unsafe {
            let client: Option<Retained<AnyObject>> =
                msg_send![class!(CWWiFiClient), sharedWiFiClient];
            let interface: Option<Retained<AnyObject>> = msg_send![client.as_ref()?, interface];
            let ssid: Option<Retained<NSString>> = msg_send![interface.as_ref()?, ssid];
            ssid.map(|s| s.to_string())
                .filter(|s| !s.is_empty() && s.len() <= 32)
        }
    })
}
thread_local! {
    static LOCATION_MANAGER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}
/// Called only by a user click on the native main thread.
pub fn request_ssid_access() {
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    LOCATION_MANAGER.with(|slot| {
        let mut manager = slot.borrow_mut();
        // SAFETY: CLLocationManager is created, retained and used on the UI
        // thread; this requests authorization without starting location updates.
        unsafe {
            let manager = manager.get_or_insert_with(|| msg_send![class!(CLLocationManager), new]);
            let status: i32 = msg_send![&**manager, authorizationStatus];
            if status == 1 || status == 2 {
                let _ = std::process::Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices").spawn();
            } else {
                let _: () = msg_send![&**manager, requestWhenInUseAuthorization];
            }
        }
    });
}
