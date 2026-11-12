//! Draws the window into PNG files without opening one, to see how it looks:
//! `cargo run -p next-hiphile --example shots -- <folder>`. It needs a GPU, not a display. Menus
//! open as windows of their own, which it does not draw.

use std::path::{Path, PathBuf};

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::core::access::{AccessAction, perform};
use anti_webui::core::event::{Key, Modifiers, NamedKey, WindowEvent};
use anti_webui::core::geometry::{PointF, Scale, SizeI};
use anti_webui::prelude::Role;
use anti_webui_md3e::color_scheme;
use hiphile_core::Hiphile;
use hiphile_core::test_support::{LibraryServer, PlaylistsServer};
use next_hiphile::{MenuBar, Reach, SMALLEST_WINDOW, System, WINDOW, app};
use next_player::testing::{CoverServer, png};
use next_ui::testing::{
    NOWHERE, SavedLibrary, click, click_with, core, find, focus, frames, has_text, key, key_with,
    press, scroll, signed_in_core, signed_in_core_with_library, signed_in_core_with_lyrics, typed,
    wait_until,
};
use next_ui::theme::{DEFAULT_SEED, cover_scheme};
use serde_json::{Value, json};
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
        for (name, size, album) in [
            ("albums", WINDOW, Some("Low Tide Lights")),
            ("albums-compilation", WINDOW, Some("Night Market Radio")),
            ("albums-smallest", SMALLEST_WINDOW, None),
        ] {
            albums_shot(
                &folder.join(format!("{name}-{theme}.png")),
                dark,
                size,
                album,
            );
        }
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
        shot(
            &folder.join(format!("settings-appearance-picked-{theme}.png")),
            dark,
            true,
            |ui| {
                settings(ui, "Appearance");
                focus(ui, Role::RadioGroup, "Color");
                key(ui, NamedKey::ArrowLeft);
                frames(ui, 1000);
            },
        );
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
        playlists_shots(&folder, dark, theme);
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
        queue_shots(&folder, dark, theme, &covers);
        lyrics_shots(&folder, dark, theme, &covers);
    }
}

/// The lyrics in windows of a few sizes, lit word by word partway into a line, and what stands in
/// for them.
fn lyrics_shots(folder: &Path, dark: bool, theme: &str, covers: &CoverServer) {
    let timed = || Ok(lyrics::lyrics_from_richsync_body(&made_up_lyrics()));
    for (name, size) in [
        ("lyrics", WINDOW),
        ("lyrics-wide", WIDE_WINDOW),
        ("lyrics-smallest", SMALLEST_WINDOW),
    ] {
        lyrics_shot(
            &folder.join(format!("{name}-{theme}.png")),
            dark,
            covers,
            size,
            timed(),
            open_lyrics,
        );
    }
    lyrics_shot(
        &folder.join(format!("lyrics-scrolled-{theme}.png")),
        dark,
        covers,
        WINDOW,
        timed(),
        |ui| {
            open_lyrics(ui);
            let scale = SCALE as f32;
            for _ in 0..6 {
                scroll(ui, 900.0 * scale, 400.0 * scale, -60.0 * scale);
            }
            frames(ui, 900);
        },
    );
    let (_core, hiphile) = lyrics_playing(covers, Some(timed()));
    render_in_motion(
        &folder.join(format!("lyrics-flight-{theme}.png")),
        dark,
        hiphile,
        WINDOW,
        |ui| {
            wait_for_cover_colors(ui, dark);
            // Once there and back, for the large cover to have come
            open_lyrics(ui);
            ui.wait_for_images(std::time::Duration::from_secs(30));
            key(ui, NamedKey::Escape);
            frames(ui, 900);
            press(ui, Role::ToggleButton, "Lyrics");
            frames(ui, 70);
        },
    );
    if dark {
        return;
    }
    let found = |lyrics| Some(Ok(lyrics));
    for (name, answer) in [
        ("lyrics-plain", found(plain_lyrics())),
        ("lyrics-none", found(lyrics::Lyrics::NotFound)),
        ("lyrics-failed", Some(Err(lyrics::Error::InvalidResponse))),
        ("lyrics-loading", None),
    ] {
        lyrics_shot_with(
            &folder.join(format!("{name}-{theme}.png")),
            dark,
            covers,
            answer,
            open_lyrics,
        );
    }
}

