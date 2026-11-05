use std::time::Duration;

use serde::Deserialize;

use crate::{CoverArtId, PlaylistId, Song};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    id: PlaylistId,
    name: String,
    comment: Option<String>,
    owner: Option<String>,
    song_count: Option<u32>,
    /// Seconds.
    duration: Option<u64>,
    cover_art: Option<CoverArtId>,
    readonly: Option<bool>,
}

impl Playlist {
    pub fn id(&self) -> &PlaylistId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref().filter(|text| !text.is_empty())
    }

    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    pub fn song_count(&self) -> Option<u32> {
        self.song_count
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration.map(Duration::from_secs)
    }

    pub fn cover_art(&self) -> Option<&CoverArtId> {
        self.cover_art.as_ref()
    }

    /// Servers match user names without regard to case, so they can be signed in to with
    /// one spelled differently.
    pub fn is_owned_by(&self, username: &str) -> bool {
        self.owner
            .as_deref()
            .is_some_and(|owner| owner.eq_ignore_ascii_case(username))
    }

    /// Only OpenSubsonic servers say; on others only the owner can, and one naming no owner
    /// is left to refuse.
    pub fn is_editable_by(&self, username: &str) -> bool {
        match self.readonly {
            Some(readonly) => !readonly,
            None => self.owner.is_none() || self.is_owned_by(username),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PlaylistWithSongs {
    #[serde(flatten)]
    playlist: Playlist,
    #[serde(default, rename = "entry")]
    songs: Vec<Song>,
}

impl PlaylistWithSongs {
    pub fn playlist(&self) -> &Playlist {
        &self.playlist
    }

    pub fn songs(&self) -> &[Song] {
        &self.songs
    }

    pub fn into_parts(self) -> (Playlist, Vec<Song>) {
        (self.playlist, self.songs)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn playlist(value: serde_json::Value) -> Playlist {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_server_saying_readonly_decides_who_can_edit() {
        let shared = playlist(json!({
            "id": "1", "name": "A", "owner": "alice", "readonly": true,
        }));
        assert!(!shared.is_editable_by("alice"));
        let open = playlist(json!({
            "id": "1", "name": "A", "owner": "alice", "readonly": false,
        }));
        assert!(open.is_editable_by("bob"));
    }

    #[test]
    fn without_a_readonly_flag_the_owner_can_edit() {
        let owned = playlist(json!({ "id": "1", "name": "A", "owner": "Alice" }));
        assert!(owned.is_editable_by("alice"));
        assert!(!owned.is_editable_by("bob"));
        let unowned = playlist(json!({ "id": "1", "name": "A" }));
        assert!(unowned.is_editable_by("bob"));
    }

    #[test]
    fn an_empty_comment_is_no_comment() {
        let blank = playlist(json!({ "id": "1", "name": "A", "comment": "" }));
        assert_eq!(blank.comment(), None);
    }
}
