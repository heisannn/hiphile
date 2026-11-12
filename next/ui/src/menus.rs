//! The items of the menus on songs, albums and artists, the same on every screen. Each sends a
//! [`Command`]. A menu's items say what they do when they are made, so make them as the menu
//! opens, as [`ContextMenu`](crate::context_menu::ContextMenu) does.

use std::sync::Arc;

use anti_webui::prelude::untrack;
use anti_webui_md3e::components::{MenuItem, menu_item};
use hiphile_core::favorites::Favorite;
use hiphile_core::navigation::Location;
use playback::Placement;
use subsonic::{Album, AlbumId, Artist, ArtistId, Song};

use crate::favorite_button::favorite_label;
use crate::{AlbumAction, Command, Commands, Known};

/// The songs a menu acts on.
#[derive(Clone, Debug, PartialEq)]
pub enum Songs {
    List(Arc<[Song]>),
    /// An album's, which are fetched when an item is chosen.
    Album(AlbumId),
}

impl Songs {
    /// What does `action` to these songs. `None` when there are none.
    pub fn command(&self, action: AlbumAction) -> Option<Command> {
        match self {
            Self::List(songs) => action.command(songs.clone()),
            Self::Album(album) => Some(Command::ActOnAlbum {
                album: album.clone(),
                action,
            }),
        }
    }

    /// Whether playing them would pass over every one: none are left once instrumentals are
    /// skipped.
    fn are_skipped(&self, known: Known) -> bool {
        match self {
            Self::List(songs) => songs.iter().all(|song| known.is_skipped(song)),
            Self::Album(_) => false,
        }
    }
}

/// An item that sends `command`, disabled while there is none.
fn sending(label: &str, command: Option<Command>, commands: &Commands) -> MenuItem {
    let item = menu_item(label);
    match command {
        Some(command) => {
            let commands = commands.clone();
            item.on_click(move || commands.send(command.clone()))
        }
        None => item.disabled_when(|| true),
    }
}

/// Play, Play Next and Add to Queue. `play` is what Play does, such as playing the list from the
/// song the menu is on; without it, Play plays `songs`. They are disabled while every song would
/// be skipped as an instrumental.
pub fn queue_items(
    commands: &Commands,
    known: Known,
    songs: &Songs,
    play: Option<Command>,
) -> Vec<MenuItem> {
    let play = play.or_else(|| songs.command(AlbumAction::Play));
    [
        ("Play", play),
        (
            "Play Next",
            songs.command(AlbumAction::Enqueue(Placement::Next)),
        ),
        (
            "Add to Queue",
            songs.command(AlbumAction::Enqueue(Placement::Last)),
        ),
    ]
    .into_iter()
    .map(|(label, command)| {
        let none = command.is_none();
        let songs = songs.clone();
        sending(label, command, commands).disabled_when(move || none || songs.are_skipped(known))
    })
    .collect()
}

/// Add to Playlist, which opens New Playlist… and the user's playlists, as they are now.
pub fn add_to_playlist_item(commands: &Commands, known: Known, songs: &Songs) -> MenuItem {
    let item = menu_item("Add to Playlist");
    let Some(new) = songs.command(AlbumAction::AddToPlaylist(None)) else {
        return item.disabled_when(|| true);
    };
    let mut groups = vec![vec![sending("New Playlist…", Some(new), commands)]];
    let choices = untrack(|| known.playlist_choices());
    if !choices.is_empty() {
        groups.push(
            choices
                .iter()
                .map(|choice| {
                    let add = songs.command(AlbumAction::AddToPlaylist(Some(choice.id().clone())));
                    sending(choice.name(), add, commands)
                })
                .collect(),
        );
    }
    item.submenu(groups)
}

/// Add to Favorites or Remove from Favorites, by whether `favorite` is one now. It is disabled
/// until the favorites are known.
pub fn favorite_item(commands: &Commands, known: Known, favorite: Favorite) -> MenuItem {
    let target = favorite.target();
    let label = favorite_label(untrack(|| known.is_favorite(&target)));
    sending(label, Some(Command::ToggleFavorite(favorite)), commands)
        .disabled_when(move || known.is_favorite(&target).is_none())
}

/// Go to Album, for a song on an album the server names.
pub fn go_to_album_item(commands: &Commands, album: Option<&AlbumId>) -> Option<MenuItem> {
    let to = Location::Albums(Some(album?.clone()));
    Some(sending("Go to Album", Some(Command::Show(to)), commands))
}

/// Show in Albums, for an album shown outside Albums.
pub fn show_in_albums_item(commands: &Commands, album: &AlbumId) -> MenuItem {
    let to = Location::Albums(Some(album.clone()));
    sending("Show in Albums", Some(Command::Show(to)), commands)
}

/// Go to Artist, for what is by an artist the server names.
pub fn go_to_artist_item(commands: &Commands, artist: Option<&ArtistId>) -> Option<MenuItem> {
    let to = Location::Artists(Some(artist?.clone()));
    Some(sending("Go to Artist", Some(Command::Show(to)), commands))
}

/// The menu of `song`: what plays it, then what files it and where it goes. `play` is what Play
/// does, as for [`queue_items`].
pub fn song_menu(
    commands: &Commands,
    known: Known,
    song: &Song,
    play: Option<Command>,
) -> Vec<Vec<MenuItem>> {
    let songs = Songs::List(Arc::from([song.clone()]));
    let mut others = vec![
        add_to_playlist_item(commands, known, &songs),
        favorite_item(commands, known, Favorite::Song(song.clone())),
    ];
    others.extend(go_to_album_item(commands, song.album_id()));
    others.extend(go_to_artist_item(commands, song.artist_id()));
    vec![queue_items(commands, known, &songs, play), others]
}

/// The menu of `album`, whose songs are fetched when an item is chosen. A screen other than
/// Albums adds [`show_in_albums_item`] to its last group.
pub fn album_menu(commands: &Commands, known: Known, album: &Album) -> Vec<Vec<MenuItem>> {
    let songs = Songs::Album(album.id().clone());
    let mut others = vec![
        add_to_playlist_item(commands, known, &songs),
        favorite_item(commands, known, Favorite::Album(album.clone())),
    ];
    others.extend(go_to_artist_item(commands, album.artist_id()));
    vec![queue_items(commands, known, &songs, None), others]
}

/// The menu of `artist` outside Artists, which goes there. Artists makes its own, as it has the
/// artist's songs at hand.
pub fn artist_menu(commands: &Commands, known: Known, artist: &Artist) -> Vec<Vec<MenuItem>> {
    let go = go_to_artist_item(commands, Some(artist.id()));
    vec![
        go.into_iter().collect(),
        vec![favorite_item(
            commands,
            known,
            Favorite::Artist(artist.clone()),
        )],
    ]
}

#[cfg(test)]
mod tests;
