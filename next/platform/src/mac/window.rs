use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSWindow, NSWindowButton};
use objc2_foundation::{NSSize, NSString};

pub(crate) struct Window {
    window: Retained<NSWindow>,
    mtm: MainThreadMarker,
}

impl Window {
    pub(crate) fn find(title: &str) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let title = NSString::from_str(title);
        let window = NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .find(|window| window.title() == title)?;
        Some(Self { window, mtm })
    }

    pub(crate) fn set_smallest_size(&self, (width, height): (i32, i32)) {
        self.window
            .setContentMinSize(NSSize::new(f64::from(width), f64::from(height)));
    }

    pub(crate) fn keep_frame(&self, name: &str) {
        let name = NSString::from_str(name);
        self.window.setFrameUsingName(&name);
        self.window.setFrameAutosaveName(&name);
    }

    pub(crate) fn show_window_buttons(&self) {
        for button in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = self.window.standardWindowButton(button) {
                button.setHidden(false);
            }
        }
    }

    pub(crate) fn bring_to_front(&self) {
        NSApplication::sharedApplication(self.mtm).activate();
        self.window.makeKeyAndOrderFront(None);
    }
}
