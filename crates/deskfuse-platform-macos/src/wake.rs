//! A scoped NSWorkspace wake observer. No UI or network work runs in the callback.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod native {
    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
    use objc2_foundation::{NSNotificationCenter, NSObjectProtocol};

    pub struct MacOSWakeObserver {
        center: Retained<NSNotificationCenter>,
        observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }
    impl MacOSWakeObserver {
        pub fn new(on_wake: impl Fn() + Send + Sync + 'static) -> Self {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            Self::observe(center, on_wake)
        }
        fn observe(
            center: Retained<NSNotificationCenter>,
            on_wake: impl Fn() + Send + Sync + 'static,
        ) -> Self {
            let block = RcBlock::new(move |_| on_wake());
            // SAFETY: The notification is an AppKit constant. No object filter
            // or operation queue is supplied; the callback captures only a
            // Send + Sync closure and never dereferences the notification.
            // The center copies the block; the retained token is removed in Drop.
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceDidWakeNotification),
                    None,
                    None,
                    &block,
                )
            };
            Self { center, observer }
        }
    }
    impl Drop for MacOSWakeObserver {
        fn drop(&mut self) {
            // SAFETY: This is the retained observer registered with this exact
            // center. The callback owns its captures, including during delivery.
            unsafe {
                self.center.removeObserver((*self.observer).as_ref());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        #[test]
        fn scoped_wake_observer_delivers_and_unregisters() {
            let center = NSNotificationCenter::new();
            let count = Arc::new(AtomicUsize::new(0));
            let callback_count = count.clone();
            let observer = MacOSWakeObserver::observe(center.clone(), move || {
                callback_count.fetch_add(1, Ordering::Relaxed);
            });
            // SAFETY: Use a private test center, never post a synthetic OS wake.
            unsafe {
                center.postNotificationName_object(NSWorkspaceDidWakeNotification, None);
            }
            assert_eq!(count.load(Ordering::Relaxed), 1);
            drop(observer);
            unsafe {
                center.postNotificationName_object(NSWorkspaceDidWakeNotification, None);
            }
            assert_eq!(count.load(Ordering::Relaxed), 1);
        }
    }
}
#[cfg(target_os = "macos")]
pub use native::MacOSWakeObserver;

#[cfg(not(target_os = "macos"))]
pub struct MacOSWakeObserver;
#[cfg(not(target_os = "macos"))]
impl MacOSWakeObserver {
    pub fn new(_on_wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self
    }
}
