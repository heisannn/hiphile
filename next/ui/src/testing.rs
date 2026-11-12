//! Driving views without a window: synthetic input, frames, and finding what is shown by role and
//! name or by text. Nothing here needs a GPU.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, AccessNode, perform, snapshot};
use anti_webui::core::event::{
    Key, KeyEvent, Modifiers, NamedKey, PointerButton, ScrollPhase, ScrollSource, WindowEvent,
};
use anti_webui::core::geometry::{PointF, Scale, SizeI};
use anti_webui::core::tree::{Document, NodeId, NodeKind, with_doc};
use anti_webui::prelude::{IntoView, Role, on_cleanup};
use anti_webui_md3e::components::snackbars;
use slotmap::KeyData;

use crate::{Command, Commands, Known, Overlays, ScreenContext};

mod library;

pub use library::{SavedLibrary, save_library, signed_in_core_with_library};

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

/// Whether the node with `role` and `label` shows that the keys are on it, with the ring md3e's
/// components show their focus with, on it or on what is in it.
pub fn shows_keys(role: Role, label: &str) -> bool {
    let node = find(role, label).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
    with_doc(|d| {
        let mut next = vec![KeyData::from_ffi(node.id).into()];
        while let Some(id) = next.pop() {
            let Some(node) = d.get(id) else {
                continue;
            };
            if node.computed.outline_width > 0.0 {
                return true;
            }
            next.extend(node.children.iter().copied());
        }
        false
    })
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
    let node = reachable(role, label);
    assert!(!node.disabled, "{role:?} {label:?} is disabled");
    perform(node.id, AccessAction::Click);
    ui.update();
}

pub fn focus(ui: &mut Ui, role: Role, label: &str) {
    let node = reachable(role, label);
    perform(node.id, AccessAction::Focus);
    ui.update();
}

/// The node with `role` and `label`, which must be where the user could reach it. Assistive
/// technology reaches what a modal popup covers, which neither the pointer nor the keys do.
fn reachable(role: Role, label: &str) -> AccessNode {
    let node = find(role, label).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
    assert!(
        is_over_every_modal(node.id),
        "{role:?} {label:?} is under a modal popup"
    );
    node
}

/// Whether a modal popup is open, which stays a while as it closes.
pub fn is_modal_open() -> bool {
    with_doc(|d| d.popups.iter().any(|popup| is_modal(d, popup)))
}

fn is_modal(d: &Document, popup: &NodeId) -> bool {
    d.get(*popup)
        .is_some_and(|node| node.visible && node.popup().is_some_and(|popup| popup.modal))
}

/// Whether the node that assistive technology knows as `id` is in the topmost modal popup, or in
/// a popup opened over it, or no modal popup is open.
fn is_over_every_modal(id: u64) -> bool {
    with_doc(|d| {
        let Some(top) = d.popups.iter().rposition(|popup| is_modal(d, popup)) else {
            return true;
        };
        let over = &d.popups[top..];
        d.ancestors(KeyData::from_ffi(id).into())
            .iter()
            .any(|node| over.contains(node))
    })
}

thread_local! {
    /// When the last press was, in milliseconds. Presses are far apart unless a test says.
    static PRESSED_AT: Cell<u32> = const { Cell::new(0) };
}

pub fn click(ui: &mut Ui, x: f32, y: f32) {
    click_with(ui, x, y, Modifiers::default());
}

pub fn click_with(ui: &mut Ui, x: f32, y: f32, modifiers: Modifiers) {
    press_at(ui, x, y, PointerButton::Primary, modifiers, 1000);
}

/// Clicks again where the last click was, soon enough to make a double click of it.
pub fn double_click(ui: &mut Ui, x: f32, y: f32) {
    click(ui, x, y);
    press_at(ui, x, y, PointerButton::Primary, Modifiers::default(), 100);
}

pub fn right_click(ui: &mut Ui, x: f32, y: f32) {
    press_at(
        ui,
        x,
        y,
        PointerButton::Secondary,
        Modifiers::default(),
        1000,
    );
}

