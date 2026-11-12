//! What the tests of several modules share.

use std::time::Duration;

use executor::Executor;

/// The core's work on threads of a runtime, as in the app.
pub(crate) struct Runtime(#[allow(dead_code)] tokio::runtime::Runtime);

pub(crate) fn executor() -> (Runtime, Executor) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    let executor = Executor::tokio(runtime.handle().clone());
    (Runtime(runtime), executor)
}

/// Takes what has come to the UI thread until `done`, as the work arrives from other threads.
pub(crate) fn wait_until(mut done: impl FnMut() -> bool) {
    for _ in 0..2000 {
        anti_webui::core::channel::pump();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("what was waited for never happened");
}
