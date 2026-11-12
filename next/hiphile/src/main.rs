use std::time::Duration;

use anti_webui::prelude::*;
use executor::Executor;
use hiphile_core::{Caches, Hiphile};
use next_hiphile::menu_bar::menus;
use next_hiphile::{MenuBar, SMALLEST_WINDOW, TITLE, WINDOW, app};
use next_platform::appearance::system_is_dark;
use next_platform::locale::set_lang;
use next_platform::menu::MainMenu;
use next_platform::window::MainWindow;

fn main() {
    // Before any thread starts, which the core's runtime is the first to do
    set_lang();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("hiphile-core")
        .build()
        .expect("failed to start the core's runtime");
    let user_agent = concat!("hiphile/", env!("CARGO_PKG_VERSION"));
    let net = net::Client::new(user_agent).expect("failed to start the network runtime");
    let executor = Executor::tokio(runtime.handle().clone());
    let hiphile = Hiphile::new(
        executor.clone(),
        net.clone(),
        config::Store::user(),
        config::Keychain::system().ok(),
        Caches::User,
    );

    let shown = hiphile.clone();
    let ran = App::new()
        .title(TITLE)
        .app_id("dev.heisannn.hiphile.next")
        .size(WINDOW.0, WINDOW.1)
        .decorations(DecorationMode::Client)
        .run(move || {
            next_host::install(executor);
            let menu_bar = MenuBar::new();
            let main_menu = MainMenu::install(&menus(), move |command| menu_bar.choose(command));
            effect(move || main_menu.update(|command| menu_bar.state_of(command)));
            // AntiWebUI opens the window once this has run
            request_frame(|_| set_up_window());
            app(shown, system_is_dark(), menu_bar)
        });

    // Before the runtime goes: saving and putting the device back as the player found it run on it
    hiphile.shutdown();
    drop(hiphile);
    // The library's sync blocks a thread of the runtime, which a plain drop would wait for
    runtime.shutdown_timeout(Duration::from_secs(2));
    // Last, and outside the runtime, as the network client requires
    drop(net);
    ran.expect("failed to run the window");
}

/// What AntiWebUI's options leave out of the window.
fn set_up_window() {
    let Some(window) = MainWindow::find(TITLE) else {
        return;
    };
    window.set_smallest_size(SMALLEST_WINDOW);
    window.keep_frame("Main Window");
    window.show_window_buttons();
    window.bring_to_front();
}
