//! Draws the Songs screen into PNG files without opening a window, to see how it looks:
//! `cargo run -p next-songs --example shots -- <folder>`. It needs a GPU, not a display. Each shot
//! is what is right of the rail, as wide as with the rail open or, for the narrow ones, closed in
//! the narrowest window. Menus open as windows of their own, which it does not draw.

use std::path::{Path, PathBuf};

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::core::access::AccessNode;
use anti_webui::core::event::{Modifiers, WindowEvent};
use anti_webui::core::geometry::PointF;
use anti_webui::prelude::*;
use anti_webui_md3e::{color_scheme, set_color_scheme};
use hiphile_core::Hiphile;
use hiphile_core::navigation::Destination;
use hiphile_core::test_support::SongsServer;
use next_songs::songs;
use next_ui::pane::GAP;
use next_ui::testing::{
    click, click_with, find, frames, nodes, recording_commands, screen_context, signed_in_core,
    typed, wait_until,
};
use next_ui::theme::{DEFAULT_SEED, scheme};
use next_ui::type_scale::body;
use next_ui::{Screen, app_bar};
use serde_json::{Value, json};
use subsonic::Song;

const SCALE: f64 = 2.0;
/// The window's size less the open rail.
const WIDE: (u32, u32) = (1200 - 220, 760);
/// The narrowest window's size less the closed rail.
const NARROW: (u32, u32) = (900 - 96, 560);

const ARTISTS: &[&str] = &[
    "Mina Arai",
    "The Weatherhouse Quartet",
    "Odd Harbour",
    "Lumen & Vale",
    "Kaito Mori",
];

const ALBUMS: &[&str] = &[
    "Blue Hours",
    "Paper Lanterns of a Very Long Summer Evening",
    "Northern Lines",
    "Small Rooms",
    "Coastal",
    "Ferns",
    "Night Market",
];

const TITLES: &[&str] = &[
    "Afterglow",
    "Arcade Lights",
    "Before the Rain Comes Down Over the Harbour Wall",
    "Bellflower",
    "Blue Hour",
    "Breakwater",
    "Candle Smoke",
    "Cedar",
    "Driftwood (Instrumental)",
    "Early Trains",
    "Ember",
    "Fieldnotes",
    "Glass Garden",
    "Harbour Lights",
    "Hearth",
    "Kites",
    "Lanterns",
    "Low Tide",
    "Marigold",
    "Morning Static",
    "Night Bus",
    "Northern Line",
    "Paper Moon",
    "Quiet Hours",
    "Rooftops",
];

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "songs-shots".to_owned()),
    );
    std::fs::create_dir_all(&folder).expect("a folder for the shots");
    let server = SongsServer::new(library());
    server.star(&["s3", "s12", "s17"]);
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let at = |name: &str| folder.join(format!("songs-{name}-{theme}.png"));
        shot(&at("wide"), WIDE, dark, &server, playing, |ui| {
            let (_, y) = middle(&row("Breakwater"));
            ui.handle_event(WindowEvent::PointerMoved {
                position: PointF::new(600.0 * SCALE as f32, y),
                modifiers: Modifiers::default(),
            });
        });
        shot(&at("narrow"), NARROW, dark, &server, playing, |_| {});
        shot(&at("selected"), WIDE, dark, &server, skipping, |ui| {
            click_heading(ui, "Artist");
            click_heading(ui, "Artist");
            frames(ui, 400);
            click_row(ui, "Cedar", Modifiers::default());
            click_row(ui, "Low Tide", Modifiers::default().with_shift());
            click_row(ui, "Breakwater", Modifiers::PRIMARY);
        });
        shot(&at("unfocused"), WIDE, dark, &server, skipping, |ui| {
            click_row(ui, "Cedar", Modifiers::default());
            let filter = find(Role::TextInput, "Filter Songs").expect("the filter");
            let (x, y) = middle(&filter);
            click(ui, x, y);
            let (_, y) = middle(&row("Breakwater"));
            ui.handle_event(WindowEvent::PointerMoved {
                position: PointF::new(600.0 * SCALE as f32, y),
                modifiers: Modifiers::default(),
            });
        });
        shot(&at("filtered"), NARROW, dark, &server, skipping, |ui| {
            let filter = find(Role::TextInput, "Filter Songs").expect("the filter");
            let (x, y) = middle(&filter);
            click(ui, x, y);
            typed(ui, "light");
        });
    }
    let failed = SongsServer::new(library());
    failed.refuse();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let path = folder.join(format!("songs-failed-{theme}.png"));
        shot(&path, WIDE, dark, &failed, |_| {}, |_| {});
    }
}

