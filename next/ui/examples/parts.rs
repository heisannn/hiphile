//! Draws the shared parts into PNG files without opening a window, to see how they look:
//! `cargo run -p next-ui --example parts -- <folder>`. It needs a GPU, not a display. Each shot is
//! what is right of the rail, as wide as with the rail open or, for the narrow ones, closed in the
//! narrowest window. Menus open as windows of their own, which it does not draw.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::prelude::*;
use anti_webui_md3e::components::{
    ButtonStyle, IconButtonStyle, carousel, image, styled_button, styled_icon_button,
};
use anti_webui_md3e::typography::TextStyle;
use anti_webui_md3e::{color_scheme, icons, set_color_scheme};
use next_ui::card_grid::{Card, card, card_row, columns_for};
use next_ui::detail_header::{DetailHeader, detail_header, play_and_shuffle};
use next_ui::details::{format_album_count, format_duration, format_song_count, join_details};
use next_ui::list_detail::{Split, list_detail};
use next_ui::list_pane::{ListHeader, list_filter, list_header, refresh_failure};
use next_ui::load_state::{empty_state, load_failure, no_matches, pane_loading};
use next_ui::now_playing::{PlayState, play_state_icon, skippable, song_row_label};
use next_ui::pane::{GAP, pane};
use next_ui::row_list::{ROW_PADDING, RowList, Selection, row_list};
use next_ui::row_text::row_text;
use next_ui::section::{
    section_failure, section_heading, section_heading_with, section_loading, section_note,
};
use next_ui::sort_menu::{SortOption, sort_menu};
use next_ui::testing::{frames, key, key_with, scroll};
use next_ui::theme::{DEFAULT_SEED, scheme};
use next_ui::type_scale::{body, muted_text};
use next_ui::{Covers, app_bar};

const SCALE: f64 = 2.0;
const HEIGHT: u32 = 760;
/// The window's width less the open rail.
const WIDE: u32 = 1200 - 220;
/// The narrowest window's width less the closed rail.
const NARROW: u32 = 900 - 96;
const COVERS: usize = 8;

fn main() {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "parts".to_owned()),
    );
    std::fs::create_dir_all(folder.join("covers")).expect("a folder for the shots");
    let covers: Rc<[PathBuf]> = (0..COVERS)
        .map(|ix| write_cover(&folder.join("covers"), ix))
        .collect();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let at = |name: &str| folder.join(format!("{name}-{theme}.png"));
        let c = covers.clone();
        shot(&at("albums"), WIDE, dark, move || albums(c, false), |_| {});
        let c = covers.clone();
        shot(
            &at("albums-narrow"),
            NARROW,
            dark,
            move || albums(c, true),
            |_| {},
        );
        let c = covers.clone();
        shot(&at("artist"), WIDE, dark, move || artist(c), |_| {});
        let c = covers.clone();
        shot(&at("home"), WIDE, dark, move || home(c), |_| {});
        let c = covers.clone();
        shot(
            &at("home-scrolled"),
            WIDE,
            dark,
            move || home(c),
            |ui| {
                let scale = SCALE as f32;
                scroll(ui, WIDE as f32 / 2.0 * scale, 600.0 * scale, 700.0 * scale);
            },
        );
        shot(&at("states"), WIDE, dark, states, |_| {});
        let c = covers.clone();
        shot(
            &at("focus-tab"),
            WIDE,
            dark,
            move || focus(c),
            |ui| {
                key(ui, NamedKey::Tab);
            },
        );
        let c = covers.clone();
        shot(
            &at("focus-keys"),
            WIDE,
            dark,
            move || focus(c),
            |ui| {
                key(ui, NamedKey::Tab);
                key(ui, NamedKey::ArrowDown);
                key(ui, NamedKey::ArrowDown);
                let shift = Modifiers::default().with_shift();
                key_with(ui, Key::Named(NamedKey::ArrowDown), shift);
                key_with(ui, Key::Named(NamedKey::ArrowDown), shift);
            },
        );
    }
}

