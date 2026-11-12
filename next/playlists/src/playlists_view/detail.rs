//! The playlist chosen, on the detail's pane: its header and its songs.

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use anti_webui::prelude::*;
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use hiphile_core::playlists::{Entry, PlaylistDetail};
use next_ui::Command;
use next_ui::context_menu::more_button;
use next_ui::cover::{CoverKind, cover};
use next_ui::detail_header::{DetailHeader, detail_header, play_and_shuffle};
use next_ui::details::{format_duration, format_song_count, join_details};
use next_ui::keymap::{KeyContext, key_context};
use next_ui::load_state::{empty_state, load_failure, pane_loading};
use next_ui::now_playing::song_row_label;
use next_ui::row_list::{RowList, RowSource, row_list};
use playback::Placement;
use subsonic::{PlaylistId, StarTarget};

use super::Playlists;
use super::keys::{self, KeyCommand};
use super::list::playlist_menu;
use super::song_row::{song_menu_of, song_row};

/// What the header's cover is asked for at, in pixels: twice its side, for a screen of two pixels
/// to a point.
const COVER_PIXELS: u32 = 384;

#[derive(Clone, Debug, PartialEq)]
enum Content {
    None,
    Loading,
    Failed(String),
    Loaded(PlaylistId),
}

pub(super) fn pane(screen: &Playlists) -> Div {
    let shown = screen.shown;
    let content = memo(move || {
        shown.with(|shown| match shown.detail() {
            PlaylistDetail::None => Content::None,
            PlaylistDetail::Loading => Content::Loading,
            PlaylistDetail::Failed(error) => Content::Failed(error.clone()),
            PlaylistDetail::Loaded(open) => Content::Loaded(open.playlist().id().clone()),
        })
    });
    let screen = screen.clone();
    div().size_full().flex().flex_col().child(dyn_view(move || {
        let content = content.get();
        let screen = screen.clone();
        untrack(move || match content {
            Content::None => div(),
            Content::Loading => pane_loading("Loading Playlist"),
            Content::Failed(error) => {
                load_failure("Couldn't load this playlist", error, move || {
                    screen.reopen()
                })
            }
            Content::Loaded(_) => div()
                .flex_1()
                .min_h(0)
                .flex()
                .flex_col()
                .child(header(&screen))
                .child(songs(&screen)),
        })
    }))
}

/// What the header shows of the playlist open.
#[derive(Clone, Debug, PartialEq)]
struct Head {
    id: PlaylistId,
    name: String,
    owner: Option<String>,
    comment: Option<String>,
    cover: Option<Arc<str>>,
    summary: String,
    has_songs: bool,
}

fn header(screen: &Playlists) -> Div {
    let (shown, username) = (screen.shown, screen.username.clone());
    let session = screen.cx.workspace.session().clone();
    let head = memo(move || {
        shown.with(|shown| {
            let open = shown.open_playlist()?;
            let playlist = open.playlist();
            let entries = open.entries().shared();
            let editable = shown.is_editable(playlist.id());
            let total: Option<Duration> = entries.iter().map(|entry| entry.song().duration()).sum();
            let total = total.or_else(|| playlist.duration());
            let summary = join_details([
                Some(format_song_count(entries.len())),
                total.filter(|_| !entries.is_empty()).map(format_duration),
                (!editable).then(|| "Readonly".to_owned()),
            ]);
            Some(Head {
                id: playlist.id().clone(),
                name: playlist.name().to_owned(),
                owner: playlist
                    .owner()
                    .filter(|_| !playlist.is_owned_by(&username))
                    .map(str::to_owned),
                comment: playlist.comment().map(str::to_owned),
                cover: playlist
                    .cover_art()
                    .map(|cover_art| session.cover_art_url(cover_art, COVER_PIXELS)),
                summary,
                has_songs: !entries.is_empty(),
            })
        })
    });
    let screen = screen.clone();
    div().flex().flex_col().flex_none().child(dyn_view(move || {
        let head = head.get();
        let screen = screen.clone();
        untrack(move || head.map(|head| head_of(&screen, head)))
    }))
}

fn head_of(screen: &Playlists, head: Head) -> Div {
    let Head {
        id,
        name,
        owner,
        comment,
        cover: url,
        summary,
        has_songs,
    } = head;
    let commands = screen.cx.commands.clone();
    let (playing, shuffling) = (screen.clone(), screen.clone());
    let play = move || {
        if let Some(songs) = playing.open_songs() {
            commands.send(Command::Play { songs, start: None });
        }
    };
    let commands = screen.cx.commands.clone();
    let shuffle = move || {
        if let Some(songs) = shuffling.open_songs() {
            commands.send(Command::Shuffle(songs));
        }
    };
    let menu_of = screen.clone();
    let more = more_button(screen.menu, move || {
        playlist_menu(&menu_of, &id, has_songs, false)
    });
    let header = DetailHeader::new(cover(url, CoverKind::Playlist), name, summary)
        .comment(comment)
        .action(play_and_shuffle(play, shuffle, move || !has_songs))
        .action(more);
    let header = match owner {
        Some(owner) => header.subtitle(div().child(text(owner))),
        None => header,
    };
    detail_header(header)
}

/// What the songs show: a note when there are none, and else whether they can be dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Songs {
    None { is_editable: bool },
    Rows { are_editable: bool },
}

