use anti_webui::prelude::{Signal, on_cleanup, signal, ui_channel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};

use super::observer::Observer;

pub(crate) fn reduces_motion() -> Option<Signal<bool>> {
    let mtm = MainThreadMarker::new()?;
    let workspace = NSWorkspace::sharedWorkspace();
    let reduced = signal(workspace.accessibilityDisplayShouldReduceMotion());
    let to_ui = ui_channel(move |now| reduced.set_if_changed(now));
    let observer = {
        let workspace = workspace.clone();
        Observer::new(mtm, move || {
            to_ui.send(workspace.accessibilityDisplayShouldReduceMotion())
        })
    };
    // The workspace posts this to a center of its own, not to the default one
    let center = workspace.notificationCenter();
    // SAFETY: an `Observer` answers `changed:` with the notification, and it is removed at the
    // cleanup below, before it can go; the name is a constant of AppKit, there since macOS 10.10
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(changed:),
            Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
            None,
        );
    }
    on_cleanup(move || {
        // SAFETY: the observer added above
        unsafe { center.removeObserver(&observer) };
    });
    Some(reduced)
}