fn shot<V: IntoView>(
    path: &Path,
    width: u32,
    dark: bool,
    view: impl FnOnce() -> V,
    drive: impl FnOnce(&mut Ui),
) {
    let (w, h) = (
        (width as f64 * SCALE) as u32,
        (HEIGHT as f64 * SCALE) as u32,
    );
    let pixels = headless::render_driven_rgba(
        w,
        h,
        SCALE,
        move || {
            set_color_scheme(scheme(DEFAULT_SEED, dark));
            view()
        },
        |ui| {
            frames(ui, 100);
            drive(ui);
            frames(ui, 900);
        },
    )
    .expect("a GPU to draw with");
    headless::write_png(path, w, h, &pixels).expect("the shot written");
    println!("{}", path.display());
}

/// A square of colors that runs from one hue to the next, as a stand-in for a cover.
fn write_cover(folder: &Path, ix: usize) -> PathBuf {
    const SIDE: u32 = 256;
    let hue = ix as f32 / COVERS as f32;
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let t = (x + y) as f32 / (SIDE * 2) as f32;
            let [r, g, b] = hsv(hue + t * 0.15, 0.55, 0.9 - t * 0.35);
            pixels.extend([r, g, b, 255]);
        }
    }
    let path = folder.join(format!("{ix}.png"));
    headless::write_png(&path, SIDE, SIDE, &pixels).expect("the cover written");
    path
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

fn picture(path: &Path) -> Div {
    div()
        .relative()
        .overflow_hidden()
        .flex_none()
        .child(image(ImageSource::Path(path.into())).absolute().inset(0))
}

/// What is right of the rail: the bar, and the body on its panes.
fn window(title: &str, actions: Vec<Div>, body_: Div) -> Div {
    body(div())
        .size_full()
        .flex()
        .flex_col()
        .dyn_style(|s| {
            let c = color_scheme();
            s.bg(c.surface).text_color(c.on_surface)
        })
        .child(app_bar(Vec::new(), title.to_owned(), actions))
        .child(
            div()
                .flex_1()
                .min_h(0)
                .flex()
                .flex_col()
                .ml(GAP)
                .mr(GAP)
                .mb(GAP)
                .child(body_.flex_1().min_h(0)),
        )
}

#[derive(Clone)]
struct Album {
    id: usize,
    title: &'static str,
    artist: &'static str,
}

const ALBUMS: &[(&str, &str)] = &[
    ("Blue Hours", "Mina Arai"),
    (
        "Paper Lanterns of a Very Long Summer Evening",
        "The Weatherhouse Quartet",
    ),
    ("Northern Lines", "Odd Harbour"),
    ("Small Rooms", "Mina Arai"),
    ("Coastal", "Lumen & Vale"),
    ("Ferns", "Odd Harbour"),
    ("Night Market", "Kaito Mori"),
    ("Second Light", "Lumen & Vale"),
    ("Glasswork", "The Weatherhouse Quartet"),
    ("Low Tide", "Kaito Mori"),
    ("Morning Static", "Odd Harbour"),
    ("Hearth", "Mina Arai"),
    ("Valleys", "Lumen & Vale"),
    ("Driftwood", "Kaito Mori"),
];

fn albums_list() -> Rc<[Album]> {
    ALBUMS
        .iter()
        .enumerate()
        .map(|(id, (title, artist))| Album { id, title, artist })
        .collect()
}

#[derive(Clone)]
struct Track {
    id: usize,
    title: &'static str,
    seconds: u64,
}

const TRACKS: &[(&str, u64)] = &[
    ("Opening", 188),
    ("Blue Hours", 254),
    ("Window Seat", 231),
    ("Blue Hours (Instrumental)", 254),
    ("The Long Way Home", 312),
    ("Quiet Streets", 205),
    ("Last Train", 276),
    ("Afterglow", 243),
];

fn tracks() -> Rc<[Track]> {
    TRACKS
        .iter()
        .enumerate()
        .map(|(id, (title, seconds))| Track {
            id,
            title,
            seconds: *seconds,
        })
        .collect()
}

