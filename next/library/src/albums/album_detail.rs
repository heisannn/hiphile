use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::MenuItem;
use anti_webui_md3e::typography::TextStyle;
use hiphile_core::favorites::Favorite;
use hiphile_core::library::{AlbumDetailSnapshot, LoadedAlbum};
use next_ui::Command;
use next_ui::context_menu::more_button;
use next_ui::cover::{CoverKind, cover};
use next_ui::detail_header::{DetailHeader, detail_header, play_and_shuffle};
use next_ui::details::{format_duration, format_song_count, join_details};
use next_ui::favorite_button::favorite_button;
use next_ui::keymap::{KeyContext, key_context};
use next_ui::link::{album_artist_link, artist_line, song_artist_link};
use next_ui::load_state::{empty_state, load_failure, pane_loading};
use next_ui::menus::{Songs, add_to_playlist_item, favorite_item, go_to_artist_item, queue_items};
use next_ui::now_playing::{play_state_icon, skippable, song_row_label};
use next_ui::row_list::{ROW_PADDING, RowList, row_list};
use next_ui::row_text::row_text;
use next_ui::type_scale::muted_text;
use subsonic::{Song, SongId};

use super::keys::SONGS;
use super::{Model, SONG_ROW_HEIGHT};

/// What the cover in the head is asked for at: as many pixels as it shows at its largest on a
/// screen of two to a point.
const COVER_PIXELS: u32 = 384;
/// The lane of a song's number, or of the icon of the song playing, as wide as three figures.
const NUMBER_LANE: f32 = 24.0;
/// The lane of a song's length, which keeps the buttons before it in line.
const DURATION_LANE: f32 = 48.0;
/// Between a row's lanes.
const LANE_GAP: f32 = 12.0;
/// What a small icon button has between its edge and its icon, which brings the last of a row
/// closer to the edge than words for its icon to line up with theirs.
const BUTTON_PADDING: f32 = 8.0;

/// What the detail shows of the album on show.
#[derive(Clone, Debug, PartialEq)]
enum Standing {
    None,
    Loading,
    Failed(String),
    Loaded(LoadedAlbum),
}

impl Standing {
    fn of(album: &AlbumDetailSnapshot) -> Self {
        match album {
            AlbumDetailSnapshot::None => Self::None,
            AlbumDetailSnapshot::Loading(_) => Self::Loading,
            AlbumDetailSnapshot::Failed { error, .. } => Self::Failed(error.clone()),
            AlbumDetailSnapshot::Loaded(loaded) => Self::Loaded(loaded.clone()),
        }
    }
}

/// The album on show: its head, and its songs.
pub(super) fn album_detail(model: &Model) -> Div {
    let album = model.album;
    let standing = memo(move || album.with(Standing::of));
    let model = model.clone();
    div().flex().flex_col().child(dyn_view(move || {
        let model = model.clone();
        let standing = standing.get();
        untrack(|| match standing {
            Standing::None => div().into_view(),
            Standing::Loading => pane_loading("Loading Album").into_view(),
            Standing::Failed(reason) => {
                let retry = model.clone();
                load_failure("Couldn't load this album", reason, move || {
                    if let Some(album) = untrack(|| retry.selected()) {
                        retry.detail.show(album);
                    }
                })
                .into_view()
            }
            Standing::Loaded(loaded) => loaded_album(&model, loaded).into_view(),
        })
    }))
}

