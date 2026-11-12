use std::ptr;

use anti_webui::prelude::{Signal, on_cleanup, signal, ui_channel};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
use objc2_foundation::{
    NSArray, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, ns_string,
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
    let key = ns_string!("effectiveAppearance");
    // SAFETY: an `Observer` answers `observeValueForKeyPath:ofObject:change:context:`, and it stops
    // observing at the cleanup below, before it can go; AppKit documents this key path as observable
    unsafe {
        app.addObserver_forKeyPath_options_context(
            &observer,
            key,
            NSKeyValueObservingOptions::empty(),
            ptr::null_mut(),
        );
    }
    on_cleanup(move || {
        // SAFETY: the observation added above
        unsafe { app.removeObserver_forKeyPath(&observer, key) };
    });
    Some(dark)
}

fn is_dark(app: &NSApplication) -> bool {
    // SAFETY: constants of AppKit, there since macOS 10.14
    let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    app.effectiveAppearance()
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[light, dark]))
        .is_some_and(|best| *best == *dark)
}