fn songs(screen: &Playlists) -> Div {
    let shown = screen.shown;
    let songs = memo(move || {
        shown.with(|shown| {
            let open = shown.open_playlist()?;
            Some(if open.entries().is_empty() {
                Songs::None {
                    is_editable: shown.is_editable(open.playlist().id()),
                }
            } else {
                Songs::Rows {
                    are_editable: shown.are_songs_editable(),
                }
            })
        })
    });
    let screen = screen.clone();
    div()
        .flex_1()
        .min_h(0)
        .flex()
        .flex_col()
        .child(dyn_view(move || {
            let songs = songs.get();
            let screen = screen.clone();
            untrack(move || {
                songs.map(|songs| match songs {
                    Songs::None { is_editable } => empty_state(
                        "No songs yet",
                        is_editable.then_some("Choose Add to Playlist on any song or album."),
                        None,
                    ),
                    Songs::Rows { are_editable } => song_list(&screen, are_editable),
                })
            })
        }))
}

/// The open playlist's entries, as the song list shows them.
#[derive(Clone, Debug, Default, PartialEq)]
struct EntryRows(Arc<Vec<Entry>>);

impl RowSource for EntryRows {
    type Item = Entry;

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, position: usize) -> Option<&Entry> {
        self.0.get(position)
    }
}

/// The songs, which can be dragged to another place while `are_editable`.
fn song_list(screen: &Playlists, are_editable: bool) -> Div {
    let shown = screen.shown;
    let entries = memo(move || {
        shown.with(|shown| {
            shown
                .open_playlist()
                .map(|open| EntryRows(open.entries().shared()))
                .unwrap_or_default()
        })
    });
    let (now_playing, known) = (screen.now_playing, screen.cx.known);
    let (row_of, playing, menu_of, reordering) = (
        screen.clone(),
        screen.clone(),
        screen.clone(),
        screen.clone(),
    );
    let list = RowList::new(
        "Songs",
        move || entries.get(),
        |entry: &Entry| entry.id(),
        move |entry, row| song_row(&row_of, entry, row.position()),
    )
    .row_label(move |entry| {
        let song = entry.song();
        let state = now_playing.with(|now| now.as_ref().and_then(|now| now.state_of(song.id())));
        song_row_label(song.title(), state, known.is_skipped(song))
    })
    .selection(screen.songs_selection)
    .node_ref(screen.songs_node)
    .on_open(move |entry| playing.play_from(entry))
    .on_context_menu(move |entry, at| {
        if let Some(groups) = song_menu_of(&menu_of, entry) {
            menu_of.menu.open_at(at, groups);
        }
    });
    let list = if are_editable {
        list.on_reorder(move |ids, before| reordering.reorder(ids, before))
    } else {
        list
    };
    let (focused, node) = (screen.songs_focused, screen.songs_node);
    // Made again when the songs can be dragged or no longer can, which takes their focus
    let refocus = Cell::new(untrack(|| focused.get()));
    let list = row_list(list)
        .flex_1()
        .min_h(0)
        .on_focus_change(move |on| focused.set(on))
        .on_resized(move |_, _| {
            if refocus.replace(false) {
                node.focus();
            }
        });
    key_context(
        div().flex_1().min_h(0).flex().flex_col().child(list),
        &songs_context(screen),
    )
}

fn songs_context(screen: &Playlists) -> KeyContext<KeyCommand> {
    let (asked, running) = (screen.clone(), screen.clone());
    KeyContext::new(
        keys::SONGS,
        move |command| {
            if *command == KeyCommand::Back {
                return true;
            }
            let Some(song) = asked.cursor_song() else {
                return false;
            };
            match command {
                KeyCommand::MoveUp | KeyCommand::MoveDown | KeyCommand::Remove => {
                    asked.shown.with(|shown| shown.are_songs_editable())
                }
                KeyCommand::PlayNext | KeyCommand::AddToQueue | KeyCommand::AddToPlaylist => true,
                KeyCommand::ToggleFavorite => {
                    let target = StarTarget::Song(song.id().clone());
                    asked.cx.known.is_favorite(&target).is_some()
                }
                KeyCommand::GoToAlbum => song.album_id().is_some(),
                KeyCommand::GoToArtist => song.artist_id().is_some(),
                _ => false,
            }
        },
        move |command| {
            let screen = &running;
            if command == KeyCommand::Back {
                screen.list_node.focus();
                return;
            }
            let Some(entry) = screen.songs_selection.cursor() else {
                return;
            };
            let Some(song) = untrack(|| screen.cursor_song()) else {
                return;
            };
            let commands = &screen.cx.commands;
            let enqueue = |placement| Command::Enqueue {
                songs: Arc::from([song.clone()]),
                placement,
            };
            match command {
                KeyCommand::MoveUp => screen.move_entry(entry, -1),
                KeyCommand::MoveDown => screen.move_entry(entry, 1),
                KeyCommand::Remove => screen.remove(entry),
                KeyCommand::PlayNext => commands.send(enqueue(Placement::Next)),
                KeyCommand::AddToQueue => commands.send(enqueue(Placement::Last)),
                KeyCommand::ToggleFavorite => {
                    commands.send(Command::ToggleFavorite(Favorite::Song(song.clone())));
                }
                KeyCommand::GoToAlbum => {
                    if let Some(album) = song.album_id() {
                        commands.send(Command::Show(Location::Albums(Some(album.clone()))));
                    }
                }
                KeyCommand::GoToArtist => {
                    if let Some(artist) = song.artist_id() {
                        commands.send(Command::Show(Location::Artists(Some(artist.clone()))));
                    }
                }
                KeyCommand::AddToPlaylist => {
                    commands.send(Command::ChoosePlaylist(Arc::from([song.clone()])));
                }
                _ => {}
            }
        },
    )
}
