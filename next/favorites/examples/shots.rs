//! Draws the favorites into PNG files without opening a window, to see how they look:
//! `cargo run -p next-favorites --example shots -- <folder>`. It needs a GPU, not a display. Each
//! shot is what is right of the rail: as wide as with the rail open in the window as it opens, and
//! for the narrow ones, closed in the narrowest window. Menus open as windows of their own, which
//! it does not draw.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::prelude::*;
use anti_webui_md3e::{color_scheme, set_color_scheme};
use next_favorites::favorites;
use next_ui::cover::install_covers;
use next_ui::pane::GAP;
use next_ui::testing::{frames, press, recording_commands, screen_context, signed_in_core};
use next_ui::theme::{DEFAULT_SEED, cover_scheme};
use next_ui::type_scale::body;
use next_ui::{Screen, app_bar};
use serde_json::{Value, json};

const SCALE: f64 = 2.0;
/// The window as it opens, less the open rail.
const WIDE: (u32, u32) = (1200 - 220, 760);
/// The narrowest window, less the closed rail.
const NARROW: (u32, u32) = (900 - 96, 560);
const COVERS: u32 = 8;

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "favorites-shots".to_owned()),
    );
    std::fs::create_dir_all(folder.join("covers")).expect("a folder for the shots");
    let covers: Arc<[Vec<u8>]> = (0..COVERS).map(|ix| cover(&folder, ix)).collect();
    let full = Server::new(starred(), false, covers.clone());
    let none = Server::new(json!({}), false, covers.clone());
    let failing = Server::new(json!({}), true, covers);
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let at = |name: &str| folder.join(format!("favorites-{name}-{theme}.png"));
        shot(&at("songs"), WIDE, dark, &full, None);
        shot(&at("songs-narrow"), NARROW, dark, &full, None);
        shot(&at("albums"), WIDE, dark, &full, Some("Albums"));
        shot(&at("artists-narrow"), NARROW, dark, &full, Some("Artists"));
        shot(&at("empty"), WIDE, dark, &none, None);
        shot(&at("failed"), WIDE, dark, &failing, None);
    }
}

fn shot(path: &Path, (width, height): (u32, u32), dark: bool, server: &Server, tab: Option<&str>) {
    let (_core, hiphile) = signed_in_core(&server.url);
    install_covers(hiphile.clone());
    let player = hiphile.workspace().expect("signed in").player().clone();
    let playing: subsonic::Song = serde_json::from_value(songs()[1].clone()).unwrap();
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
            let (commands, _) = recording_commands();
            let cx = screen_context(&hiphile, commands);
            let Screen {
                title, body: pane, ..
            } = favorites(&cx);
            window(title, pane)
        },
        |ui| {
            settle(ui);
            if let Some(tab) = tab {
                press(ui, Role::Tab, tab);
            }
            settle(ui);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, w, h, &pixels).expect("the shot written");
    println!("{}", path.display());
}

/// Lets what the server sends and the covers come, and the tabs' indicator get where it goes.
fn settle(ui: &mut Ui) {
    for _ in 0..60 {
        ui.poll_images();
        frames(ui, 16);
        std::thread::sleep(Duration::from_millis(10));
    }
    frames(ui, 600);
}

