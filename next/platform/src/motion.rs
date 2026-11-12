use anti_webui::prelude::{Signal, signal};

/// Whether the system asks for less motion (Accessibility > Display > Reduce motion), following it
/// for as long as the owner this is made in lives. Call inside `App::run`, in an owner. Off the main
/// thread it stays false.
pub fn reduces_motion() -> Signal<bool> {
    #[cfg(target_os = "macos")]
    if let Some(reduced) = crate::mac::motion::reduces_motion() {
        return reduced;
    }
    signal(false)
}
