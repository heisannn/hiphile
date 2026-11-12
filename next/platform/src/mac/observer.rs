use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSDictionary, NSKeyValueChangeKey, NSNotification, NSString};

define_class!(
    // SAFETY: NSObject has no requirements for subclasses, and this has no `Drop`
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HiphileNextObserver"]
    #[ivars = Box<dyn Fn()>]
    pub(crate) struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    impl Observer {
        #[unsafe(method(changed:))]
        fn changed(&self, _notification: &NSNotification) {
            (self.ivars())();
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut c_void,
        ) {
            (self.ivars())();
        }
    }
);

impl Observer {
    /// Calls `changed` for a notification sent to `changed:`, and for a change of the key path it
    /// observes. `changed` runs inside AppKit, so it only sends on to the UI thread.
    pub(crate) fn new(mtm: MainThreadMarker, changed: impl Fn() + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Box::new(changed) as Box<dyn Fn()>);
        // SAFETY: `init` of NSObject, on an instance just allocated
        unsafe { msg_send![super(this), init] }
    }
}
