//! NSPasteboard change observer using distributed notifications.
//!
//! macOS NSPasteboard posts distributed notifications when content changes.
//! This observer uses NSDistributedNotificationCenter to receive immediate
//! event-driven notifications with no polling overhead.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::class;
use objc2_foundation::{NSNotificationName, NSObjectProtocol};
use std::sync::Arc;
use std::sync::Mutex;

/// Callback invoked when pasteboard content changes.
pub type ChangeCallback = Box<dyn Fn(i64) + Send + 'static>;

/// Event-driven observer for NSPasteboard changes.
pub struct PasteboardObserver {
    center: Retained<AnyObject>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    callbacks: Arc<Mutex<Vec<ChangeCallback>>>,
}

impl PasteboardObserver {
    /// Create and start a new pasteboard observer.
    ///
    /// The observer registers for NSDistributedNotificationCenter notifications
    /// and invokes callbacks immediately when the pasteboard changes.
    /// This is true event-driven monitoring with no polling overhead.
    #[must_use]
    pub fn new() -> Self {
        let callbacks = Arc::new(Mutex::new(Vec::<ChangeCallback>::new()));
        
        // Get NSDistributedNotificationCenter default center
        // SAFETY: NSDistributedNotificationCenter is a standard Cocoa class.
        #[allow(unsafe_code)]
        let center: Retained<AnyObject> = unsafe {
            objc2::msg_send![class!(NSDistributedNotificationCenter), defaultCenter]
        };
        
        let callback_ref = callbacks.clone();
        let block = RcBlock::new(move |_notification: *mut AnyObject| {
            let count = Self::read_change_count();
            if let Ok(cbs) = callback_ref.lock() {
                for callback in cbs.iter() {
                    callback(count);
                }
            }
        });
        
        // Listen for "com.apple.pasteboard.changed" notifications
        let notification_name = NSNotificationName::from_str("com.apple.pasteboard.changed");
        
        // SAFETY: NSDistributedNotificationCenter is thread-safe.
        // The block captures only Send+Sync data and the notification center
        // retains the block. The observer token is removed in Drop.
        #[allow(unsafe_code)]
        let observer: Retained<ProtocolObject<dyn NSObjectProtocol>> = unsafe {
            objc2::msg_send![
                &center,
                addObserverForName: Some(&*notification_name),
                object: std::ptr::null::<AnyObject>(),
                queue: std::ptr::null::<AnyObject>(),
                usingBlock: &*block
            ]
        };
        
        Self {
            center,
            observer,
            callbacks,
        }
    }

    /// Register a callback to be invoked when the pasteboard changes.
    ///
    /// The callback receives the new change count as its argument.
    pub fn on_change<F>(&self, callback: F)
    where
        F: Fn(i64) + Send + 'static,
    {
        if let Ok(mut cbs) = self.callbacks.lock() {
            cbs.push(Box::new(callback));
        }
    }

    /// Read the current pasteboard change count.
    fn read_change_count() -> i64 {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        pasteboard.changeCount() as i64
    }

    /// Stop the observer and unregister from notifications.
    pub fn stop(&mut self) {
        // SAFETY: This is the retained observer registered with this exact center.
        #[allow(unsafe_code)]
        unsafe {
            let _: () = objc2::msg_send![&self.center, removeObserver: &*self.observer];
        }
    }
}

impl Drop for PasteboardObserver {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Default for PasteboardObserver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn observer_detects_pasteboard_changes() {
        let observer = PasteboardObserver::new();
        let change_count = Arc::new(AtomicUsize::new(0));
        let test_count = change_count.clone();

        observer.on_change(move |_count| {
            test_count.fetch_add(1, Ordering::Relaxed);
        });

        // Write to the pasteboard
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let text = objc2_foundation::NSString::from_str("test change");
        #[allow(unsafe_code)]
        unsafe {
            pasteboard.setString_forType(&text, objc2_app_kit::NSPasteboardTypeString);
        }

        // Give time for the notification to be delivered
        thread::sleep(Duration::from_millis(100));

        assert!(
            change_count.load(Ordering::Relaxed) > 0,
            "observer should detect pasteboard change"
        );
    }

    #[test]
    fn observer_stops_cleanly() {
        let mut observer = PasteboardObserver::new();
        let change_count = Arc::new(AtomicUsize::new(0));
        let test_count = change_count.clone();

        observer.on_change(move |_count| {
            test_count.fetch_add(1, Ordering::Relaxed);
        });

        observer.stop();

        // Write after stopping should not trigger callback
        let before = change_count.load(Ordering::Relaxed);
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let text = objc2_foundation::NSString::from_str("after stop");
        #[allow(unsafe_code)]
        unsafe {
            pasteboard.setString_forType(&text, objc2_app_kit::NSPasteboardTypeString);
        }

        thread::sleep(Duration::from_millis(100));
        let after = change_count.load(Ordering::Relaxed);

        assert_eq!(
            before, after,
            "stopped observer should not detect changes"
        );
    }
}
