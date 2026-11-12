//! Draws the artists into PNG files without opening a window, to see how they look:
//! `cargo run -p next-artists --example shots -- <folder>`. It needs a GPU, not a display. Each
//! shot is what is right of the rail: as wide as with the rail open in the window as it opens, and
//! for the narrow ones, closed in the narrowest window. Menus open as windows of their own, which
//! it does not draw.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::core::event::NamedKey;
use anti_webui::prelude::*;
use anti_webui_md3e::{color_scheme, set_color_scheme};
use hiphile_core::navigation::Location;
use hiphile_core::test_support::ArtistsServer;
use next_artists::artists;
use next_ui::cover::install_covers;
use next_ui::list_detail::Split;
use next_ui::pane::GAP;
use next_ui::testing::{focus, frames, key, press, screen_context, scroll, signed_in_core};
use next_ui::theme::{DEFAULT_SEED, cover_scheme};
use next_ui::type_scale::body;
use next_ui::{Command, Commands, Screen, app_bar};
use serde_json::{Value, json};
use subsonic::ArtistId;

const SCALE: f64 = 2.0;
/// The window as it opens, less the open rail.
const WIDE: (u32, u32) = (1200 - 220, 760);
/// The narrowest window, less the closed rail.
const NARROW: (u32, u32) = (900 - 96, 560);
const COVERS: u32 = 12;

/// What a shot does once the screen has what the server sends.
#[derive(Clone, Copy)]
enum Then {
    Nothing,
    /// Onto the page from the list, which selects its first song.
    OpenPage,
    ScrollPage(f32),
    SeeMore,
}

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "artists-shots".to_owned()),
    );
    std::fs::create_dir_all(folder.join("covers")).expect("a folder for the shots");
    let covers: Arc<[Vec<u8>]> = (0..COVERS).map(|ix| cover(&folder, ix)).collect();
    let full = server(&covers);
    let failing = server(&covers);
    failing.refuse("getArtists");
    let sparse = server(&covers);
    sparse.refuse("getTopSongs");
    sparse.set_similar("ar2", Vec::new());
    let kaito = || Location::Artists(Some(ArtistId::new("ar2")));
    let odd = || Location::Artists(Some(ArtistId::new("ar6")));
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let at = |name: &str| folder.join(format!("artists-{name}-{theme}.png"));
        shot(&at("page"), WIDE, dark, &full, kaito(), Then::Nothing);
        shot(
            &at("page-narrow"),
            NARROW,
            dark,
            &full,
            kaito(),
            Then::Nothing,
        );
        shot(&at("selected"), WIDE, dark, &full, kaito(), Then::OpenPage);
        shot(
            &at("albums"),
            WIDE,
            dark,
            &full,
            kaito(),
            Then::ScrollPage(520.0),
        );
        shot(
            &at("albums-narrow"),
            NARROW,
            dark,
            &full,
            kaito(),
            Then::ScrollPage(900.0),
        );
        shot(
            &at("similar"),
            WIDE,
            dark,
            &full,
            kaito(),
            Then::ScrollPage(4000.0),
        );
        shot(&at("see-more"), WIDE, dark, &full, kaito(), Then::SeeMore);
        shot(&at("no-albums"), WIDE, dark, &full, odd(), Then::Nothing);
        shot(
            &at("sections-failed"),
            WIDE,
            dark,
            &sparse,
            kaito(),
            Then::Nothing,
        );
        shot(
            &at("failed"),
            WIDE,
            dark,
            &failing,
            Location::Artists(None),
            Then::Nothing,
        );
    }
}

fn shot(
    path: &Path,
    (width, height): (u32, u32),
    dark: bool,
    server: &ArtistsServer,
    at: Location,
    then: Then,
) {
    let (_core, hiphile) = signed_in_core(server.url());
    install_covers(hiphile.clone());
    let player = hiphile.workspace().expect("signed in").player().clone();
    let playing: subsonic::Song = serde_json::from_value(top_songs()[1].clone()).unwrap();
    player.fill_queue(&[playing], 0);
    player.pretend_playing(true);
    let (w, h) = (
        (width as f64 * SCALE) as u32,
        (height as f64 * SCALE) as u32,
    );
    let pixels = headless::render_driven_rgba(
        w,
        h,
        SCALE,
        move || {
            set_color_scheme(cover_scheme(DEFAULT_SEED, dark));
            let location = signal(at);
            let commands = Commands::new(move |command| {
                if let Command::Show(to) | Command::ShowInPlace(to) = command {
                    location.set_if_changed(to);
                }
            });
            let cx = screen_context(&hiphile, commands);
            let Screen {
                title,
                actions,
                body: pane,
                ..
            } = artists(location, Split::new(), &cx);
            window(title, actions, pane)
        },
        move |ui| {
            settle(ui);
            match then {
                Then::Nothing => {}
                Then::OpenPage => {
                    focus(ui, Role::ListBox, "Artists");
                    key(ui, NamedKey::ArrowRight);
                    key(ui, NamedKey::ArrowDown);
                }
                Then::ScrollPage(dy) => {
                    // Where the page is, in the pixels the events here are in
                    let at = |side: u32| (side as f64 * 0.75 * SCALE) as f32;
                    scroll(ui, at(width), at(height), dy);
                }
                Then::SeeMore => press(ui, Role::Button, "See more"),
            }
            settle(ui);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, w, h, &pixels).expect("the shot written");
    println!("{}", path.display());
}

/// Lets what the server sends and the covers come, and what moves get where it goes.
fn settle(ui: &mut Ui) {
    for _ in 0..80 {
        ui.poll_images();
        frames(ui, 16);
        std::thread::sleep(Duration::from_millis(10));
    }
    frames(ui, 600);
}

/// What is right of the rail: the bar, and the body on its pane.
fn window(title: String, actions: Vec<Div>, pane: Div) -> Div {
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
                .mr(GAP)
                .mb(GAP)
                .child(pane.flex_1().min_h(0)),
        )
}