fn loaded_album(model: &Model, loaded: LoadedAlbum) -> Div {
    let songs = loaded.songs().clone();
    let list = if songs.is_empty() {
        empty_state("This album has no songs.", None, None)
    } else {
        let context = {
            let (model, asked) = (model.clone(), model.clone());
            KeyContext::new(
                SONGS,
                move |key| asked.is_enabled_on_songs(*key),
                move |key| model.take_song_key(key),
            )
        };
        key_context(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(0)
                .child(row_list(song_rows(model, &loaded)).flex_1().min_h(0)),
            &context,
        )
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(header(model, &loaded))
        .child(list)
}

fn header(model: &Model, loaded: &LoadedAlbum) -> Div {
    let album = loaded.album();
    let songs = loaded.songs().clone();
    let (commands, known) = (model.cx.commands.clone(), model.cx.known);
    let url = album.cover_art().map(|cover| {
        model
            .cx
            .workspace
            .session()
            .cover_art_url(cover, COVER_PIXELS)
    });
    let summary = join_details([
        album.year().map(|year| year.to_string()),
        Some(format_song_count(songs.len())),
        loaded.duration().map(format_duration),
    ]);
    let play = {
        let (commands, songs) = (commands.clone(), songs.clone());
        move || {
            commands.send(Command::Play {
                songs: songs.clone(),
                start: None,
            });
        }
    };
    let shuffle = {
        let (commands, songs) = (commands.clone(), songs.clone());
        move || commands.send(Command::Shuffle(songs.clone()))
    };
    let nothing_to_play = {
        let songs = songs.clone();
        move || songs.iter().all(|song| known.is_skipped(song))
    };
    let menu = {
        let (commands, songs, album) = (commands.clone(), songs.clone(), album.clone());
        move || {
            let listed = Songs::List(songs.clone());
            let mut others = vec![
                add_to_playlist_item(&commands, known, &listed),
                favorite_item(&commands, known, Favorite::Album(album.clone())),
            ];
            others.extend(go_to_artist_item(&commands, album.artist_id()));
            vec![queue_items(&commands, known, &listed, None), others]
        }
    };
    let header = DetailHeader::new(cover(url, CoverKind::Album), album.name(), summary)
        .action(play_and_shuffle(play, shuffle, nothing_to_play))
        .action(favorite_button(
            Favorite::Album(album.clone()),
            &commands,
            known,
        ))
        .action(more_button(model.menu, menu));
    let header = match album_artist_link(album, &commands) {
        // Quiet, as the details are, until the pointer is on it
        Some(artist) => header.subtitle(
            div()
                .flex()
                .flex_row()
                .min_w(0)
                .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
                .child(artist),
        ),
        None => header,
    };
    detail_header(header)
}

fn song_rows(model: &Model, loaded: &LoadedAlbum) -> RowList<Arc<[Song]>, SongId> {
    let songs = loaded.songs().clone();
    let (known, now_playing) = (model.cx.known, model.now_playing);
    let look = (model.clone(), loaded.clone());
    let (opened, menu) = (model.clone(), model.clone());
    let (to_open, to_menu) = (songs.clone(), songs.clone());
    RowList::new(
        "Songs",
        move || songs.clone(),
        |song: &Song| song.id().clone(),
        move |song, _| song_row(&look.0, &look.1, song),
    )
    .row_height(SONG_ROW_HEIGHT)
    .row_label(move |song| {
        let state = now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(song.id())));
        song_row_label(song.title(), state, known.is_skipped(song))
    })
    .selection(model.songs)
    .node_ref(model.song_rows)
    .on_open(move |id| {
        if let Some(at) = to_open.iter().position(|song| song.id() == &id) {
            opened.cx.commands.send(Command::Play {
                songs: to_open.clone(),
                start: Some(at),
            });
        }
    })
    .on_context_menu(move |id, at_point| {
        if let Some(at) = to_menu.iter().position(|song| song.id() == &id) {
            let groups = song_menu(&menu, &to_menu, at);
            menu.menu.open_at(at_point, groups);
        }
    })
}

/// The number, or what plays, the title and, on a compilation, the artist, then whether it is a
/// favorite, how long it lasts and what else there is to do with it. A song played past is faded.
fn song_row(model: &Model, loaded: &LoadedAlbum, song: &Song) -> Div {
    let (commands, known, now_playing) =
        (model.cx.commands.clone(), model.cx.known, model.now_playing);
    let id = song.id().clone();
    let state =
        memo(move || now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(&id))));
    let number = song
        .track()
        .map(|track| track.to_string())
        .unwrap_or_default();
    let lane = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .justify_end()
        .w(NUMBER_LANE)
        .child(dyn_view(move || {
            let number = number.clone();
            if state.get().is_some() {
                untrack(|| play_state_icon(move || state.get()).into_view())
            } else {
                untrack(|| muted_text(TextStyle::BODY_MEDIUM, number).into_view())
            }
        }));
    let artist = loaded
        .is_by_another_artist(song)
        .then(|| song_artist_link(song, &commands))
        .flatten()
        .map(|artist| artist_line(Some(artist), None));
    let words = row_text(song.title().to_owned(), None)
        .flex_1()
        .children(artist);
    let duration = muted_text(
        TextStyle::BODY_MEDIUM,
        song.duration().map(format_duration).unwrap_or_default(),
    )
    .flex_none()
    .w(DURATION_LANE)
    .text_align(TextAlign::Right);
    let menu = {
        let model = model.clone();
        let (songs, song_id) = (loaded.songs().clone(), song.id().clone());
        move || {
            let at = songs.iter().position(|song| song.id() == &song_id);
            at.map(|at| song_menu(&model, &songs, at))
                .unwrap_or_default()
        }
    };
    let skipped = song.clone();
    skippable(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(LANE_GAP)
            .pl(ROW_PADDING)
            .pr(ROW_PADDING - BUTTON_PADDING)
            .size_full()
            .child(lane)
            .child(words)
            .child(favorite_button(
                Favorite::Song(song.clone()),
                &commands,
                known,
            ))
            .child(duration)
            .child(more_button(model.menu, menu)),
        move || known.is_skipped(&skipped),
    )
}

/// What plays the album from the song at `at`, then what files it and where it goes.
fn song_menu(model: &Model, songs: &Arc<[Song]>, at: usize) -> Vec<Vec<MenuItem>> {
    let (commands, known) = (&model.cx.commands, model.cx.known);
    let song = songs[at].clone();
    let one = Songs::List(Arc::from([song.clone()]));
    let play = Command::Play {
        songs: songs.clone(),
        start: Some(at),
    };
    let mut others = vec![
        add_to_playlist_item(commands, known, &one),
        favorite_item(commands, known, Favorite::Song(song.clone())),
    ];
    others.extend(go_to_artist_item(commands, song.artist_id()));
    vec![queue_items(commands, known, &one, Some(play)), others]
}
