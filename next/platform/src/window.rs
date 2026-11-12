/// The window AntiWebUI opened, set up as AntiWebUI's options cannot: its least size, its frame
/// kept from one run to the next, its buttons, and coming to the front.
pub struct MainWindow {
    #[cfg(target_os = "macos")]
    window: crate::mac::window::Window,
}

impl MainWindow {
    /// The window titled `title`. AntiWebUI opens it after `App::run`'s root has run, so look from
    /// its first frame on (`request_frame`). Off the main thread there is none.
    pub fn find(title: &str) -> Option<Self> {
        #[cfg(target_os = "macos")]
        return crate::mac::window::Window::find(title).map(|window| Self { window });
        #[cfg(not(target_os = "macos"))]
        {
            let _ = title;
            None
        }
    }

    /// Keeps what is inside the window's frame from being made smaller than `width` by `height`
    /// logical pixels. AntiWebUI never sets it.
    pub fn set_smallest_size(&self, smallest: (i32, i32)) {
        #[cfg(target_os = "macos")]
        self.window.set_smallest_size(smallest);
        #[cfg(not(target_os = "macos"))]
        let _ = smallest;
    }

    /// Moves and sizes the window as it was when it last moved or changed size under `name`, if it
    /// ever did, and keeps that under `name` from now on. AppKit saves it in the app's defaults,
    /// and moves it onto a screen that is there.
    pub fn keep_frame(&self, name: &str) {
        #[cfg(target_os = "macos")]
        self.window.keep_frame(name);
        #[cfg(not(target_os = "macos"))]
        let _ = name;
    }

    /// Shows the close, minimize and zoom buttons where AppKit puts them, over the top left of what
    /// the window shows. AntiWebUI hides them from a window that draws its own title bar
    /// (`DecorationMode::Client`), once as it opens it.
    pub fn show_window_buttons(&self) {
        #[cfg(target_os = "macos")]
        self.window.show_window_buttons();
    }

    /// Activates the app and makes the window the key window, in front of the others.
    pub fn bring_to_front(&self) {
        #[cfg(target_os = "macos")]
        self.window.bring_to_front();
    }
}