fn sort() -> Div {
    let order = signal(0usize);
    let descending = signal(false);
    let by = |label: &str, to: usize| {
        SortOption::new(label, move || order.get() == to, move || order.set(to))
    };
    let way = |label: &str, to: bool| {
        SortOption::new(
            label,
            move || descending.get() == to,
            move || descending.set(to),
        )
    };
    sort_menu(
        Covers::new(),
        move || descending.get(),
        vec![
            vec![by("Title", 0), by("Artist", 1), by("Year", 2)],
            vec![way("Ascending", false), way("Descending", true)],
        ],
    )
}

fn albums(covers: Rc<[PathBuf]>, narrow: bool) -> Div {
    let filter = signal(if narrow {
        "blue".to_owned()
    } else {
        String::new()
    });
    let selection = Selection::new();
    selection.select(0usize);
    let rows = albums_list();
    let shown = rows.clone();
    let row_covers = covers.clone();
    let list = RowList::new(
        "Albums",
        move || shown.clone(),
        |album: &Album| album.id,
        move |album, _| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(12.0_f32)
                .px(ROW_PADDING)
                .size_full()
                .child(
                    picture(&row_covers[album.id % COVERS])
                        .size(40.0_f32)
                        .rounded(8.0_f32),
                )
                .child(row_text(album.title, Some(album.artist.to_owned())).flex_1())
        },
    )
    .row_height(56.0)
    .selection(selection);
    let rows = node_ref();
    let list = list.node_ref(rows);
    let mut list_pane = div()
        .flex()
        .flex_col()
        .child(list_filter(filter, "Filter Albums", rows))
        .child(list_header(
            ListHeader::new(|| Some(format_album_count(128))).updating("Updating Albums", || true),
        ));
    if narrow {
        list_pane = list_pane.child(refresh_failure(
            "Albums didn't update",
            "The server didn't answer.",
            || {},
        ));
    }
    let list_pane = list_pane.child(row_list(list).flex_1().min_h(0));
    window(
        "Albums",
        vec![sort()],
        list_detail(Split::new(), list_pane, album_detail(&covers[0])),
    )
}

fn album_detail(cover: &Path) -> Div {
    let songs = tracks();
    let total = Duration::from_secs(songs.iter().map(|song| song.seconds).sum());
    let header = DetailHeader::new(
        picture(cover),
        "Blue Hours",
        join_details([
            Some("2021".to_owned()),
            Some(format_song_count(songs.len())),
            Some(format_duration(total)),
        ]),
    )
    .subtitle(link("Mina Arai"))
    .comment(Some(
        "Recorded over one winter in a small room above a bakery, with the window open whenever \
         the snow allowed it. The second side was mixed again in the spring."
            .to_owned(),
    ))
    .action(play_and_shuffle(|| {}, || {}, || false));
    let playing = 1usize;
    let skipped = 3usize;
    let list = RowList::new(
        "Songs",
        move || songs.clone(),
        |song: &Track| song.id,
        move |song, _| {
            let id = song.id;
            let state = (id == playing).then_some(PlayState::Playing);
            skippable(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(12.0_f32)
                    .px(ROW_PADDING)
                    .size_full()
                    .child(dyn_view(move || match state {
                        Some(_) => play_state_icon(move || state).into_view(),
                        None => muted_text(TextStyle::BODY_MEDIUM, (id + 1).to_string())
                            .w(20.0_f32)
                            .text_align(TextAlign::Center)
                            .into_view(),
                    }))
                    .child(row_text(song.title, None).flex_1())
                    .child(muted_text(
                        TextStyle::BODY_MEDIUM,
                        format_duration(Duration::from_secs(song.seconds)),
                    )),
                move || id == skipped,
            )
        },
    )
    .row_label(move |song| {
        song_row_label(
            song.title,
            (song.id == playing).then_some(PlayState::Playing),
            song.id == skipped,
        )
    });
    div()
        .flex()
        .flex_col()
        .child(detail_header(header))
        .child(row_list(list).flex_1().min_h(0))
}

