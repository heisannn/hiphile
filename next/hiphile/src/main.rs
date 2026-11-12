use std::time::Duration;

use anti_webui::prelude::*;
use executor::Executor;
use hiphile_core::{Caches, Hiphile};
use next_hiphile::{WINDOW, app};

fn main() {
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
        .title("Hiphile")
        .app_id("dev.heisannn.hiphile.next")
        .size(WINDOW.0, WINDOW.1)
        .decorations(DecorationMode::Server)
        .run(move || {
            next_host::install(executor);
            app(shown, signal(false))
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