fn press_at(
    ui: &mut Ui,
    x: f32,
    y: f32,
    button: PointerButton,
    modifiers: Modifiers,
    after_ms: u32,
) {
    let position = PointF::new(x, y);
    let time = PRESSED_AT.with(|at| {
        at.set(at.get() + after_ms);
        at.get()
    });
    ui.handle_event(WindowEvent::PointerMoved {
        position,
        modifiers,
    });
    for pressed in [true, false] {
        ui.handle_event(WindowEvent::PointerButton {
            position,
            button,
            pressed,
            modifiers,
            time,
        });
    }
    ui.update();
}

/// Presses the primary button at `from`, moves to `to` in steps, and lets go there.
pub fn drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32)) {
    let none = Modifiers::default();
    let at = |t: f32| PointF::new(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
    let time = PRESSED_AT.with(|pressed| {
        pressed.set(pressed.get() + 1000);
        pressed.get()
    });
    let button = |position, pressed| WindowEvent::PointerButton {
        position,
        button: PointerButton::Primary,
        pressed,
        modifiers: none,
        time,
    };
    ui.handle_event(WindowEvent::PointerMoved {
        position: at(0.0),
        modifiers: none,
    });
    ui.handle_event(button(at(0.0), true));
    ui.update();
    for step in 1..=8 {
        ui.handle_event(WindowEvent::PointerMoved {
            position: at(step as f32 / 8.0),
            modifiers: none,
        });
        ui.update();
    }
    ui.handle_event(button(at(1.0), false));
    ui.update();
}

/// Scrolls what is at (`x`, `y`) by `dy` pixels of the window, down for more than 0.
pub fn scroll(ui: &mut Ui, x: f32, y: f32, dy: f32) {
    ui.handle_event(WindowEvent::Scroll {
        position: PointF::new(x, y),
        delta_x: 0.0,
        delta_y: dy,
        modifiers: Modifiers::default(),
        source: ScrollSource::Unknown,
        phase: ScrollPhase::Single,
        time: 0,
    });
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
    /// Never dropped. The last of a client must not go on the runtime's threads, which drop the
    /// tasks that hold the core in the background once it shuts down.
    nets: Vec<net::Client>,
    _dir: tempfile::TempDir,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        // Without waiting for blocking work, which may wait on a server that never answers
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
        for net in self.nets.drain(..) {
            std::mem::forget(net);
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

/// As [`signed_in_core`], with the lyrics that `find_lyrics` finds.
pub fn signed_in_core_with_lyrics(
    server: &str,
    find_lyrics: lyrics::FindLyrics,
) -> (TestCore, hiphile_core::Hiphile) {
    let (mut core, hiphile) = core("");
    let session = hiphile_core::test_support::session(server);
    core.nets.push(session.client().net().clone());
    hiphile.open_workspace_with_lyrics(session, config::Profile::new(server, "alice"), find_lyrics);
    (core, hiphile)
}

/// Commands that are kept in the order they are sent, for the test to read.
pub fn recording_commands() -> (Commands, Rc<RefCell<Vec<Command>>>) {
    let sent = Rc::new(RefCell::new(Vec::new()));
    let commands = Commands::new({
        let sent = sent.clone();
        move |command| sent.borrow_mut().push(command)
    });
    (commands, sent)
}

/// What a screen is made with on the workspace of `hiphile`, which is signed in, sending to
/// `commands`. Made in a view, as it follows the workspace for as long as the view lives; what it
/// tells shows where the view puts `snackbars.view()`.
///
/// It keeps `hiphile` until then, as the app does: the workspace goes with the last of it, and
/// what the view keeps of the context may not be the workspace.
pub fn screen_context(hiphile: &hiphile_core::Hiphile, commands: Commands) -> ScreenContext {
    let workspace = hiphile.workspace().expect("signed in");
    let known = Known::follow(hiphile, &workspace);
    let kept = hiphile.clone();
    on_cleanup(move || drop(kept));
    ScreenContext::new(workspace, commands, known, snackbars(), Overlays::new())
}