const ARTISTS: [(&str, u32); 9] = [
    ("Lumen & Vale", 3),
    ("Mina Arai", 4),
    ("Kaito Mori", 7),
    ("The Weatherhouse Quartet and the Long Evening Orchestra", 2),
    ("Mira Okada", 1),
    ("Night Ferry", 2),
    ("Odd Harbour", 0),
    ("Paper Moons", 5),
    ("Sora Hasegawa", 3),
];

fn artist(ix: usize) -> Value {
    let (name, albums) = ARTISTS[ix];
    json!({
        "id": format!("ar{ix}"),
        "name": name,
        "albumCount": albums,
        "coverArt": format!("c{}", ix % COVERS as usize),
    })
}

fn albums() -> Vec<Value> {
    [
        ("Night Market", 2023),
        ("Last Train Home", 2021),
        ("Low Tide", 2019),
        ("Driftwood and Other Small Songs for the Morning", 2017),
        ("Harbour Lights", 2015),
        ("First Snow", 2013),
        ("Kaito Mori", 2010),
    ]
    .iter()
    .enumerate()
    .map(|(ix, (name, year))| {
        json!({
            "id": format!("al{ix}"),
            "name": name,
            "artist": "Kaito Mori",
            "artistId": "ar2",
            "year": year,
            "songCount": 10,
            "coverArt": format!("c{}", (ix + 3) % COVERS as usize),
        })
    })
    .collect()
}

fn top_songs() -> Vec<Value> {
    [
        ("Last Train", "Kaito Mori", "Last Train Home", 301),
        ("Night Market", "Kaito Mori", "Night Market", 276),
        (
            "Paper Lanterns",
            "Kaito Mori feat. Mina Arai",
            "Night Market",
            312,
        ),
        ("Driftwood", "Kaito Mori", "Low Tide", 188),
        (
            "A Very Long Title for a Song That Goes On Well Past the Edge of Its Row",
            "Kaito Mori",
            "Harbour Lights",
            198,
        ),
        ("Window Seat", "Kaito Mori", "First Snow", 229),
        ("Afterglow", "Kaito Mori", "Low Tide", 266),
        ("Morning Static", "Kaito Mori", "Kaito Mori", 243),
    ]
    .iter()
    .enumerate()
    .map(|(ix, (title, artist, album, seconds))| {
        let artist_id = if *artist == "Kaito Mori" {
            "ar2"
        } else {
            "ar-feat"
        };
        json!({
            "id": format!("s{ix}"),
            "title": title,
            "artist": artist,
            "artistId": artist_id,
            "album": album,
            "albumId": format!("al{}", ix % 7),
            "coverArt": format!("c{}", (ix % 7 + 3) % COVERS as usize),
            "duration": seconds,
        })
    })
    .collect()
}

fn server(covers: &Arc<[Vec<u8>]>) -> ArtistsServer {
    let server = ArtistsServer::new();
    server.set_artists((0..ARTISTS.len()).map(artist).collect());
    for ix in 0..ARTISTS.len() {
        server.set_albums(&format!("ar{ix}"), Vec::new());
    }
    server.set_albums("ar2", albums());
    server.set_top_songs("Kaito Mori", top_songs());
    server.set_similar("ar2", [1, 4, 0, 7, 8].map(artist).to_vec());
    let covers = covers.clone();
    server.set_covers(move |id| {
        let ix = id.trim_start_matches('c').parse::<usize>().unwrap_or(0);
        covers[ix % covers.len()].clone()
    });
    server
}

/// A square that runs from one hue to the next, as a stand-in for a cover, written to `folder`.
fn cover(folder: &Path, ix: u32) -> Vec<u8> {
    const SIDE: u32 = 128;
    let hue = ix as f32 / COVERS as f32;
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let t = (x + y) as f32 / (SIDE * 2) as f32;
            let [r, g, b] = hsv(hue + t * 0.15, 0.55, 0.9 - t * 0.35);
            rgba.extend([r, g, b, 255]);
        }
    }
    let path = folder.join("covers").join(format!("{ix}.png"));
    headless::write_png(&path, SIDE, SIDE, &rgba).unwrap();
    std::fs::read(path).unwrap()
}

fn hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.fract() * 6.0;
    let f = h.fract();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match h as u32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r, g, b].map(|c| (c * 255.0) as u8)
}
