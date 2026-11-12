//! Draws Home on a server of made-up albums into PNG files without opening a window:
//! `cargo run -p next-hiphile --example home -- <folder>`. It needs a GPU, not a display. Menus
//! open as windows of their own, which it does not draw.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::core::event::NamedKey;
use anti_webui::prelude::{Role, signal};
use hiphile_core::test_support::HomeServer;
use next_hiphile::{MenuBar, Reach, SMALLEST_WINDOW, WINDOW, app};
use next_player::testing::png;
use next_ui::testing::{find, frames, key, nodes, signed_in_core};

const SCALE: f64 = 2.0;
const COVER: u32 = 256;

const WORDS: [&str; 24] = [
    "Amber", "Paper", "Northern", "Salt", "Velvet", "Hollow", "Silver", "Quiet", "Copper", "Late",
    "Distant", "Glass", "Static", "Cedar", "Neon", "Winter", "Open", "Low", "Second", "Golden",
    "Hidden", "Slow", "Paris", "Signal",
];
const THINGS: [&str; 17] = [
    "Lanterns", "Bloom", "Fields", "Rooms", "Hours", "Weather", "Lights", "Engines", "Shore",
    "Letters", "Tapes", "Gardens", "Motion", "Sundays", "Radio", "Rivers", "Harbor",
];
const FIRST: [&str; 13] = [
    "Mira", "Theo", "Ines", "Otto", "Clara", "Jun", "Noor", "Felix", "Hana", "Lev", "Ada", "Rui",
    "Sol",
];
const LAST: [&str; 11] = [
    "Okada", "Lind", "Valdés", "Brand", "Wynn", "Sato", "Haddad", "Moreau", "Kerr", "Novak",
    "Ferreira",
];

/// The songs played last, the latest first, on albums of their own.
const PLAYED: [(&str, &str, &[&str]); 3] = [
    (
        "Low Tide Lights",
        "Mira Okada",
        &["Glass Harbor", "Undertow", "Night Ferry"],
    ),
    (
        "Kitchen Radio",
        "Theo Lind and the Late Shift",
        &["Morning Static", "Two Spoons", "Weather Report (Reprise)"],
    ),
    (
        "Field Recordings, Vol. 2: Rain on a Tin Roof at the Edge of Town",
        "Ines Valdés",
        &["Rain, First Hour", "Tin Roof"],
    ),
];

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "shots".to_owned()),
    );
    std::fs::create_dir_all(&folder).expect("a folder for the shots");
    let server = server();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let at = |name: &str| folder.join(format!("home-{name}-{theme}.png"));
        shot(&at("wide"), &server, dark, WINDOW, |_| {});
        shot(&at("narrow"), &server, dark, SMALLEST_WINDOW, |_| {});
        shot(&at("selected"), &server, dark, WINDOW, |ui| {
            key(ui, NamedKey::ArrowDown);
            key(ui, NamedKey::ArrowRight);
        });
        shot(&at("songs"), &server, dark, WINDOW, |ui| {
            for _ in 0..3 {
                key(ui, NamedKey::ArrowDown);
            }
        });
        shot(&at("end"), &server, dark, SMALLEST_WINDOW, |ui| {
            key(ui, NamedKey::End);
        });
        let failing = HomeServer::new();
        for what in ["newest", "recent", "frequent", "random"] {
            failing.refuse(what);
        }
        shot(&at("failed"), &failing, dark, WINDOW, |_| {});
        let waiting = HomeServer::new();
        for what in ["newest", "recent", "frequent", "random"] {
            waiting.hold(what);
        }
        shot(&at("loading"), &waiting, dark, WINDOW, |_| {});
    }
}

fn server() -> HomeServer {
    let server = HomeServer::new();
    let mut names = (0..).map(|ix: usize| {
        let name = format!(
            "{} {}",
            WORDS[ix % WORDS.len()],
            THINGS[(ix * 7) % THINGS.len()]
        );
        let artist = format!(
            "{} {}",
            FIRST[(ix * 5) % FIRST.len()],
            LAST[(ix * 3) % LAST.len()]
        );
        (name, artist)
    });
    let mut albums = |count: usize, prefix: &str| -> Vec<String> {
        (0..count)
            .map(|ix| {
                let id = format!("{prefix}{ix}");
                let (name, artist) = names.next().unwrap();
                server.add_album(&id, &name, &artist);
                id
            })
            .collect()
    };
    let recommended = albums(12, "r");
    let most_played = albums(12, "m");
    let newest = albums(24, "n");
    let list = |name: &str, ids: &[String]| {
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        server.set_list(name, &ids);
    };
    list("random", &recommended);
    list("frequent", &most_played);
    list("newest", &newest);

    let mut minute = 59;
    let mut played = Vec::new();
    for (ix, (album, artist, songs)) in PLAYED.iter().enumerate() {
        let id = format!("p{ix}");
        server.add_album(&id, album, artist);
        for (n, title) in songs.iter().enumerate() {
            let at = format!("2026-11-12T09:{minute:02}:00Z");
            server.add_song(&id, &format!("{id}-{n}"), title, Some(&at));
            minute -= 3;
        }
        played.push(id);
    }
    list("recent", &played);

    for (ix, id) in recommended
        .iter()
        .chain(&most_played)
        .chain(&newest)
        .chain(&played)
        .enumerate()
    {
        server.set_cover_of(id, cover(ix));
    }
    server
}

/// A cover of two colors that turn with `ix` around the wheel, and a disc of a third.
fn cover(ix: usize) -> Vec<u8> {
    let hue = (ix as f32 * 0.137).fract();
    let top = rgb(hue, 0.55, 0.85);
    let bottom = rgb((hue + 0.12).fract(), 0.7, 0.35);
    let disc = rgb((hue + 0.5).fract(), 0.35, 0.95);
    let (cx, cy) = (0.3 + 0.4 * ((ix * 3 % 5) as f32 / 4.0), 0.4);
    png(COVER, move |x, y| {
        let (fx, fy) = (x as f32 / COVER as f32, y as f32 / COVER as f32);
        if ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() < 0.16 {
            return [disc[0], disc[1], disc[2], 255];
        }
        let t = (fx + fy) / 2.0;
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        [
            mix(top[0], bottom[0]),
            mix(top[1], bottom[1]),
            mix(top[2], bottom[2]),
            255,
        ]
    })
}

fn rgb(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let h = hue * 6.0;
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    [r, g, b].map(|channel| ((channel + m) * 255.0) as u8)
}

fn shot(
    path: &Path,
    server: &HomeServer,
    dark: bool,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = signed_in_core(server.url());
    let (width, height) = (
        (size.0 as f64 * SCALE) as u32,
        (size.1 as f64 * SCALE) as u32,
    );
    let pixels = headless::render_driven_rgba(
        width,
        height,
        SCALE,
        move || app(hiphile, signal(dark), MenuBar::new(), Reach::Window),
        |ui| {
            settle(ui);
            drive(ui);
            frames(ui, 900);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, width, height, &pixels).expect("the shot written");
    println!("{}", path.display());
}

/// Until the shelves have come or failed, and their covers with them, or a while has passed.
fn settle(ui: &mut Ui) {
    frames(ui, 100);
    for _ in 0..400 {
        ui.poll_images();
        frames(ui, 16);
        let loading = nodes(Role::ProgressBar).iter().any(|bar| {
            bar.label
                .as_deref()
                .is_some_and(|l| l.starts_with("Loading"))
        });
        if !loading && find(Role::Heading, "New albums").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..60 {
        ui.poll_images();
        frames(ui, 16);
        std::thread::sleep(Duration::from_millis(5));
    }
}
