use std::collections::VecDeque;

use subsonic::{PlaylistId, SongId};

#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    ReplaceSongs {
        playlist: PlaylistId,
        songs: Vec<SongId>,
    },
    AddSongs {
        playlist: PlaylistId,
        songs: Vec<SongId>,
    },
    /// Puts a song back in a playlist that is not on show, whose songs have
    /// to be read first.
    InsertSong {
        playlist: PlaylistId,
        ix: usize,
        song: SongId,
    },
    Rename {
        playlist: PlaylistId,
        name: String,
    },
    Delete {
        playlist: PlaylistId,
    },
}

impl Write {
    pub fn playlist(&self) -> &PlaylistId {
        match self {
            Self::ReplaceSongs { playlist, .. }
            | Self::AddSongs { playlist, .. }
            | Self::InsertSong { playlist, .. }
            | Self::Rename { playlist, .. }
            | Self::Delete { playlist } => playlist,
        }
    }

    pub async fn send(self, client: client::Client) -> Result<(), client::Error> {
        match self {
            Self::ReplaceSongs { playlist, songs } => {
                client.replace_playlist_songs(&playlist, &songs).await
            }
            Self::AddSongs { playlist, songs } => client.add_to_playlist(&playlist, &songs).await,
            Self::InsertSong { playlist, ix, song } => {
                let loaded = client.playlist(&playlist).await?;
                let mut songs: Vec<SongId> = loaded
                    .songs()
                    .iter()
                    .map(|song| song.id().clone())
                    .collect();
                songs.insert(ix.min(songs.len()), song);
                client.replace_playlist_songs(&playlist, &songs).await
            }
            Self::Rename { playlist, name } => client.rename_playlist(&playlist, &name).await,
            Self::Delete { playlist } => client.delete_playlist(&playlist).await,
        }
    }

    fn changes_songs(&self) -> bool {
        match self {
            Self::ReplaceSongs { .. } | Self::AddSongs { .. } | Self::InsertSong { .. } => true,
            Self::Rename { .. } | Self::Delete { .. } => false,
        }
    }

    /// What couldn't be done, to go before the reason.
    pub fn failure(&self) -> &'static str {
        match self {
            Self::ReplaceSongs { .. } | Self::InsertSong { .. } => {
                "Couldn't save the playlist's songs."
            }
            Self::AddSongs { .. } => "Couldn't add to the playlist.",
            Self::Rename { .. } => "Couldn't rename the playlist.",
            Self::Delete { .. } => "Couldn't delete the playlist.",
        }
    }
}

/// Writes waiting their turn. They go one at a time, in order: a later change
/// overtaking an earlier one on the network would leave the older on the
/// server.
#[derive(Default)]
pub struct WriteQueue(VecDeque<Write>);

impl WriteQueue {
    /// Adds `write`. Saving a playlist's whole song list makes one waiting
    /// right before it pointless, so moving a song many places in a row saves
    /// only where it ends up.
    pub fn push(&mut self, write: Write) {
        if let (
            Write::ReplaceSongs { playlist, .. },
            Some(Write::ReplaceSongs {
                playlist: waiting, ..
            }),
        ) = (&write, self.0.back())
            && playlist == waiting
        {
            self.0.pop_back();
        }
        self.0.push_back(write);
    }

    pub fn pop(&mut self) -> Option<Write> {
        self.0.pop_front()
    }

    pub fn contains(&self, playlist: &PlaylistId) -> bool {
        self.0.iter().any(|write| write.playlist() == playlist)
    }

    /// Drops the changes to `playlist`'s songs that wait, after a write to it
    /// failed: they were made from songs the server may not have. A rename or
    /// a deletion still goes.
    pub fn forget_songs(&mut self, playlist: &PlaylistId) {
        self.0
            .retain(|write| write.playlist() != playlist || !write.changes_songs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(playlist: &str, songs: &[&str]) -> Write {
        Write::ReplaceSongs {
            playlist: PlaylistId::new(playlist),
            songs: songs.iter().map(|song| SongId::new(*song)).collect(),
        }
    }

    fn drain(queue: &mut WriteQueue) -> Vec<Write> {
        std::iter::from_fn(|| queue.pop()).collect()
    }

    #[test]
    fn a_new_song_list_replaces_the_one_waiting_for_the_same_playlist() {
        let mut queue = WriteQueue::default();
        queue.push(replace("p", &["a", "b"]));
        queue.push(replace("p", &["b", "a"]));
        assert_eq!(drain(&mut queue), [replace("p", &["b", "a"])]);
    }

    #[test]
    fn song_lists_of_different_playlists_both_go() {
        let mut queue = WriteQueue::default();
        queue.push(replace("p", &["a"]));
        queue.push(replace("q", &["a"]));
        assert_eq!(
            drain(&mut queue),
            [replace("p", &["a"]), replace("q", &["a"])]
        );
    }

    #[test]
    fn a_song_list_never_overtakes_a_different_write_in_between() {
        let rename = Write::Rename {
            playlist: PlaylistId::new("p"),
            name: "New".to_owned(),
        };
        let mut queue = WriteQueue::default();
        queue.push(replace("p", &["a"]));
        queue.push(rename.clone());
        queue.push(replace("p", &["b"]));
        assert_eq!(
            drain(&mut queue),
            [replace("p", &["a"]), rename, replace("p", &["b"])]
        );
    }

    #[test]
    fn forgetting_a_playlists_songs_leaves_the_others_writes() {
        let mut queue = WriteQueue::default();
        queue.push(replace("p", &["a"]));
        queue.push(replace("q", &["a"]));
        queue.forget_songs(&PlaylistId::new("p"));
        assert_eq!(drain(&mut queue), [replace("q", &["a"])]);
    }

    #[test]
    fn forgetting_a_playlists_songs_leaves_its_rename_and_deletion() {
        let playlist = PlaylistId::new("p");
        let rename = Write::Rename {
            playlist: playlist.clone(),
            name: "New".to_owned(),
        };
        let delete = Write::Delete {
            playlist: playlist.clone(),
        };
        let mut queue = WriteQueue::default();
        queue.push(rename.clone());
        queue.push(replace("p", &["a"]));
        queue.push(delete.clone());
        queue.forget_songs(&playlist);
        assert_eq!(drain(&mut queue), [rename, delete]);
    }
}
