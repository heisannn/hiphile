use std::sync::Arc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::components::MenuItem;
use hiphile_core::favorites::Favorite;
use hiphile_core::library::{AlbumDetailSnapshot, LoadedAlbum};
use next_ui::Command;
use next_ui::context_menu::more_button;
use next_ui::cover::{CoverKind, cover};
use next_ui::detail_header::{DetailHeader, detail_header, play_and_shuffle};
use next_ui::details::{format_duration, format_song_count, join_details};
use next_ui::favorite_button::favorite_button;
use next_ui::keymap::{KeyContext, key_context};
use next_ui::link::{artist_line, artist_link, song_artist_link};
use next_ui::load_state::{self, ALBUM, empty_state, load_failure, pane_loading};
use next_ui::menus::{Songs, add_to_playlist_item, favorite_item, go_to_artist_item, queue_items};
use next_ui::now_playing::song_row_label;
use next_ui::row_buttons::RowButtons;
use next_ui::row_list::{RowList, RowState, row_list};
use next_ui::song_row::{SongRow, song_words};
use subsonic::{Album, AlbumId, ArtistId, CoverArtId, Song, SongId};

use super::keys::SONGS;
use super::{Model, SONG_ROW_HEIGHT};

/// What the cover in the head is asked for at: as many pixels as it shows at its largest on a
/// screen of two to a point.
const COVER_PIXELS: u32 = 384;
/// What the detail shows of the album on show. A loaded album is known by its id alone: another
/// version of it, such as the server's after the saved one, changes what shows in place, which
/// keeps the focus, the cursor and the scroll in its songs.
#[derive(Clone, Debug, PartialEq)]
enum Standing {
    None,
    Loading,
    Failed(String),
    Loaded(AlbumId),
}

impl Standing {
    fn of(album: &AlbumDetailSnapshot) -> Self {
        match album {
            AlbumDetailSnapshot::None => Self::None,
            AlbumDetailSnapshot::Loading(_) => Self::Loading,
            AlbumDetailSnapshot::Failed { error, .. } => Self::Failed(error.clone()),
            AlbumDetailSnapshot::Loaded(loaded) => Self::Loaded(loaded.album().id().clone()),
        }
    }
}

/// The album on show: its head, and its songs.
pub(super) fn album_detail(model: &Model) -> Div {
    let album = model.album;
    let standing = memo(move || album.with(Standing::of));
    let loaded = memo(move || album.with(|album| album.loaded().cloned()));
    let model = model.clone();
    div().flex().flex_col().child(dyn_view(move || {
        let model = model.clone();
        let standing = standing.get();
        untrack(|| match standing {
            Standing::None => div().into_view(),
            Standing::Loading => pane_loading(ALBUM.loading()).into_view(),
            Standing::Failed(reason) => {
                let retry = model.clone();
                load_failure(ALBUM.load_failed(), reason, move || {
                    if let Some(album) = untrack(|| retry.selected()) {
                        retry.detail.show(album);
                    }
                })
                .into_view()
            }
            Standing::Loaded(_) => loaded_album(&model, loaded).into_view(),
        })
    }))
}

/// The album that `loaded` holds, which follows it.
fn loaded_album(model: &Model, loaded: Memo<Option<LoadedAlbum>>) -> Div {
    let songs = memo(move || {
        loaded.with(|loaded| {
            loaded
                .as_ref()
                .map(|loaded| loaded.songs().clone())
                .unwrap_or_default()
        })
    });
    let is_empty = memo(move || songs.with(|songs| songs.is_empty()));
    let listed = model.clone();
    let list = dyn_view(move || {
        if is_empty.get() {
            return untrack(|| empty_state(&load_state::SONGS.none(), None, None));
        }
        let model = listed.clone();
        untrack(|| {
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
                    .child(row_list(song_rows(&model, loaded, songs)).flex_1().min_h(0)),
                &context,
            )
        })
    });
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(header(model, loaded, songs))
        .child(div().flex().flex_col().flex_1().min_h(0).child(list))
}

/// What the head shows of the album, which it is made again for when that changes.
#[derive(Clone, Debug, PartialEq)]
struct Head {
    name: String,
    artist: Option<String>,
    artist_id: Option<ArtistId>,
    cover: Option<CoverArtId>,
    summary: String,
}

impl Head {
    fn of(loaded: &LoadedAlbum) -> Self {
        let album = loaded.album();
        Self {
            name: album.name().to_owned(),
            artist: album.artist().map(str::to_owned),
            artist_id: album.artist_id().cloned(),
            cover: album.cover_art().cloned(),
            summary: join_details([
                album.year().map(|year| year.to_string()),
                Some(format_song_count(loaded.songs().len())),
                loaded.duration().map(format_duration),
            ]),
        }
    }
}

