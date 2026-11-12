//! Driving views without a window: synthetic input, frames, and finding what is shown by role and
//! name or by text. Nothing here needs a GPU.

use std::time::{Duration, Instant};

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, AccessNode, perform, snapshot};
use anti_webui::core::event::{Key, KeyEvent, Modifiers, NamedKey, PointerButton, WindowEvent};
use anti_webui::core::geometry::{PointF, Scale, SizeI};
use anti_webui::core::tree::{NodeKind, with_doc};
use anti_webui::prelude::{IntoView, Role};

/// The size the main window opens at.
pub const WINDOW: (i32, i32) = (1200, 760);

/// Shows `view` in a focused window of [`WINDOW`]'s size.
pub fn start<V: IntoView>(view: impl FnOnce() -> V) -> Ui {
    start_at(view, WINDOW)
}

pub fn start_at<V: IntoView>(view: impl FnOnce() -> V, (width, height): (i32, i32)) -> Ui {
    let mut ui = Ui::new(view);
    ui.handle_event(WindowEvent::Resized {
        size: SizeI::new(width, height),
        scale: Scale::ONE,
    });
    ui.handle_event(WindowEvent::Focused(true));
    ui.update();
    let _ = ui.paint(Instant::now());
    ui
}

/// Takes what has come to the UI thread and lays it out until `done`, as the core's work arrives
/// from other threads.
pub fn wait_until(ui: &mut Ui, mut done: impl FnMut() -> bool) {
    for _ in 0..5000 {
        ui.poll_images();
        ui.update();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("what was waited for never happened");
}

/// Plays frames for this long, as animations need.
pub fn frames(ui: &mut Ui, milliseconds: u64) {
    let start = Instant::now();
    for t in (0..=milliseconds).step_by(8) {
        let now = start + Duration::from_millis(t);
        ui.animate(now);
        ui.update();
        let _ = ui.paint(now);
    }
}

/// The first node with `role` and `label`. A name taken from what a node shows can start with an
/// icon, which is a character of a font's own and is passed over.
pub fn find(role: Role, label: &str) -> Option<AccessNode> {
    snapshot()
        .nodes
        .into_iter()
        .find(|node| node.role == role && node.label.as_deref().map(without_icons) == Some(label))
}

fn without_icons(label: &str) -> &str {
    let is_icon = |c: char| ('\u{e000}'..='\u{f8ff}').contains(&c);
    label.trim_start_matches(|c: char| is_icon(c) || c.is_whitespace())
}

pub fn nodes(role: Role) -> Vec<AccessNode> {
    snapshot()
        .nodes
        .into_iter()
        .filter(|node| node.role == role)
        .collect()
}

/// Presses the node with `role` and `label`, as assistive technology does, which also reaches
/// what is in a popup.
pub fn press(ui: &mut Ui, role: Role, label: &str) {
    let node = find(role, label).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
    assert!(!node.disabled, "{role:?} {label:?} is disabled");
    perform(node.id, AccessAction::Click);
    ui.update();
}

pub fn focus(ui: &mut Ui, role: Role, label: &str) {
    let node = find(role, label).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
    perform(node.id, AccessAction::Focus);
    ui.update();
}

pub fn click(ui: &mut Ui, x: f32, y: f32) {
    let position = PointF::new(x, y);
    ui.handle_event(WindowEvent::PointerMoved {
        position,
        modifiers: Modifiers::default(),
    });
    for pressed in [true, false] {
        ui.handle_event(WindowEvent::PointerButton {
            position,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
            time: 0,
        });
    }
    ui.update();
}

pub fn click_text(ui: &mut Ui, text: &str) {
    let (x, y) = middle_of(text);
    click(ui, x, y);
}

/// Types `text` where the focus is.
pub fn typed(ui: &mut Ui, text: &str) {
    for c in text.chars() {
        let c = c.to_string();
        ui.handle_event(WindowEvent::Key(KeyEvent {
            key: Key::Character(c.clone()),
            text: Some(c),
            modifiers: Modifiers::default(),
            pressed: true,
            repeat: false,
        }));
    }
    ui.update();
}

pub fn key(ui: &mut Ui, named: NamedKey) {
    key_with(ui, Key::Named(named), Modifiers::default());
}

pub fn key_with(ui: &mut Ui, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        ui.handle_event(WindowEvent::Key(KeyEvent {
            key: key.clone(),
            text: None,
            modifiers,
            pressed,
            repeat: false,
        }));
    }
    ui.update();
}

pub fn has_text(text: &str) -> bool {
    with_doc(|doc| {
        doc.nodes
            .iter()
            .any(|(_, node)| matches!(&node.kind, NodeKind::Text(shown) if shown.text == text))
    })
}

/// The middle of the node that shows `text`, in the window.
pub fn middle_of(text: &str) -> (f32, f32) {
    let id = with_doc(|doc| {
        doc.nodes.iter().find_map(|(id, node)| match &node.kind {
            NodeKind::Text(shown) if shown.text == text => Some(id),
            _ => None,
        })
    })
    .unwrap_or_else(|| panic!("no text {text:?}"));
    let rect = with_doc(|doc| doc.window_rect(id)).expect("a place");
    (
        rect.x as f32 + rect.width as f32 / 2.0,
        rect.y as f32 + rect.height as f32 / 2.0,
    )
}

/// A server that cannot be reached.
pub const NOWHERE: &str = "http://127.0.0.1:9";

/// What a test keeps until it ends: the core's runtime and the folder of its settings.
pub struct TestCore {
    runtime: Option<tokio::runtime::Runtime>,
    /// Dropped after the runtime: the last of a client must not go inside it, as the runtime's
    /// tasks that hold the core would otherwise drop it there.
    nets: Vec<net::Client>,
    _dir: tempfile::TempDir,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        // Without waiting for blocking work, which may wait on a server that never answers
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// A core whose settings file holds `config`, in a folder of its own, lent to this UI thread.
pub fn core(config: &str) -> (TestCore, hiphile_core::Hiphile) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let executor = executor::Executor::tokio(runtime.handle().clone());
    next_host::install(executor.clone());
    let dir = tempfile::tempdir().unwrap();
    let config = format!("romaji_search = false\n{config}");
    std::fs::write(dir.path().join("config.toml"), config).unwrap();
    let net = net::Client::new("hiphile-test").unwrap();
    let store = Ok(config::Store::at(dir.path()));
    let hiphile = hiphile_core::Hiphile::new(
        executor,
        net.clone(),
        store,
        None,
        hiphile_core::Caches::None,
    );
    (
        TestCore {
            runtime: Some(runtime),
            nets: vec![net],
            _dir: dir,
        },
        hiphile,
    )
}

/// A core signed in as alice on `server`, as far as the views can tell: nothing is written where
/// the user's own library is.
pub fn signed_in_core(server: &str) -> (TestCore, hiphile_core::Hiphile) {
    let (mut core, hiphile) = core("");
    let session = hiphile_core::test_support::session(server);
    core.nets.push(session.client().net().clone());
    hiphile.open_workspace(session, config::Profile::new(server, "alice"));
    (core, hiphile)
}
