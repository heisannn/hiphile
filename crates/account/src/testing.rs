use gpui_kit::{
    AppContext as _, Context, IntoElement, Render, TestAppContext, VisualTestContext, Window, div,
};
use hiphile_core::{Caches, Hiphile};
use tempfile::TempDir;

/// A server that cannot be reached.
pub(crate) const NOWHERE: &str = "http://127.0.0.1:9";

/// A core whose settings file holds `config`, in a folder of its own.
pub(crate) fn hiphile(config: &str, cx: &mut TestAppContext) -> (TempDir, Hiphile) {
    let dir = tempfile::tempdir().unwrap();
    let config = format!("romaji_search = false\n{config}");
    std::fs::write(dir.path().join("config.toml"), config).unwrap();
    let executor = cx.update(|cx| gpui_host::executor(cx));
    let net = net::Client::new("hiphile-test").unwrap();
    let store = Ok(config::Store::at(dir.path()));
    (dir, Hiphile::new(executor, net, store, None, Caches::None))
}

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A window with nothing in it, to open dialogs in: they live in its root.
pub(crate) fn window(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(gpui_kit::init);
    let (handle, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Host)
            })
        })
        .unwrap();
    VisualTestContext::from_window(handle, cx).into_mut()
}

/// Lets what is under way finish, with answers arriving off the executor.
pub(crate) fn settle(cx: &mut VisualTestContext) {
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    cx.run_until_parked();
}