fn header(model: &Model, loaded: Memo<Option<LoadedAlbum>>, songs: Memo<Arc<[Song]>>) -> View {
    let head = memo(move || loaded.with(|loaded| loaded.as_ref().map(Head::of)));
    let model = model.clone();
    dyn_view(move || {
        let head = head.get();
        let model = model.clone();
        untrack(move || {
            let album = loaded.with(|loaded| loaded.as_ref().map(|loaded| loaded.album().clone()));
            match (head, album) {
                (Some(head), Some(album)) => head_of(&model, &album, &head, songs),
                _ => div(),
            }
        })
    })
}

/// The head of `album` as `head` shows it. Its buttons act on the songs as they are when pressed.
fn head_of(model: &Model, album: &Album, head: &Head, songs: Memo<Arc<[Song]>>) -> Div {
    let (commands, known) = (model.cx.commands.clone(), model.cx.known);
    let url = head.cover.as_ref().map(|cover| {
        model
            .cx
            .workspace
            .session()
            .cover_art_url(cover, COVER_PIXELS)
    });
    let play = {
        let commands = commands.clone();
        move || {
            commands.send(Command::Play {
                songs: untrack(|| songs.get()),
                start: None,
            });
        }
    };
    let shuffle = {
        let commands = commands.clone();
        move || commands.send(Command::Shuffle(untrack(|| songs.get())))
    };
    let nothing_to_play =
        move || songs.with(|songs| songs.iter().all(|song| known.is_skipped(song)));
    let menu = {
        let (commands, album) = (commands.clone(), album.clone());
        move || {
            let listed = Songs::List(untrack(|| songs.get()));
            let mut others = vec![
                add_to_playlist_item(&commands, known, &listed),
                favorite_item(&commands, known, Favorite::Album(album.clone())),
            ];
            others.extend(go_to_artist_item(&commands, album.artist_id()));
            vec![queue_items(&commands, known, &listed, None), others]
        }
    };
    let header = DetailHeader::new(
        cover(url, CoverKind::Album),
        head.name.clone(),
        head.summary.clone(),
    )
    .action(play_and_shuffle(play, shuffle, nothing_to_play))
    .action(favorite_button(
        Favorite::Album(album.clone()),
        &commands,
        known,
    ))
    .action(more_button(model.menu, menu));
    let artist = head
        .artist
        .clone()
        .map(|name| artist_link(name, head.artist_id.clone(), &commands));
    let header = match artist {
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

fn song_rows(
    model: &Model,
    loaded: Memo<Option<LoadedAlbum>>,
    songs: Memo<Arc<[Song]>>,
) -> RowList<Arc<[Song]>, SongId> {
    let (known, now_playing) = (model.cx.known, model.now_playing);
    let (look, opened, menu) = (model.clone(), model.clone(), model.clone());
    RowList::new(
        "Songs",
        move || songs.get(),
        |song: &Song| song.id().clone(),
        move |song, row| {
            let is_by_another_artist = untrack(|| {
                loaded.with(|loaded| {
                    loaded
                        .as_ref()
                        .is_some_and(|loaded| loaded.is_by_another_artist(song))
                })
            });
            song_row(&look, songs, song, is_by_another_artist, row)
        },
    )
    .row_height(SONG_ROW_HEIGHT)
    .row_label(move |song| {
        let state = now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(song.id())));
        song_row_label(song.title(), state, known.is_skipped(song))
    })
    .selection(model.songs)
    .node_ref(model.song_rows)
    .on_open(move |id| {
        let songs = untrack(|| songs.get());
        if let Some(at) = songs.iter().position(|song| song.id() == &id) {
            opened.cx.commands.send(Command::Play {
                songs,
                start: Some(at),
            });
        }
    })
    .on_context_menu(move |id, at_point| {
        let songs = untrack(|| songs.get());
        if let Some(at) = songs.iter().position(|song| song.id() == &id) {
            let groups = song_menu(&menu, &songs, at);
            menu.menu.open_at(at_point, groups);
        }
    })
}

/// The number, the title and, on a compilation, the artist, then whether it plays, how long it
/// lasts, and its buttons.
fn song_row(
    model: &Model,
    songs: Memo<Arc<[Song]>>,
    song: &Song,
    is_by_another_artist: bool,
    row: RowState<SongId>,
) -> Div {
    let (commands, known, now_playing) =
        (model.cx.commands.clone(), model.cx.known, model.now_playing);
    let id = song.id().clone();
    let state = move || now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(&id)));
    let artist = is_by_another_artist
        .then(|| song_artist_link(song, &commands))
        .flatten()
        .map(|artist| artist_line(Some(artist), None));
    let menu = {
        let model = model.clone();
        let song_id = song.id().clone();
        move || {
            let songs = untrack(|| songs.get());
            let at = songs.iter().position(|song| song.id() == &song_id);
            at.map(|at| song_menu(&model, &songs, at))
                .unwrap_or_default()
        }
    };
    let buttons = RowButtons::new(
        Favorite::Song(song.clone()),
        &commands,
        known,
        model.menu,
        menu,
    )
    .selected_when(move || row.is_selected());
    let skipped = song.clone();
    next_ui::song_row::song_row(
        SongRow::new(song, song_words(song, artist), state)
            .number(song.track())
            .skipped_when(move || known.is_skipped(&skipped))
            .buttons(buttons),
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