fn open_lyrics(ui: &mut Ui) {
    press(ui, Role::ToggleButton, "Lyrics");
    frames(ui, 900);
}

fn lyrics_shot(
    path: &Path,
    dark: bool,
    covers: &CoverServer,
    size: (i32, i32),
    found: Result<lyrics::Lyrics, lyrics::Error>,
    drive: impl FnOnce(&mut Ui),
) {
    lyrics_shot_at(path, dark, covers, size, Some(found), drive);
}

fn lyrics_shot_with(
    path: &Path,
    dark: bool,
    covers: &CoverServer,
    answer: Option<Result<lyrics::Lyrics, lyrics::Error>>,
    drive: impl FnOnce(&mut Ui),
) {
    lyrics_shot_at(path, dark, covers, WINDOW, answer, drive);
}

/// The lyrics of a song stopped partway through its third line, which `answer` finds, or which
/// never come for `None`, drawn as soon as `drive` is done.
fn lyrics_shot_at(
    path: &Path,
    dark: bool,
    covers: &CoverServer,
    size: (i32, i32),
    answer: Option<Result<lyrics::Lyrics, lyrics::Error>>,
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = lyrics_playing(covers, answer);
    render_after(path, dark, hiphile, size, 0, move |ui| {
        wait_for_cover_colors(ui, dark);
        drive(ui);
    });
}

/// Signed in on a song stopped partway through its third line, whose lyrics `answer` finds, or
/// never come for `None`.
fn lyrics_playing(
    covers: &CoverServer,
    answer: Option<Result<lyrics::Lyrics, lyrics::Error>>,
) -> (next_ui::testing::TestCore, Hiphile) {
    use futures::FutureExt as _;
    let answer = std::sync::Mutex::new(answer);
    let find: lyrics::FindLyrics =
        std::sync::Arc::new(move |_| match answer.lock().unwrap().take() {
            Some(answer) => futures::future::ready(answer).boxed(),
            None => futures::future::pending().boxed(),
        });
    let (core, hiphile) = signed_in_core_with_lyrics(covers.url(), find);
    let player = hiphile.workspace().expect("signed in").player().clone();
    player.fill_queue(&[song()], 0);
    player.place_at(std::time::Duration::from_millis(19_700));
    (core, hiphile)
}

