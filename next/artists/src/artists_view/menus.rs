//! The menus of the artists and of what is on an artist's page. Those of an artist fetch the
//! artist's songs once an item is chosen.

use std::rc::Rc;

use anti_webui::prelude::untrack;
use anti_webui_md3e::components::{MenuItem, menu_item};
use hiphile_core::favorites::Favorite;
use next_ui::AlbumAction;
use next_ui::menus::{
    Songs, add_to_playlist_item, favorite_item, go_to_artist_item, queue_items,
    show_in_albums_item, song_menu,
};
use playback::Placement;
use subsonic::{Album, Artist, ArtistId, Song};

use super::View;

impl View {
    /// An item that does `action` to the songs of `artist`.
    fn acting(self: &Rc<Self>, label: &str, artist: &ArtistId, action: AlbumAction) -> MenuItem {
        let (view, artist) = (self.clone(), artist.clone());
        menu_item(label).on_click(move || view.act_with(artist.clone(), action.clone()))
    }

    /// Play Next and Add to Queue for the songs of `artist`.
    pub(crate) fn enqueue_items(self: &Rc<Self>, artist: &ArtistId) -> Vec<MenuItem> {
        vec![
            self.acting("Play Next", artist, AlbumAction::Enqueue(Placement::Next)),
            self.acting(
                "Add to Queue",
                artist,
                AlbumAction::Enqueue(Placement::Last),
            ),
        ]
    }

    /// Add to Playlist, which opens New Playlist… and the user's playlists, as they are now.
    pub(crate) fn add_artist_to_playlist_item(self: &Rc<Self>, artist: &ArtistId) -> MenuItem {
        let new = self.acting("New Playlist…", artist, AlbumAction::AddToPlaylist(None));
        let mut groups = vec![vec![new]];
        let choices = untrack(|| self.cx.known.playlist_choices());
        if !choices.is_empty() {
            groups.push(
                choices
                    .iter()
                    .map(|choice| {
                        let to = AlbumAction::AddToPlaylist(Some(choice.id().clone()));
                        self.acting(choice.name(), artist, to)
                    })
                    .collect(),
            );
        }
        menu_item("Add to Playlist").submenu(groups)
    }

    /// The menu of an artist of the list.
    pub(crate) fn artist_menu(self: &Rc<Self>, artist: &ArtistId) -> Vec<Vec<MenuItem>> {
        let mut queue = vec![self.acting("Play", artist, AlbumAction::Play)];
        queue.extend(self.enqueue_items(artist));
        let mut others = vec![self.add_artist_to_playlist_item(artist)];
        if let Some(whole) = self.whole_artist(artist) {
            others.push(favorite_item(
                &self.cx.commands,
                self.cx.known,
                Favorite::Artist(whole),
            ));
        }
        vec![queue, others]
    }

    /// The menu of a similar artist on the page, which goes to them too.
    pub(crate) fn similar_artist_menu(self: &Rc<Self>, artist: &Artist) -> Vec<Vec<MenuItem>> {
        let id = artist.id();
        let mut queue = vec![self.acting("Play", id, AlbumAction::Play)];
        queue.extend(self.enqueue_items(id));
        let mut others = vec![
            self.add_artist_to_playlist_item(id),
            favorite_item(
                &self.cx.commands,
                self.cx.known,
                Favorite::Artist(artist.clone()),
            ),
        ];
        others.extend(go_to_artist_item(&self.cx.commands, Some(id)));
        vec![queue, others]
    }

    /// The menu of an album of the artist on show, whose songs are fetched once an item is
    /// chosen.
    pub(crate) fn album_menu(self: &Rc<Self>, album: &Album) -> Vec<Vec<MenuItem>> {
        let (commands, known) = (&self.cx.commands, self.cx.known);
        let songs = Songs::Album(album.id().clone());
        vec![
            queue_items(commands, known, &songs, None),
            vec![
                add_to_playlist_item(commands, known, &songs),
                favorite_item(commands, known, Favorite::Album(album.clone())),
                show_in_albums_item(commands, album.id()),
            ],
        ]
    }

    /// The menu of one of the top songs, whose Play plays those on show from it.
    pub(crate) fn song_menu(self: &Rc<Self>, song: &Song) -> Vec<Vec<MenuItem>> {
        let play = self
            .top_song(song.id())
            .and_then(|(position, _)| self.play_top_songs_from(position));
        song_menu(&self.cx.commands, self.cx.known, song, play)
    }
}
