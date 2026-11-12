//! Draws the window into PNG files without opening one, to see how it looks:
//! `cargo run -p next-hiphile --example shots -- <folder>`. It needs a GPU, not a display. Menus
//! open as windows of their own, which it does not draw.

use std::path::{Path, PathBuf};

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, perform};
use anti_webui::core::event::{Modifiers, NamedKey, WindowEvent};
use anti_webui::core::geometry::PointF;
use anti_webui::prelude::{Role, signal};
use anti_webui_md3e::color_scheme;
use hiphile_core::Hiphile;
use next_hiphile::{MenuBar, Reach, SMALLEST_WINDOW, WINDOW, app};
use next_player::testing::{CoverServer, png};
use next_ui::testing::{
    NOWHERE, SavedLibrary, click, core, find, frames, has_text, key, press, signed_in_core,
    signed_in_core_with_library, typed, wait_until,
};
use next_ui::theme::{DEFAULT_SEED, scheme};
use serde_json::json;
use subsonic::Song;

const SCALE: f64 = 2.0;

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "shots".to_owned()),
    );
    std::fs::create_dir_all(&folder).expect("a folder for the shots");
    let covers = CoverServer::new(png(256, cover_pixel));
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        shot(
            &folder.join(format!("sign-in-{theme}.png")),
            dark,
            false,
            |_| {},
        );
        shot(
            &folder.join(format!("home-{theme}.png")),
            dark,
            true,
            |_| {},
        );
        shot(
            &folder.join(format!("albums-narrow-{theme}.png")),
            dark,
            true,
            |ui| {
                press(ui, Role::Button, "Close Navigation");
                press(ui, Role::Button, "Albums");
            },
        );
        shot(
            &folder.join(format!("add-server-{theme}.png")),
            dark,
            true,
            |ui| {
                press(ui, Role::Button, "Servers");
                press(ui, Role::MenuItem, "Add Server…");
            },
        );
        for page in SETTINGS_PAGES {
            shot(
                &folder.join(format!("settings-{}-{theme}.png", page.to_lowercase())),
                dark,
                true,
                |ui| settings(ui, page),
            );
        }
        for resampler in ["r8brain", "soxr"] {
            shot(
                &folder.join(format!("settings-{resampler}-{theme}.png")),
                dark,
                true,
                |ui| {
                    settings(ui, "Audio");
                    press(ui, Role::Radio, resampler);
                    frames(ui, 100);
                    let group = find(Role::Heading, &format!("{resampler} filter"))
                        .expect("the resampler's settings");
                    perform(group.id, AccessAction::ScrollIntoView);
                },
            );
        }
        shot(
            &folder.join(format!("settings-r8brain-performance-{theme}.png")),
            dark,
            true,
            |ui| {
                settings(ui, "Audio");
                press(ui, Role::Radio, "r8brain");
                frames(ui, 100);
                let last = find(Role::TextInput, "r8brain filter bank cache").expect("the field");
                perform(last.id, AccessAction::ScrollIntoView);
            },
        );
        shot(
            &folder.join(format!("settings-crossfeed-{theme}.png")),
            dark,
            true,
            |ui| {
                settings(ui, "DSP");
                press(ui, Role::Switch, "Crossfeed");
            },
        );
        shot_at(
            &folder.join(format!("settings-narrow-{theme}.png")),
            dark,
            true,
            SMALLEST_WINDOW,
            |ui| settings(ui, "Audio"),
        );
        search_shots(&folder, theme, dark, &covers);
        playing_shot(
            &folder.join(format!("player-playing-{theme}.png")),
            dark,
            &covers,
            |player| player.pretend_playing(true),
            |ui| {
                seek_steps(ui, 7);
                point_at(ui, 600.0, 300.0);
            },
        );
        playing_shot(
            &folder.join(format!("player-paused-{theme}.png")),
            dark,
            &covers,
            |player| player.pretend_playing(false),
            |ui| {
                seek_steps(ui, 7);
                let line = find(Role::Slider, "Position").expect("the seek bar");
                let (x, y) = (
                    line.bounds.x + line.bounds.width / 2.0,
                    line.bounds.y + line.bounds.height / 2.0,
                );
                point_at(ui, x, y);
            },
        );
        playing_shot(
            &folder.join(format!("player-error-{theme}.png")),
            dark,
            &covers,
            |player| player.set_error("The file couldn't be read. Press Play to try again."),
            |_| {},
        );
    }
}

