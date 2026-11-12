use std::ptr;

use anti_webui::prelude::{Signal, on_cleanup, signal, ui_channel};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
use objc2_foundation::{
    NSArray, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSString,
    ns_string,
};

use super::observer::Observer;

pub(crate) fn system_is_dark() -> Option<Signal<bool>> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let dark = signal(is_dark(&app));
    let to_ui = ui_channel(move |now| dark.set_if_changed(now));
    let observer = {
        let app = app.clone();
        Observer::new(mtm, move || to_ui.send(is_dark(&app)))
    };
    // SAFETY: an `Observer` answers `observeValueForKeyPath:ofObject:change:context:`, and the
    // `Observation` below holds it until it stops observing; AppKit documents this key path as
    // observable
    unsafe {
        app.addObserver_forKeyPath_options_context(
            &observer,
            key(),
            NSKeyValueObservingOptions::empty(),
            ptr::null_mut(),
        );
    }
    let observation = Observation { app, observer };
    // Without an owner this is dropped at once, which stops observing too
    on_cleanup(move || drop(observation));
    Some(dark)
}

fn key() -> &'static NSString {
    ns_string!("effectiveAppearance")
}

/// The app's appearance, observed until this is dropped. KVO does not keep the observer alive, so
/// this does for as long as it observes.
struct Observation {
    app: Retained<NSApplication>,
    observer: Retained<Observer>,
}

impl Drop for Observation {
    fn drop(&mut self) {
        // SAFETY: the observation that `system_is_dark` added
        unsafe { self.app.removeObserver_forKeyPath(&self.observer, key()) };
    }
}

fn is_dark(app: &NSApplication) -> bool {
    // SAFETY: constants of AppKit, there since macOS 10.14
    let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    app.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[light, dark]))
        .is_some_and(|best| *best == *dark)
}