fn wait_for_cover_colors(ui: &mut Ui, dark: bool) {
    let default = cover_scheme(DEFAULT_SEED, dark).primary;
    for _ in 0..500 {
        if color_scheme().primary != default {
            break;
        }
        frames(ui, 16);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    frames(ui, 300);
}

/// Lyrics made up for the shots, a line each 4.5 seconds from ten seconds in, timed word by
/// word, in English and in Japanese.
fn made_up_lyrics() -> String {
    const LINES: [&str; 14] = [
        "Lanterns drift along the harbor",
        "潮が 引くたび 光が 揺れる",
        "Every ferry carries someone home",
        "名前の ない 夜を 越えていく",
        "Hold the signal, hold the shore",
        "波の 音だけが 道しるべ",
        "We were glass and we were light",
        "まだ 消えない 灯りを 抱いて",
        "Count the beacons one by one",
        "遠い 岬で 誰かが 呼ぶ",
        "Till the tide gives back the sun",
        "朝が 来るまで ここに いるよ",
        "Lanterns drift along the harbor",
        "潮が 引くたび 光が 揺れる",
    ];
    let lines: Vec<Value> = LINES
        .iter()
        .enumerate()
        .map(|(ix, line)| {
            let start = 10.0 + ix as f64 * 4.5;
            let words: Vec<&str> = line.split(' ').collect();
            let step = 3.6 / words.len() as f64;
            let parts: Vec<Value> = words
                .iter()
                .enumerate()
                .map(|(at, word)| {
                    let shown = if line.is_ascii() && at + 1 < words.len() {
                        format!("{word} ")
                    } else {
                        (*word).to_owned()
                    };
                    json!({ "c": shown, "o": at as f64 * step })
                })
                .collect();
            let text: String = parts
                .iter()
                .map(|part| part["c"].as_str().unwrap())
                .collect();
            json!({ "ts": start, "te": start + 4.2, "x": text, "l": parts })
        })
        .collect();
    Value::Array(lines).to_string()
}

fn plain_lyrics() -> lyrics::Lyrics {
    let text = [
        "Lanterns drift along the harbor",
        "潮が引くたび 光が揺れる",
        "Every ferry carries someone home",
        "名前のない夜を 越えていく",
        "",
        "Hold the signal, hold the shore",
        "波の音だけが 道しるべ",
        "We were glass and we were light",
        "まだ消えない 灯りを抱いて",
    ]
    .join("\n");
    lyrics::plain_lyrics(&text, Some("Words made up for these pictures"))
}

/// The queue where it fits in windows of a few sizes, with rows selected, and empty.
fn queue_shots(folder: &Path, dark: bool, theme: &str, covers: &CoverServer) {
    for (name, size) in [
        ("queue", WINDOW),
        ("queue-wide", WIDE_WINDOW),
        ("queue-narrow", SMALLEST_WINDOW),
    ] {
        playing_shot_at(
            &folder.join(format!("{name}-{theme}.png")),
            dark,
            covers,
            size,
            &queue(),
            |player| player.pretend_playing(true),
            open_queue,
        );
    }
    playing_shot_at(
        &folder.join(format!("queue-selected-{theme}.png")),
        dark,
        covers,
        WINDOW,
        &queue(),
        |player| player.pretend_playing(false),
        |ui| {
            open_queue(ui);
            select_rows(ui, &["Night Ferry", "Breakwater"]);
        },
    );
    shot(
        &folder.join(format!("queue-empty-{theme}.png")),
        dark,
        true,
        open_queue,
    );
}

/// Wider than most windows open, to show the queue at its own width.
const WIDE_WINDOW: (i32, i32) = (1440, 900);

/// Opens the queue where it fits.
fn open_queue(ui: &mut Ui) {
    let toggle = find(Role::ToggleButton, "Queue").expect("the queue's toggle");
    if !toggle.disabled {
        press(ui, Role::ToggleButton, "Queue");
    }
}

/// Clicks the first of the rows named `titles` and adds the rest to the selection, as ⌘ does.
fn select_rows(ui: &mut Ui, titles: &[&str]) {
    let scale = SCALE as f32;
    for (at, title) in titles.iter().enumerate() {
        let row = find(Role::ListOption, title).expect("the row");
        let (x, y) = (
            (row.bounds.x + row.bounds.width / 2.0) * scale,
            (row.bounds.y + row.bounds.height / 2.0) * scale,
        );
        let modifiers = if at == 0 {
            Modifiers::default()
        } else {
            Modifiers::PRIMARY
        };
        click_with(ui, x, y, modifiers);
        frames(ui, 400);
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
            wait_until(ui, || has_text("No matches for “xylophone”"));
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
    playing_shot_at(path, dark, covers, WINDOW, &[song()], setup, drive);
}

/// As [`playing_shot`], in a window of `size` with `queue` in the queue: its fourth song is the
/// current one where it has four.
fn playing_shot_at(
    path: &Path,
    dark: bool,
    covers: &CoverServer,
    size: (i32, i32),
    queue: &[Song],
    setup: impl FnOnce(&playback::Player),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = signed_in_core(covers.url());
    let player = hiphile.workspace().expect("signed in").player().clone();
    player.fill_queue(queue, 3.min(queue.len() - 1));
    setup(&player);
    let default = cover_scheme(DEFAULT_SEED, dark).primary;
    render(path, dark, hiphile, size, move |ui| {
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

/// The playlists, the narrowest window's, another user's, renaming, deleting and none.
fn playlists_shots(folder: &Path, dark: bool, theme: &str) {
    playlists_shot(
        &folder.join(format!("playlists-{theme}.png")),
        dark,
        &playlists_server(),
        WINDOW,
        |ui| show_playlists(ui, "Glass Harbor"),
    );
    playlists_shot(
        &folder.join(format!("playlists-narrow-{theme}.png")),
        dark,
        &playlists_server(),
        SMALLEST_WINDOW,
        |ui| show_playlists(ui, "Glass Harbor"),
    );
    playlists_shot(
        &folder.join(format!("playlists-shared-{theme}.png")),
        dark,
        &playlists_server(),
        WINDOW,
        |ui| {
            show_playlists(ui, "Glass Harbor");
            press(ui, Role::ListOption, "Late Night Sessions");
            wait_until(ui, || find(Role::ListOption, "Smoke Rings").is_some());
        },
    );
    playlists_shot(
        &folder.join(format!("playlists-rename-{theme}.png")),
        dark,
        &playlists_server(),
        WINDOW,
        |ui| {
            show_playlists(ui, "Glass Harbor");
            key(ui, NamedKey::F(2));
        },
    );
    playlists_shot(
        &folder.join(format!("playlists-delete-{theme}.png")),
        dark,
        &playlists_server(),
        WINDOW,
        |ui| {
            show_playlists(ui, "Glass Harbor");
            key_with(ui, Key::Named(NamedKey::Backspace), Modifiers::PRIMARY);
        },
    );
    playlists_shot(
        &folder.join(format!("playlists-empty-{theme}.png")),
        dark,
        &PlaylistsServer::new(&[]),
        WINDOW,
        |ui| {
            press(ui, Role::Button, "Playlists");
            frames(ui, 400);
        },
    );
}

/// The window signed in to `server`, once `drive` has shown what the shot is of and the covers
/// have come.
fn playlists_shot(
    path: &Path,
    dark: bool,
    server: &PlaylistsServer,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
    let (_core, hiphile) = signed_in_core(server.url());
    render(path, dark, hiphile, size, move |ui| {
        drive(ui);
        for _ in 0..200 {
            ui.poll_images();
            frames(ui, 8);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    });
}

/// Goes to the playlists and waits for the first one's songs, of which `song` is one.
fn show_playlists(ui: &mut Ui, song: &str) {
    press(ui, Role::Button, "Playlists");
    wait_until(ui, || find(Role::ListOption, song).is_some());
    frames(ui, 400);
}

/// Playlists of alice's and one of bob's, of songs on four albums.
fn playlists_server() -> PlaylistsServer {
    let server = PlaylistsServer::new(&[
        (
            "p1",
            "Coastal Drive",
            &["1", "2", "3", "4", "5", "6", "7", "8", "9"],
        ),
        ("p2", "Late Night Sessions", &["10", "11", "12"]),
        ("p3", "Focus", &["13", "14", "2", "6"]),
        ("p4", "Rainy Sunday", &["15", "16"]),
        ("p5", "Workout", &[]),
    ]);
    server.make_readonly("p2");
    server.describe_playlist(
        "p1",
        json!({ "comment": "Long roads after sunset", "coverArt": "pl-1" }),
    );
    server.describe_playlist("p2", json!({ "coverArt": "pl-2" }));
    for (id, title, artist, album, duration) in PLAYLIST_SONGS {
        let cover = format!("al-{}", album.0);
        server.describe_song(
            id,
            json!({
                "title": title,
                "artist": artist,
                "artistId": format!("ar-{artist}"),
                "album": album.1,
                "albumId": format!("al-{}", album.0),
                "coverArt": cover,
                "duration": duration,
            }),
        );
    }
    for (id, top, bottom) in [
        ("al-1", [28, 36, 92], [236, 120, 84]),
        ("al-2", [18, 74, 70], [190, 222, 160]),
        ("al-3", [64, 30, 82], [214, 140, 196]),
        ("al-4", [90, 70, 40], [240, 206, 120]),
        ("pl-1", [30, 40, 70], [250, 160, 90]),
        ("pl-2", [20, 30, 60], [110, 150, 220]),
    ] {
        server.serve_cover(id, png(256, move |_, y| gradient(top, bottom, y)));
    }
    server.serve_favorites(&["3", "11"]);
    server
}

/// Album id and title.
type Album = (u8, &'static str);

const LOW_TIDE: Album = (1, "Low Tide Lights");
const GREEN_ROOM: Album = (2, "The Green Room");
const VELVET: Album = (3, "Velvet Static");
const AMBER: Album = (4, "Amber Hours");

/// Id, title, artist, album and seconds.
const PLAYLIST_SONGS: [(&str, &str, &str, Album, u32); 16] = [
    ("1", "Glass Harbor", "Mira Okada", LOW_TIDE, 245),
    ("2", "Paper Lanterns", "Mira Okada", LOW_TIDE, 212),
    ("3", "Northbound", "The Quiet Fields", GREEN_ROOM, 198),
    ("4", "Signal Fires", "The Quiet Fields", GREEN_ROOM, 263),
    ("5", "Chrome Rain", "Velvet Static", VELVET, 187),
    ("6", "Afterglow Avenue", "Velvet Static", VELVET, 301),
    ("7", "Salt and Cedar", "Mira Okada", LOW_TIDE, 229),
    ("8", "Long Exposure", "Juno Avery", AMBER, 276),
    ("9", "Last Ferry Home", "Juno Avery", AMBER, 334),
    ("10", "Smoke Rings", "Theo Marsh Trio", AMBER, 402),
    ("11", "Quarter to Two", "Theo Marsh Trio", AMBER, 357),
    ("12", "Slow Burn Waltz", "Theo Marsh Trio", AMBER, 389),
    ("13", "Still Water", "The Quiet Fields", GREEN_ROOM, 241),
    ("14", "Grid Lines", "Velvet Static", VELVET, 205),
    ("15", "Window Seat", "Juno Avery", AMBER, 218),
    ("16", "Puddles", "Mira Okada", LOW_TIDE, 176),
];

/// From `top` at the top of a cover to `bottom` at its bottom.
fn gradient(top: [u8; 3], bottom: [u8; 3], y: u32) -> [u8; 4] {
    let t = y as f32 / 255.0;
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    [
        mix(top[0], bottom[0]),
        mix(top[1], bottom[1]),
        mix(top[2], bottom[2]),
        255,
    ]
}

fn render(
    path: &Path,
    dark: bool,
    hiphile: Hiphile,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
    render_after(path, dark, hiphile, size, 900, drive);
}

/// Draws the window just as `drive` leaves it, which `render` cannot: once pictures have come, the
/// headless renderer plays a second of frames before it draws.
fn render_in_motion(
    path: &Path,
    dark: bool,
    hiphile: Hiphile,
    size: (i32, i32),
    drive: impl FnOnce(&mut Ui),
) {
    use anti_webui::wgpu::{Gpu, Renderer, wgpu::TextureFormat};
    let (width, height) = (
        (size.0 as f64 * SCALE) as u32,
        (size.1 as f64 * SCALE) as u32,
    );
    let gpu = Gpu::headless().expect("a GPU to draw with");
    let mut renderer = Renderer::new(&gpu.device, TextureFormat::Rgba8Unorm);
    let mut ui = Ui::new(move || app(hiphile, System::fixed(dark), MenuBar::new(), Reach::Window));
    ui.handle_event(WindowEvent::Resized {
        size: SizeI::new(width as i32, height as i32),
        scale: Scale::from_f64(SCALE),
    });
    ui.handle_event(WindowEvent::Focused(true));
    ui.update();
    frames(&mut ui, 100);
    drive(&mut ui);
    ui.update();
    let scene = ui.paint(std::time::Instant::now());
    let pixels = gpu.render_to_rgba(&mut renderer, &scene);
    headless::write_png(path, width, height, &pixels).expect("the shot written");
    println!("{}", path.display());
}

/// Draws the window `settle` milliseconds after `drive` is done.
fn render_after(
    path: &Path,
    dark: bool,
    hiphile: Hiphile,
    size: (i32, i32),
    settle: u64,
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
        move || app(hiphile, System::fixed(dark), MenuBar::new(), Reach::Window),
        |ui| {
            frames(ui, 100);
            drive(ui);
            frames(ui, settle);
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

/// Songs of a few albums, one with a title too long for the queue's rows.
fn queue() -> Vec<Song> {
    [
        ("Glass Harbor", "Mira Okada", 245),
        ("Paper Lanterns", "Mira Okada", 198),
        ("Undertow", "Mira Okada", 312),
        (
            "Signal Fires Along the Northern Coastline at Dusk",
            "Mira Okada & The Low Tide Ensemble",
            427,
        ),
        ("Saltwater", "Kenji Arai", 181),
        ("Night Ferry", "Kenji Arai", 264),
        ("Breakwater", "Mira Okada", 233),
        ("Harbor Lights (Reprise)", "Mira Okada", 95),
        ("Drift", "Sora Lin", 289),
        ("Tidal", "Sora Lin", 204),
        ("Low Tide Lights", "Mira Okada", 356),
        ("Afterglow", "Sora Lin", 242),
        ("Moonlit Pier", "Kenji Arai", 301),
        ("Shoreline", "Sora Lin", 219),
    ]
    .into_iter()
    .enumerate()
    .map(|(at, (title, artist, duration))| {
        serde_json::from_value(json!({
            "id": format!("q{at}"),
            "title": title,
            "artist": artist,
            "artistId": format!("ar-{artist}"),
            "album": "Low Tide Lights",
            "albumId": "al-1",
            "coverArt": "al-1",
            "duration": duration,
            "suffix": "flac",
        }))
        .unwrap()
    })
    .collect()
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

/// Albums on a server of [`library`] with covers, showing `album`, or the first.
fn albums_shot(path: &Path, dark: bool, size: (i32, i32), album: Option<&str>) {
    let server = LibraryServer::new(albums_library());
    server.serve_covers(|id| Some(png(256, |x, y| album_cover_pixel(id, x, y))));
    let (_core, hiphile) = signed_in_core(server.url());
    render(path, dark, hiphile, size, |ui| {
        press(ui, Role::Button, "Albums");
        wait_until(ui, || find(Role::ListBox, "Songs").is_some());
        if let Some(album) = album {
            press(ui, Role::ListOption, album);
            wait_until(ui, || {
                find(Role::Heading, album).is_some() && find(Role::ListBox, "Songs").is_some()
            });
        }
        // The covers come over the network
        for _ in 0..150 {
            ui.poll_images();
            frames(ui, 8);
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
    });
}

/// A made-up album: its name, its artist, its year, and the titles and lengths in seconds of its
/// songs, none for one by several artists.
type MadeUpAlbum = (
    &'static str,
    &'static str,
    u32,
    &'static [(&'static str, u32)],
);

const ALBUMS: [MadeUpAlbum; 14] = [
    (
        "Low Tide Lights",
        "Mira Okada",
        2021,
        &[
            ("Glass Harbor", 245),
            ("Paper Lanterns", 212),
            ("The Long Swim Home", 318),
            ("Undertow", 197),
            ("Salt on the Window", 264),
            ("Breakwater", 233),
            (
                "Ferry Lights at Four in the Morning, Seen from the Pier",
                291,
            ),
            ("Kelp Forest", 186),
            ("Low Tide", 402),
        ],
    ),
    (
        "Paper Satellites",
        "The Quiet Engines",
        2019,
        &[
            ("Orbit", 221),
            ("Static Bloom", 248),
            ("Ground Control", 305),
        ],
    ),
    (
        "Northbound Static",
        "Ines Calder",
        2023,
        &[("Radio North", 233), ("Snowline", 279)],
    ),
    (
        "Glass Orchard",
        "Hollow Pines",
        2017,
        &[("Orchard", 256), ("Frost Fruit", 199)],
    ),
    (
        "Saltwater Hymns",
        "Mira Okada",
        2018,
        &[("Hymn I", 188), ("Hymn II", 207)],
    ),
    ("Night Market Radio", "Various Artists", 2022, &[]),
    (
        "Amber Signal",
        "Tomas Reyes Trio",
        2015,
        &[("Amber", 342), ("Signal Fire", 411)],
    ),
    (
        "Field Notes",
        "Ada Lindqvist",
        2020,
        &[("Notebook", 174), ("Meadow", 230)],
    ),
    (
        "Velvet Interval",
        "Kaito Mori",
        2016,
        &[("Interval", 265), ("Velvet", 241)],
    ),
    (
        "Small Weather",
        "The Lantern Club",
        2024,
        &[("Drizzle", 162), ("Small Weather", 219)],
    ),
    (
        "Copper Coast",
        "Ines Calder",
        2014,
        &[("Copper", 247), ("Coastline", 288)],
    ),
    (
        "Echo Garden",
        "Sunfold",
        2012,
        &[("Echo", 301), ("Garden", 276)],
    ),
    (
        "Midnight Ferry",
        "Hollow Pines",
        2021,
        &[("Midnight", 239), ("Ferry", 254)],
    ),
    (
        "A Long Way to the Lighthouse (Deluxe Edition, Remastered)",
        "The Quiet Engines",
        2010,
        &[("Lighthouse", 296), ("Keeper", 233)],
    ),
];

/// The songs of the album by several artists: titles, artists and lengths in seconds.
const COMPILATION: [(&str, &str, u32); 6] = [
    ("Neon Noodle Stand", "Kaito Mori", 204),
    ("Lantern Row", "The Lantern Club", 231),
    ("Two for One", "Ada Lindqvist", 188),
    ("Closing Time Shutters", "Sunfold", 266),
    ("Rain on Tarp", "Mira Okada", 242),
    ("Last Bus", "Tomas Reyes Trio", 315),
];

/// Albums of made-up artists, one of them by several, as `getAlbum` gives them, in the order of
/// their names as a server lists them.
fn albums_library() -> Vec<Value> {
    let mut albums = ALBUMS.to_vec();
    albums.sort_by_key(|(name, ..)| name.to_lowercase());
    albums
        .iter()
        .enumerate()
        .map(|(at, (name, artist, year, songs))| {
            let id = format!("al-{at}");
            let song = |track: usize, title: &str, by: &str, duration: u32| {
                json!({
                    "id": format!("{id}-{track}"),
                    "title": title,
                    "track": track,
                    "duration": duration,
                    "artist": by,
                    "artistId": by.to_lowercase().replace(' ', "-"),
                    "album": name,
                    "albumId": id,
                    "coverArt": id,
                })
            };
            let songs: Vec<Value> = if songs.is_empty() {
                COMPILATION
                    .iter()
                    .enumerate()
                    .map(|(at, (title, by, duration))| song(at + 1, title, by, *duration))
                    .collect()
            } else {
                songs
                    .iter()
                    .enumerate()
                    .map(|(at, (title, duration))| song(at + 1, title, artist, *duration))
                    .collect()
            };
            let mut album = json!({
                "id": id,
                "name": name,
                "artist": artist,
                "year": year,
                "coverArt": id,
                "songCount": songs.len(),
                "song": songs,
            });
            if *artist != "Various Artists" {
                album["artistId"] = json!(artist.to_lowercase().replace(' ', "-"));
            }
            album
        })
        .collect()
}

/// A cover of its own for each album: a sky that fades to a sea, with a sun.
fn album_cover_pixel(id: &str, x: u32, y: u32) -> [u8; 4] {
    const SKIES: [([f32; 3], [f32; 3]); 7] = [
        ([28.0, 36.0, 92.0], [236.0, 120.0, 84.0]),
        ([18.0, 70.0, 64.0], [190.0, 214.0, 140.0]),
        ([90.0, 24.0, 60.0], [250.0, 170.0, 120.0]),
        ([40.0, 40.0, 48.0], [170.0, 180.0, 200.0]),
        ([12.0, 52.0, 110.0], [120.0, 200.0, 230.0]),
        ([70.0, 40.0, 20.0], [240.0, 200.0, 110.0]),
        ([50.0, 20.0, 90.0], [200.0, 150.0, 240.0]),
    ];
    let index: usize = id.bytes().map(usize::from).sum();
    let (top, bottom) = SKIES[index % SKIES.len()];
    let (fx, fy) = (x as f32 / 255.0, y as f32 / 255.0);
    let sun_x = 0.25 + (index % 5) as f32 * 0.12;
    if ((fx - sun_x).powi(2) + (fy - 0.5).powi(2)).sqrt() < 0.13 {
        return [255, 236, 200, 255];
    }
    let t = fy.min(0.65) / 0.65;
    let shade = if fy > 0.65 { 0.45 } else { 1.0 };
    let mix =
        |channel: usize| ((top[channel] + (bottom[channel] - top[channel]) * t) * shade) as u8;
    [mix(0), mix(1), mix(2), 255]
}