const SETTINGS_PAGES: [&str; 7] = [
    "Audio",
    "DSP",
    "Queue",
    "Lyrics",
    "Storage",
    "Discord",
    "Appearance",
];

fn settings(ui: &mut Ui, page: &str) {
    press(ui, Role::Button, "Settings");
    frames(ui, 400);
    press(ui, Role::Tab, page);
}

fn shot(path: &Path, dark: bool, signed_in: bool, drive: impl FnOnce(&mut Ui)) {
    shot_at(path, dark, signed_in, WINDOW, drive);
}

fn shot_at(
    path: &Path,
    dark: bool,
    signed_in: bool,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = if signed_in {
        signed_in_core(NOWHERE)
    } else {
        core("")
    };
    render(path, dark, hiphile, size, drive);
}

fn search_shots(folder: &Path, theme: &str, dark: bool, covers: &CoverServer) {
    for (name, size) in [("search", WINDOW), ("search-narrow", SMALLEST_WINDOW)] {
        search_shot(
            &folder.join(format!("{name}-{theme}.png")),
            dark,
            covers,
            size,
            |ui| {
                typed(ui, "blue");
                wait_until(ui, || find(Role::ListOption, "Blue in Green").is_some());
                key(ui, NamedKey::ArrowDown);
                key(ui, NamedKey::ArrowDown);
            },
        );
    }
    search_shot(
        &folder.join(format!("search-opened-{theme}.png")),
        dark,
        covers,
        WINDOW,
        |_| {},
    );
    search_shot(
        &folder.join(format!("search-no-matches-{theme}.png")),
        dark,
        covers,
        WINDOW,
        |ui| {
            typed(ui, "xylophone");
            wait_until(ui, || has_text("Nothing matches."));
        },
    );
    shot(
        &folder.join(format!("search-no-library-{theme}.png")),
        dark,
        true,
        |ui| press(ui, Role::Button, "Search"),
    );
}