/// What is right of the rail: the bar, and the body on its pane.
fn window(title: String, pane: Div) -> Div {
    body(div())
        .size_full()
        .flex()
        .flex_col()
        .dyn_style(|s| {
            let c = color_scheme();
            s.bg(c.surface).text_color(c.on_surface)
        })
        .child(app_bar(Vec::new(), title, Vec::new()))
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

fn songs() -> Vec<Value> {
    [
        ("Glass Harbor", "Mira Okada", "Low Tide Lights", 245),
        (
            "Paper Lanterns",
            "The Weatherhouse Quartet",
            "Summer Evenings",
            312,
        ),
        (
            "A Very Long Title for a Song That Goes On Well Past the Edge of Its Row",
            "Lumen & Vale",
            "Second Light",
            198,
        ),
        ("Night Market", "Kaito Mori", "Night Market", 276),
        ("Ferns", "Odd Harbour", "Ferns", 231),
        ("Hearth", "Mina Arai", "Small Rooms", 254),
        ("Valleys", "Lumen & Vale", "Valleys", 205),
        ("Driftwood", "Kaito Mori", "Low Tide", 188),
        ("Morning Static", "Odd Harbour", "Morning Static", 243),
        ("Window Seat", "Mina Arai", "Blue Hours", 229),
        ("Last Train", "Kaito Mori", "Night Market", 301),
        ("Afterglow", "Lumen & Vale", "Coastal", 266),
    ]
    .iter()
    .enumerate()
    .map(|(ix, (title, artist, album, seconds))| {
        json!({
            "id": format!("s{ix}"),
            "title": title,
            "artist": artist,
            "artistId": format!("ar-{artist}"),
            "album": album,
            "albumId": format!("al-{album}"),
            "coverArt": format!("c{}", ix % 8),
            "duration": seconds,
        })
    })
    .collect()
}

fn starred() -> Value {
    let albums: Vec<Value> = [
        ("Blue Hours", "Mina Arai", 2021),
        ("Night Market", "Kaito Mori", 2019),
        ("Coastal", "Lumen & Vale", 2023),
        ("Ferns", "Odd Harbour", 2018),
    ]
    .iter()
    .enumerate()
    .map(|(ix, (name, artist, year))| {
        json!({
            "id": format!("al{ix}"),
            "name": name,
            "artist": artist,
            "artistId": format!("ar-{artist}"),
            "year": year,
            "coverArt": format!("c{}", ix + 2),
        })
    })
    .collect();
    let artists: Vec<Value> = [("Mina Arai", 4), ("Kaito Mori", 3), ("Odd Harbour", 0)]
        .iter()
        .enumerate()
        .map(|(ix, (name, albums))| {
            json!({
                "id": format!("ar{ix}"),
                "name": name,
                "albumCount": albums,
                "coverArt": format!("c{}", ix + 5),
            })
        })
        .collect();
    json!({ "song": songs(), "album": albums, "artist": artists })
}

/// Serves `starred` as the favorites, or refuses to, and a cover of its own hue for each id.
struct Server {
    url: String,
}

impl Server {
    fn new(starred: Value, refuses: bool, covers: Arc<[Vec<u8>]>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let starred = Arc::new(starred);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let (starred, covers) = (starred.clone(), covers.clone());
                std::thread::spawn(move || answer(&mut stream, &starred, refuses, &covers));
            }
        });
        Self { url }
    }
}

fn answer(stream: &mut std::net::TcpStream, starred: &Value, refuses: bool, covers: &[Vec<u8>]) {
    let mut head = [0; 4096];
    let Ok(read) = stream.read(&mut head) else {
        return;
    };
    let head = String::from_utf8_lossy(&head[..read]).into_owned();
    let target = head.split(' ').nth(1).unwrap_or_default();
    if target.contains("/getCoverArt") {
        let id = target
            .split(['?', '&'])
            .find_map(|param| param.strip_prefix("id="))
            .unwrap_or_default();
        let ix = id.trim_start_matches('c').parse::<usize>().unwrap_or(0);
        let png = &covers[ix % covers.len()];
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            png.len()
        );
        let _ = stream.write_all(png);
        return;
    }
    let mut body = json!({ "status": "ok", "version": "1.16.1" });
    if target.contains("/getStarred2") {
        if refuses {
            body["status"] = json!("failed");
            body["error"] = json!({ "code": 0, "message": "The server is down for maintenance." });
        } else {
            body["starred2"] = starred.clone();
        }
    }
    let body = json!({ "subsonic-response": body }).to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
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
