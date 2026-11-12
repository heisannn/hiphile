//! What the app on AntiWebUI asks of AppKit itself, since AntiWebUI keeps AppKit to its own: the menu
//! bar, the main window's least size, frame and buttons, the system's appearance and motion
//! settings, and `LANG`.
//!
//! AppKit is called on the main thread only, which runs the main window. Elsewhere, as in tests,
//! and on other systems, each of these does nothing.

pub mod appearance;
pub mod locale;
pub mod menu;
pub mod motion;
pub mod window;

// Objective-C through objc2 is unsafe to call; each block says why it holds
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac;