/// A list and cards that the keys are on.
fn focus(covers: Rc<[PathBuf]>) -> Div {
    let songs = tracks();
    let list = RowList::new(
        "Songs",
        move || songs.clone(),
        |song: &Track| song.id,
        |song, _| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(12.0_f32)
                .px(ROW_PADDING)
                .size_full()
                .child(row_text(song.title, None).flex_1())
                .child(muted_text(
                    TextStyle::BODY_MEDIUM,
                    format_duration(Duration::from_secs(song.seconds)),
                ))
        },
    );
    let cards = card_row(
        4,
        albums_list()
            .iter()
            .take(4)
            .map(|album| {
                let id = album.id;
                card(
                    Card::new(picture(&covers[id % COVERS]), album.title)
                        .details(album.artist)
                        .selected_when(move || id == 1)
                        .focused_when(|| true)
                        .keyed_when(move || id == 1),
                )
            })
            .collect(),
    );
    window(
        "Focus",
        Vec::new(),
        list_detail(
            Split::new(),
            div()
                .flex()
                .flex_col()
                .child(row_list(list).flex_1().min_h(0)),
            div().flex().flex_col().child(cards),
        ),
    )
}

fn link(name: &str) -> Div {
    div()
        .dyn_style(|s| s.text_color(color_scheme().primary))
        .child(text(name.to_owned()))
}

fn artist(covers: Rc<[PathBuf]>) -> Div {
    let names: Rc<[(usize, &str)]> = [
        "Kaito Mori",
        "Lumen & Vale",
        "Mina Arai",
        "Odd Harbour",
        "The Weatherhouse Quartet",
    ]
    .into_iter()
    .enumerate()
    .collect();
    let selection = Selection::new();
    selection.select(2usize);
    let row_covers = covers.clone();
    let list = RowList::new(
        "Artists",
        move || names.clone(),
        |artist: &(usize, &str)| artist.0,
        move |artist, _| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(12.0_f32)
                .px(ROW_PADDING)
                .size_full()
                .child(
                    picture(&row_covers[(artist.0 + 3) % COVERS])
                        .size(40.0_f32)
                        .rounded(20.0_f32),
                )
                .child(row_text(artist.1, None).flex_1())
        },
    )
    .row_height(56.0)
    .selection(selection);
    let rows = node_ref();
    let list = list.node_ref(rows);
    let list_pane = div()
        .flex()
        .flex_col()
        .child(list_filter(signal(String::new()), "Filter Artists", rows))
        .child(list_header(ListHeader::new(|| {
            Some("5 artists".to_owned())
        })))
        .child(row_list(list).flex_1().min_h(0));
    let header = DetailHeader::new(
        picture(&covers[5]),
        "Mina Arai",
        join_details([Some(format_album_count(3)), Some(format_song_count(34))]),
    )
    .artist_picture()
    .comment(Some(
        "A songwriter from the north coast, known for quiet records made in small rooms."
            .to_owned(),
    ))
    .action(play_and_shuffle(|| {}, || {}, || false));
    let width = signal(0.0f32);
    let albums = albums_list();
    let shelf_covers = covers.clone();
    let shelf = dyn_view(move || {
        let columns = columns_for(width.get());
        let covers = shelf_covers.clone();
        let albums = albums.clone();
        untrack(move || {
            card_row(
                columns,
                albums
                    .iter()
                    .filter(|album| album.artist == "Mina Arai")
                    .map(|album| {
                        card(
                            Card::new(picture(&covers[album.id % COVERS]), album.title)
                                .details("2021"),
                        )
                    })
                    .collect(),
            )
            .into_view()
        })
    });
    let detail = div()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .on_resized(move |w, _| width.set_if_changed(w))
        .child(detail_header(header))
        .child(section_heading("Albums"))
        .child(shelf)
        .child(section_heading_with(
            "Top Songs",
            styled_icon_button(icons::REFRESH, IconButtonStyle::standard())
                .access(|a| a.label("Reload Top Songs")),
        ))
        .child(section_loading("Loading Top Songs"))
        .child(section_heading("Similar Artists"))
        .child(section_failure("The server didn't answer.", || {}))
        .child(section_heading("Appears On"))
        .child(section_note("Mina Arai appears on no other albums."));
    window(
        "Artists",
        Vec::new(),
        list_detail(Split::new(), list_pane, detail),
    )
}