fn library() -> Vec<Value> {
    TITLES
        .iter()
        .enumerate()
        .map(|(ix, title)| {
            let (artist, album) = (ix % ARTISTS.len(), ix % ALBUMS.len());
            json!({
                "id": format!("s{ix}"),
                "title": title,
                "duration": 140 + (ix * 37) % 300,
                "artist": ARTISTS[artist],
                "artistId": format!("artist-{artist}"),
                "album": ALBUMS[album],
                "albumId": format!("album-{album}"),
            })
        })
        .collect()
}

fn song(ix: usize) -> Song {
    serde_json::from_value(library()[ix].clone()).expect("a song")
}

/// Blue Hour playing.
fn playing(hiphile: &Hiphile) {
    let player = hiphile.workspace().expect("signed in").player().clone();
    player.fill_queue(&[song(4)], 0);
    player.pretend_playing(true);
}

/// Instrumentals skipped and Blue Hour paused.
fn skipping(hiphile: &Hiphile) {
    hiphile.settings().queue().set_skip_instrumentals(true);
    let player = hiphile.workspace().expect("signed in").player().clone();
    player.fill_queue(&[song(4)], 0);
    player.pretend_playing(false);
}

/// Where the middle of `node` is in the window's pixels.
fn middle(node: &AccessNode) -> (f32, f32) {
    let (b, scale) = (node.bounds, SCALE as f32);
    (
        (b.x + b.width / 2.0) * scale,
        (b.y + b.height / 2.0) * scale,
    )
}

fn click_heading(ui: &mut Ui, label: &str) {
    let heading = nodes(Role::Button)
        .into_iter()
        .find(|button| {
            button
                .label
                .as_deref()
                .is_some_and(|name| name == label || name.starts_with(&format!("{label}, ")))
        })
        .expect("the heading");
    let (x, y) = middle(&heading);
    click(ui, x, y);
    frames(ui, 100);
}

fn row(title: &str) -> AccessNode {
    nodes(Role::ListOption)
        .into_iter()
        .find(|row| {
            row.label
                .as_deref()
                .is_some_and(|name| name.starts_with(title))
        })
        .expect("the row")
}

fn click_row(ui: &mut Ui, title: &str, modifiers: Modifiers) {
    let row = row(title);
    let (_, y) = middle(&row);
    click_with(ui, (row.bounds.x + 40.0) * SCALE as f32, y, modifiers);
    frames(ui, 100);
}

/// What is right of the rail, for the songs of `server` once `setup` has done what it does,
/// and once `drive` has done what it does to the screen.
fn shot(
    path: &Path,
    (width, height): (u32, u32),
    dark: bool,
    server: &SongsServer,
    setup: impl FnOnce(&Hiphile),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = signed_in_core(server.url());
    hiphile
        .workspace()
        .expect("signed in")
        .showing(Destination::Songs);
    setup(&hiphile);
    let (w, h) = (
        (f64::from(width) * SCALE) as u32,
        (f64::from(height) * SCALE) as u32,
    );
    let pixels = headless::render_driven_rgba(
        w,
        h,
        SCALE,
        move || {
            set_color_scheme(scheme(DEFAULT_SEED, dark));
            let (commands, _) = recording_commands();
            window(songs(&screen_context(&hiphile, commands)))
        },
        |ui| {
            wait_until(ui, || {
                find(Role::ListBox, "Songs").is_some() || find(Role::Button, "Retry").is_some()
            });
            frames(ui, 100);
            drive(ui);
            frames(ui, 900);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, w, h, &pixels).expect("the shot written");
    println!("{}", path.display());
}

/// The bar, and the screen's panes below it.
fn window(screen: Screen) -> Div {
    let Screen {
        title,
        actions,
        body: panes,
        ..
    } = screen;
    body(div())
        .size_full()
        .flex()
        .flex_col()
        .dyn_style(|s| {
            let c = color_scheme();
            s.bg(c.surface).text_color(c.on_surface)
        })
        .child(app_bar(Vec::new(), title, actions))
        .child(
            div()
                .flex_1()
                .min_h(0)
                .flex()
                .flex_col()
                .ml(GAP)
                .mr(GAP)
                .mb(GAP)
                .child(panes.flex_1().min_h(0)),
        )
}