/// The search open over Home, on a saved library whose covers come from `covers`, after `drive`.
fn search_shot(
    path: &Path,
    dark: bool,
    covers: &CoverServer,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile, _catalog) = signed_in_core_with_library(covers.url(), &library());
    render(path, dark, hiphile, size, move |ui| {
        press(ui, Role::Button, "Search");
        // For the bar to open to its full size
        frames(ui, 600);
        drive(ui);
        // For the covers to come
        for _ in 0..50 {
            frames(ui, 16);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
}

/// A song with a cover from `covers` in the queue, in the state that `setup` puts the player in.
/// The shot waits for the colors that the cover gives.
fn playing_shot(
    path: &Path,
    dark: bool,
    covers: &CoverServer,
    setup: impl FnOnce(&playback::Player),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = signed_in_core(covers.url());
    let player = hiphile.workspace().expect("signed in").player().clone();
    player.fill_queue(&[song()], 0);
    setup(&player);
    let default = scheme(DEFAULT_SEED, dark).primary;
    render(path, dark, hiphile, WINDOW, move |ui| {
        // The scheme cross-fades, which takes frames
        for _ in 0..500 {
            if color_scheme().primary != default {
                break;
            }
            frames(ui, 16);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_ne!(color_scheme().primary, default, "no colors from the cover");
        frames(ui, 900);
        drive(ui);
    });
}

fn render(
    path: &Path,
    dark: bool,
    hiphile: Hiphile,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
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
            frames(ui, 100);
            drive(ui);
            frames(ui, 900);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, width, height, &pixels).expect("the shot written");
    println!("{}", path.display());
}

fn library() -> SavedLibrary {
    let album = |id: &str, name: &str, artist: &str, artist_id: &str| {
        json!({ "id": id, "name": name, "artist": artist, "artistId": artist_id,
                "coverArt": format!("al-{id}") })
    };
    let song =
        |id: &str, title: &str, artist: &str, artist_id: &str, album: &str, album_id: &str| {
            json!({ "id": id, "title": title, "artist": artist, "artistId": artist_id,
                "album": album, "albumId": album_id, "coverArt": format!("al-{album_id}") })
        };
    SavedLibrary {
        albums: serde_json::from_value(json!([
            album("kind", "Kind of Blue", "Miles Davis", "miles"),
            album("train", "Blue Train", "John Coltrane", "john"),
            album("hours", "Blue Hours", "Mina Arai", "mina"),
            album("tide", "Low Tide Lights", "Mira Okada", "mira"),
        ]))
        .unwrap(),
        songs: serde_json::from_value(json!([
            song(
                "green",
                "Blue in Green",
                "Miles Davis",
                "miles",
                "Kind of Blue",
                "kind"
            ),
            song(
                "train",
                "Blue Train",
                "John Coltrane",
                "john",
                "Blue Train",
                "train"
            ),
            song(
                "hours",
                "Blue Hours",
                "Mina Arai",
                "mina",
                "Blue Hours",
                "hours"
            ),
            song(
                "lanterns",
                "Paper Lanterns Over the Blue Harbor on a Very Long Summer Evening",
                "The Weatherhouse Quartet",
                "weatherhouse",
                "Blue Hours",
                "hours",
            ),
            song(
                "notice",
                "Moment's Notice",
                "John Coltrane",
                "john",
                "Blue Train",
                "train"
            ),
            song(
                "glass",
                "Glass Harbor",
                "Mira Okada",
                "mira",
                "Low Tide Lights",
                "tide"
            ),
            song(
                "bluebird",
                "Bluebird",
                "Blue Note Trio",
                "trio",
                "Low Tide Lights",
                "tide"
            ),
        ]))
        .unwrap(),
        ..SavedLibrary::default()
    }
}

fn song() -> Song {
    serde_json::from_value(json!({
        "id": "s1",
        "title": "Glass Harbor",
        "artist": "Mira Okada",
        "album": "Low Tide Lights",
        "coverArt": "al-1",
        "duration": 245,
        "suffix": "flac",
    }))
    .unwrap()
}

/// A dusk over the sea: dark blue above, orange near the horizon, and a pale sun.
fn cover_pixel(x: u32, y: u32) -> [u8; 4] {
    let (fx, fy) = (x as f32 / 255.0, y as f32 / 255.0);
    let sun = ((fx - 0.62).powi(2) + (fy - 0.58).powi(2)).sqrt() < 0.14;
    if sun {
        return [255, 226, 170, 255];
    }
    let mix = |a: f32, b: f32, t: f32| (a + (b - a) * t) as u8;
    if fy < 0.62 {
        let t = fy / 0.62;
        [
            mix(28.0, 236.0, t),
            mix(36.0, 120.0, t),
            mix(92.0, 84.0, t),
            255,
        ]
    } else {
        let t = (fy - 0.62) / 0.38;
        [
            mix(30.0, 12.0, t),
            mix(62.0, 24.0, t),
            mix(96.0, 48.0, t),
            255,
        ]
    }
}

/// Seeks a twentieth of the song further each step, as the Up key does.
fn seek_steps(ui: &mut Ui, steps: usize) {
    let line = find(Role::Slider, "Position").expect("the seek bar");
    for _ in 0..steps {
        perform(line.id, AccessAction::Increment);
        ui.update();
    }
    // On the pane, away from the bar: the keyboard's focus would show the handle
    let scale = SCALE as f32;
    click(ui, 700.0 * scale, 300.0 * scale);
}

/// Puts the pointer at `x` and `y` in logical pixels.
fn point_at(ui: &mut Ui, x: f32, y: f32) {
    let scale = SCALE as f32;
    ui.handle_event(WindowEvent::PointerMoved {
        position: PointF::new(x * scale, y * scale),
        modifiers: Modifiers::default(),
    });
    ui.update();
}