fn home(covers: Rc<[PathBuf]>) -> Div {
    let width = signal(0.0f32);
    let albums = albums_list();
    let shelf_covers = covers.clone();
    let shelf = move |from: usize, selected: Option<usize>, focused: bool| {
        let covers = shelf_covers.clone();
        let albums = albums.clone();
        dyn_view(move || {
            let columns = columns_for(width.get());
            let covers = covers.clone();
            let albums = albums.clone();
            untrack(move || {
                card_row(
                    columns,
                    albums
                        .iter()
                        .skip(from)
                        .take(columns)
                        .map(|album| {
                            let id = album.id;
                            card(
                                Card::new(picture(&covers[id % COVERS]), album.title)
                                    .details(album.artist)
                                    .selected_when(move || selected == Some(id))
                                    .focused_when(move || focused),
                            )
                        })
                        .collect(),
                )
                .into_view()
            })
        })
    };
    let carousel_covers = covers.clone();
    let recommended = carousel(COVERS, 200.0, move |ix| {
        picture(&carousel_covers[ix]).size_full()
    })
    .h(200.0_f32);
    let artists = [
        "Kaito Mori",
        "Lumen & Vale",
        "Mina Arai",
        "Odd Harbour",
        "Weatherhouse",
    ];
    let artist_covers = covers.clone();
    let artist_shelf = dyn_view(move || {
        let columns = columns_for(width.get());
        let covers = artist_covers.clone();
        untrack(move || {
            card_row(
                columns,
                artists
                    .iter()
                    .take(columns)
                    .enumerate()
                    .map(|(ix, name)| {
                        card(Card::new(picture(&covers[(ix + 3) % COVERS]), *name).artist_picture())
                    })
                    .collect(),
            )
            .into_view()
        })
    });
    let page = div()
        .flex()
        .flex_col()
        .overflow_y_scroll()
        .on_resized(move |w, _| width.set_if_changed(w))
        .child(section_heading("Recommended"))
        .child(
            div()
                .flex_none()
                .px(16.0_f32)
                .pb(16.0_f32)
                .child(recommended),
        )
        .child(section_heading("Recently Added"))
        .child(shelf(0, Some(1), false))
        .child(section_heading_with(
            "Random",
            styled_icon_button(icons::REFRESH, IconButtonStyle::standard())
                .access(|a| a.label("Reload Random")),
        ))
        .child(shelf(6, Some(8), true))
        .child(section_heading("Artists"))
        .child(artist_shelf)
        .child(section_heading("Recently Played"))
        .child(section_loading("Loading Recently Played"));
    window("Home", Vec::new(), pane(page))
}

fn states() -> Div {
    let column = |top: Div, bottom: Div| {
        div()
            .flex_1()
            .min_w(0)
            .flex()
            .flex_col()
            .gap(GAP)
            .child(pane(top).flex_1())
            .child(pane(bottom).flex_1())
    };
    window(
        "States",
        Vec::new(),
        div()
            .flex()
            .flex_row()
            .gap(GAP)
            .child(column(
                pane_loading("Loading Albums"),
                load_failure(
                    "Albums didn't load",
                    "The server didn't answer. Check that it is running and that this computer can reach it.",
                    || {},
                ),
            ))
            .child(column(
                empty_state(
                    "No playlists",
                    Some("Playlists you make, and those shared with you, show here."),
                    Some(styled_button("New Playlist", ButtonStyle::tonal())),
                ),
                no_matches(
                    "No albums match “blue”",
                    Some("Readings of Japanese titles are still downloading."),
                ),
            )),
    )
}
