use anti_webui::prelude::{Signal, signal};

/// Whether the system shows its dark appearance, following it for as long as the owner this is
/// made in lives. Call inside `App::run`, in an owner. Off the main thread it stays false.
pub fn system_is_dark() -> Signal<bool> {
    #[cfg(target_os = "macos")]
    if let Some(dark) = crate::mac::appearance::system_is_dark() {
        return dark;
    }
    signal(false)
}
